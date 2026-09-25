//! `lo export` / `lo import`: share an exact configuration
//! as a shortcode.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::IsTerminal;

use anyhow::{Context, Result, bail};
use clap::Args;
use loadout_code::{Payload, SourcePin};
use loadout_git::Git;
use loadout_model::config::normalize_url;
use loadout_model::{Membership, SourceSub};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options};
use crate::exit;
use crate::membership;
use crate::sources::{load_lock, repo_dir};
use crate::state::Resolved;

/// The fingerprint of the last applied configuration.
pub fn fingerprint(resolved: &Resolved) -> String {
    let ids: Vec<String> = resolved.items.iter().map(|i| i.id.to_string()).collect();
    loadout_code::fingerprint(
        resolved
            .items
            .iter()
            .zip(&ids)
            .map(|(i, id)| (id.as_str(), i.content_hash.as_str(), i.enabled)),
        resolved.placements.keys().map(String::as_str),
    )
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Also print the code as a QR code.
    #[arg(long)]
    pub qr: bool,
    /// Leave out commit pins: the importer gets the latest of each source.
    #[arg(long)]
    pub latest: bool,
}

/// `--json` output of `lo export`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportReport {
    /// The shortcode (`lo1_…`).
    pub code: String,
    /// Fingerprint of the configuration it describes.
    pub fingerprint: String,
    /// False with `--latest`.
    pub pinned: bool,
    #[serde(skip)]
    qr: Option<String>,
}

impl Report for ExportReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if let Some(qr) = &self.qr {
            writeln!(out, "{qr}")?;
        }
        writeln!(out, "{}", self.code)?;
        writeln!(
            out,
            "fingerprint {}{}",
            self.fingerprint,
            if self.pinned {
                ""
            } else {
                " (not pinned: the importer gets the latest commits)"
            }
        )
    }
}

pub fn export(ctx: &Ctx, args: ExportArgs) -> Result<u8> {
    let config = ctx.load_config()?;
    let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
        bail!("nothing synced yet; run `lo sync` first");
    };
    let lock = load_lock(&ctx.paths)?;
    let pin = |url: &str| {
        (!args.latest)
            .then(|| lock.source(url).map(|s| s.commit.clone()))
            .flatten()
    };
    let manual: BTreeSet<String> = config
        .sources
        .iter()
        .map(|s| normalize_url(&s.url))
        .collect();
    let payload = Payload {
        v: loadout_code::PAYLOAD_VERSION,
        company_config: config.company_config.clone(),
        profile: config
            .profile
            .iter()
            .map(|(k, v)| (k.clone(), v.0.clone()))
            .collect(),
        sources: config
            .sources
            .iter()
            .map(|s| SourcePin {
                url: s.url.clone(),
                commit: pin(&s.url),
                layer: s.layer.clone(),
                group: s.group.clone(),
                priority: s.priority,
            })
            .collect(),
        pins: resolved
            .sources
            .iter()
            .filter(|s| !manual.contains(&normalize_url(&s.url)))
            .filter_map(|s| pin(&s.url).map(|c| (s.url.clone(), c)))
            .collect(),
        toggles: config.toggles.clone(),
        prefer: config.prefer.clone(),
        targets: config
            .targets
            .enabled
            .clone()
            .unwrap_or_else(|| resolved.placements.keys().cloned().collect()),
    };
    let code = loadout_code::encode(&payload);
    let qr = if args.qr {
        Some(render_qr(&code)?)
    } else {
        None
    };
    ctx.emit(&ExportReport {
        fingerprint: fingerprint(&resolved),
        pinned: !args.latest,
        code,
        qr,
    })?;
    Ok(exit::OK)
}

fn render_qr(code: &str) -> Result<String> {
    let qr = qrcode::QrCode::with_error_correction_level(code.as_bytes(), qrcode::EcLevel::L)
        .context("the shortcode is too long for a QR code")?;
    Ok(qr
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build())
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The shortcode (`lo1_…`).
    pub code: String,
    /// Ignore the commit pins and install the latest of each source.
    #[arg(long)]
    pub latest: bool,
    /// Apply without asking for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// What an import will change (shown before confirming).
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ImportSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_config: Option<String>,
    /// `layer:group` memberships.
    pub groups: Vec<String>,
    /// Manual sources to subscribe to.
    pub sources: Vec<String>,
    /// Manual sources removed because the code doesn't list them.
    pub unsubscribed: Vec<String>,
    pub toggles: BTreeMap<String, bool>,
    pub prefer: BTreeMap<String, String>,
    pub targets: Vec<String>,
    /// Pinned commits (URL → commit); empty with `--latest`.
    pub pins: BTreeMap<String, String>,
    /// Pinned commits this machine doesn't have yet (fetched now; needs
    /// network).
    pub missing_commits: Vec<String>,
}

/// `--json` output of `lo import`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportReport {
    pub summary: ImportSummary,
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<ApplyReport>,
    /// Fingerprint after the import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub warnings: Vec<String>,
}

impl ImportSummary {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if let Some(c) = &self.company_config {
            writeln!(out, "Company config:  {c}")?;
        }
        writeln!(out, "Groups:    {}", list(&self.groups))?;
        writeln!(out, "Sources:  {}", list(&self.sources))?;
        if !self.unsubscribed.is_empty() {
            writeln!(out, "Remove:   {}", self.unsubscribed.join(", "))?;
        }
        if !self.toggles.is_empty() {
            let t: Vec<String> = self
                .toggles
                .iter()
                .map(|(k, v)| format!("{k}={}", if *v { "on" } else { "off" }))
                .collect();
            writeln!(out, "Toggles:  {}", t.join(", "))?;
        }
        if !self.prefer.is_empty() {
            let p: Vec<String> = self
                .prefer
                .iter()
                .map(|(k, v)| format!("{k}→{v}"))
                .collect();
            writeln!(out, "Prefer:   {}", p.join(", "))?;
        }
        writeln!(out, "Targets:  {}", list(&self.targets))?;
        if self.pins.is_empty() {
            writeln!(out, "Pins:     none (latest commits)")?;
        } else {
            writeln!(
                out,
                "Pins:     {} source(s) at exact commits",
                self.pins.len()
            )?;
        }
        if !self.missing_commits.is_empty() {
            writeln!(
                out,
                "Fetch:    {} commit(s) not on this machine yet (needs network): {}",
                self.missing_commits.len(),
                self.missing_commits.join(", ")
            )?;
        }
        Ok(())
    }
}

fn list(v: &[String]) -> String {
    if v.is_empty() {
        "none".into()
    } else {
        v.join(", ")
    }
}

impl Report for ImportReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        if !self.applied {
            self.summary.human(out)?;
            return writeln!(out, "Not applied.");
        }
        if let Some(s) = &self.sync {
            s.human(out)?;
        }
        if let Some(fp) = &self.fingerprint {
            writeln!(out, "fingerprint {fp}")?;
        }
        Ok(())
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub fn import(ctx: &Ctx, args: ImportArgs) -> Result<u8> {
    let payload = loadout_code::decode(&args.code)?;
    let mut doc = ctx.load_config_doc()?;
    let config = doc.config();
    let git = Git::new();

    let mut pins: BTreeMap<String, String> = BTreeMap::new();
    if !args.latest {
        pins.extend(payload.pins.clone());
        for s in &payload.sources {
            if let Some(c) = &s.commit {
                pins.insert(s.url.clone(), c.clone());
            }
        }
    }
    let code_sources: BTreeSet<String> = payload
        .sources
        .iter()
        .map(|s| normalize_url(&s.url))
        .collect();
    let summary = ImportSummary {
        company_config: payload.company_config.clone(),
        groups: payload
            .profile
            .iter()
            .flat_map(|(layer, groups)| groups.iter().map(move |u| format!("{layer}:{u}")))
            .collect(),
        sources: payload.sources.iter().map(|s| s.url.clone()).collect(),
        unsubscribed: config
            .sources
            .iter()
            .filter(|s| !code_sources.contains(&normalize_url(&s.url)))
            .map(|s| s.url.clone())
            .collect(),
        toggles: payload.toggles.clone(),
        prefer: payload.prefer.clone(),
        targets: payload.targets.clone(),
        missing_commits: pins
            .iter()
            .filter(|(url, c)| !git.has_commit(&repo_dir(&ctx.paths, url), c))
            .map(|(url, c)| format!("{url}@{}", &c[..c.len().min(8)]))
            .collect(),
        pins: pins.clone(),
    };

    // Show a summary and require confirmation.
    if !args.yes {
        let interactive =
            !ctx.json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        if !interactive {
            bail!(
                "importing replaces your profile, sources, toggles and targets; re-run with --yes to confirm"
            );
        }
        let mut text = String::new();
        summary.human(&mut text)?;
        eprint!("{text}");
        let ok = dialoguer::Confirm::new()
            .with_prompt("Apply this configuration?")
            .default(false)
            .interact()
            .context("reading confirmation")?;
        if !ok {
            ctx.emit(&ImportReport {
                summary,
                applied: false,
                sync: None,
                fingerprint: None,
                warnings: Vec::new(),
            })?;
            return Ok(exit::OK);
        }
    }

    // Write the configuration.
    if let Some(c) = &payload.company_config {
        doc.set_company_config(c);
    }
    doc.set_profile(&payload.profile);
    let mut membership = Membership::default();
    if let Some(url) = &payload.company_config
        && let Some(pinned) = pins.get(url)
    {
        crate::sources::pin_checkout(&git, &ctx.paths, url, pinned)?;
    }
    if let Some(url) = &payload.company_config
        && let Ok(c) = membership::load_company_config(
            ctx,
            &git,
            url,
            !repo_dir(&ctx.paths, url).join(".git").exists(),
        )
    {
        // Keep the imported groups across membership refreshes.
        for u in c
            .company_config
            .groups
            .iter()
            .filter(|u| u.layer != "company")
        {
            let member = payload
                .profile
                .get(&u.layer)
                .is_some_and(|groups| groups.contains(&u.name));
            if member {
                membership.joined.push(u.id());
            } else {
                membership.left.push(u.id());
            }
        }
    }
    doc.set_membership(&membership);
    for s in &config.sources {
        doc.remove_source(&s.url);
    }
    for s in &payload.sources {
        doc.add_source(&SourceSub {
            url: s.url.clone(),
            layer: s.layer.clone(),
            group: s.group.clone(),
            priority: s.priority,
            git_ref: None,
        });
    }
    doc.remove_table("toggles");
    for (id, on) in &payload.toggles {
        doc.set_toggle(id, *on);
    }
    doc.remove_table("prefer");
    for (key, src) in &payload.prefer {
        doc.set_prefer(key, src);
    }
    doc.set_targets_enabled(&payload.targets);
    ctx.save_config(&doc)?;

    let config = doc.config();
    // A code exported with --latest carries no pins: fetch, like --latest.
    let opts = if args.latest || pins.is_empty() {
        Options::new(Fetch::Remote)
    } else {
        Options {
            approved: pins
                .iter()
                .map(|(u, c)| (normalize_url(u), c.clone()))
                .collect(),
            ..Options::new(Fetch::Cached)
        }
    };
    let sync = engine::apply(ctx, &config, opts).with_context(|| {
        if summary.missing_commits.is_empty() {
            "applying the imported configuration".to_owned()
        } else {
            format!(
                "applying the imported configuration (these commits had to be fetched: {})",
                summary.missing_commits.join(", ")
            )
        }
    })?;
    let fp = Resolved::load(&ctx.paths.resolved_file())?.map(|r| fingerprint(&r));
    let code = exit_code(&sync);
    ctx.emit(&ImportReport {
        summary,
        applied: true,
        warnings: sync.warnings.clone(),
        sync: Some(sync),
        fingerprint: fp,
    })?;
    Ok(code)
}
