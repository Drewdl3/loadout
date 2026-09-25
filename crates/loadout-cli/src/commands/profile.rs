//! `lo profile [show|refresh|set]`, `lo join`, `lo leave` —
//! viewing and changing group membership.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use loadout_git::Git;
use loadout_members::{Basis, GroupStatus};
use loadout_model::{CompanyConfig, Config, Membership};
use schemars::JsonSchema;
use serde::Serialize;

use crate::commands::sync::exit_code;
use crate::ctx::{Ctx, Report};
use crate::engine::{self, ApplyReport, Fetch, Options};
use crate::exit;
use crate::membership::{self, LoadedCompanyConfig, MembershipState};

#[derive(Debug, Args)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: Option<ProfileCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// Show your groups (default).
    Show,
    /// Re-run membership discovery, then sync.
    Refresh,
    /// Override your groups for one layer, then sync.
    Set {
        layer: String,
        /// Groups to belong to in this layer (none = leave them all).
        groups: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct GroupArgs {
    /// `layer:group`, e.g. `product:billing`.
    pub group: String,
}

/// `--json` output of `lo profile show`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProfileReport {
    pub company_config: Option<String>,
    /// Layer → groups you belong to.
    pub profile: BTreeMap<String, Vec<String>>,
    /// Every company config group and how its membership was decided (after the
    /// last discovery).
    pub groups: Vec<GroupStatus>,
    /// Unix time of the last discovery.
    pub refreshed_at: Option<u64>,
}

/// `--json` output of `lo profile refresh|set`, `join`, `leave`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MembershipChangeReport {
    pub profile: ProfileReport,
    pub sync: ApplyReport,
}

impl Report for ProfileReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        match &self.company_config {
            Some(c) => writeln!(out, "Company config: {c}")?,
            None => writeln!(
                out,
                "No company config (run `lo init <company-config-url>`); profile from config.toml:"
            )?,
        }
        let width = self.profile.keys().map(String::len).max().unwrap_or(0);
        for (layer, groups) in &self.profile {
            let detail: Vec<String> = groups
                .iter()
                .map(|u| match self.status(layer, u) {
                    Some(s) => format!("{u} ({})", basis_text(s)),
                    None => u.clone(),
                })
                .collect();
            writeln!(out, "  {layer:<width$}  {}", detail.join(", "))?;
        }
        let others: Vec<&GroupStatus> = self.groups.iter().filter(|u| !u.member).collect();
        if !others.is_empty() {
            writeln!(out, "Not a member of:")?;
            for u in others {
                let hint = if u.joinable {
                    format!(" — `lo join {}:{}`", u.layer, u.group)
                } else {
                    String::new()
                };
                writeln!(out, "  {}:{} ({}){hint}", u.layer, u.group, basis_text(u))?;
            }
        }
        Ok(())
    }
}

impl ProfileReport {
    fn status(&self, layer: &str, group: &str) -> Option<&GroupStatus> {
        self.groups
            .iter()
            .find(|u| u.layer == layer && u.group == group)
    }
}

fn basis_text(u: &GroupStatus) -> String {
    match (u.basis, &u.error) {
        (Basis::Company, _) => "company".into(),
        (Basis::Rule, _) if u.member => "discovered".into(),
        (Basis::Rule, _) => "no match".into(),
        (Basis::Joined, _) => "joined".into(),
        (Basis::Left, _) => "left".into(),
        (Basis::Cached, Some(e)) => format!("cached; {e}"),
        (Basis::Cached, None) => "cached".into(),
    }
}

impl Report for MembershipChangeReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        self.profile.human(out)?;
        self.sync.human(out)
    }

    fn warnings(&self) -> &[String] {
        &self.sync.warnings
    }
}

pub fn profile(ctx: &Ctx, args: ProfileArgs) -> Result<u8> {
    match args.command.unwrap_or(ProfileCommand::Show) {
        ProfileCommand::Show => {
            let config = ctx.load_config()?;
            ctx.emit(&profile_report(ctx, &config))?;
            Ok(exit::OK)
        }
        ProfileCommand::Refresh => {
            let (config, loaded) = with_company_config(ctx, true)?;
            let d = membership::run_discovery(&loaded.company_config, &config);
            membership::save_discovery(ctx, &loaded.url, &d)?;
            let mut r = resync(ctx, loaded)?;
            r.sync.warnings.splice(0..0, d.warnings);
            finish(ctx, r)
        }
        ProfileCommand::Set { layer, groups } => {
            let (config, loaded) = with_company_config(ctx, false)?;
            let ch = &loaded.company_config;
            if layer == "company" {
                bail!("the company layer is set by the company config");
            }
            let in_layer: Vec<_> = ch.groups.iter().filter(|u| u.layer == layer).collect();
            if in_layer.is_empty() {
                bail!("the company config has no groups in layer {layer:?}");
            }
            for u in &groups {
                if !in_layer.iter().any(|d| &d.name == u) {
                    bail!("the company config has no group {layer}:{u}");
                }
            }
            let changes: Vec<(String, bool)> = in_layer
                .iter()
                .map(|d| (d.id(), groups.contains(&d.name)))
                .collect();
            change_membership(ctx, &config, loaded, &changes)
        }
    }
}

pub fn join(ctx: &Ctx, args: GroupArgs) -> Result<u8> {
    set_one(ctx, &args.group, true)
}

pub fn leave(ctx: &Ctx, args: GroupArgs) -> Result<u8> {
    set_one(ctx, &args.group, false)
}

fn set_one(ctx: &Ctx, group: &str, member: bool) -> Result<u8> {
    let Some((layer, name)) = group.split_once(':') else {
        bail!("expected layer:group, e.g. product:billing");
    };
    let (config, loaded) = with_company_config(ctx, false)?;
    if layer == "company" {
        bail!("everyone belongs to the company group");
    }
    let Some(def) = loaded.company_config.group(layer, name) else {
        bail!("the company config has no group {group}; see `lo profile`");
    };
    if member && !def.membership.allows_manual() {
        bail!(
            "{group} membership is decided by the company config ({}), not by opting in",
            rule_kind(&def.membership)
        );
    }
    let id = def.id();
    change_membership(ctx, &config, loaded, &[(id, member)])
}

fn rule_kind(rule: &loadout_model::Rule) -> &'static str {
    use loadout_model::Rule::*;
    match rule {
        Manual(_) => "manual",
        GithubTeam(_) => "github_team",
        GitlabGroup(_) => "gitlab_group",
        Env(_) => "env",
        Exec(_) => "exec",
        RepoAccess(_) => "repo_access",
        Any(_) => "any",
        All(_) => "all",
    }
}

/// Records join/leave choices, updates the cached profile without
/// re-querying providers, and syncs.
fn change_membership(
    ctx: &Ctx,
    config: &Config,
    loaded: LoadedCompanyConfig,
    changes: &[(String, bool)],
) -> Result<u8> {
    let mut m: Membership = config.membership.clone();
    for (id, member) in changes {
        m.joined.retain(|u| u != id);
        m.left.retain(|u| u != id);
        if *member {
            m.joined.push(id.clone());
        } else {
            m.left.push(id.clone());
        }
    }
    m.joined.sort();
    m.left.sort();
    let mut doc = ctx.load_config_doc()?;
    doc.set_membership(&m);
    ctx.save_config(&doc)?;

    let mut state = membership::load_state(ctx)
        .unwrap_or_else(|| initial_state(&loaded.company_config, &loaded.url, config));
    for (id, member) in changes {
        if let Some(u) = state
            .groups
            .iter_mut()
            .find(|u| &format!("{}:{}", u.layer, u.group) == id)
        {
            u.member = *member;
            u.basis = if *member { Basis::Joined } else { Basis::Left };
            u.error = None;
        }
    }
    let d = loadout_members::Discovery {
        profile: state
            .groups
            .iter()
            .filter(|u| u.member)
            .map(|u| (u.layer.as_str(), u.group.as_str()))
            .collect(),
        groups: state.groups.clone(),
        warnings: Vec::new(),
    };
    membership::save_discovery(ctx, &loaded.url, &d)?;
    let r = resync(ctx, loaded)?;
    finish(ctx, r)
}

/// Group statuses from the cached `[profile]` when no discovery ran yet.
fn initial_state(company_config: &CompanyConfig, url: &str, config: &Config) -> MembershipState {
    let cached = membership::cached_profile(config);
    let mut groups = vec![GroupStatus {
        layer: "company".into(),
        group: company_config.name.clone(),
        member: true,
        basis: Basis::Company,
        joinable: false,
        error: None,
    }];
    groups.extend(company_config.groups.iter().map(|u| GroupStatus {
        layer: u.layer.clone(),
        group: u.name.clone(),
        member: cached.contains(&u.layer, &u.name),
        basis: Basis::Cached,
        joinable: u.membership.allows_manual(),
        error: None,
    }));
    MembershipState {
        version: 1,
        company_config: url.to_owned(),
        refreshed_at: 0,
        groups,
    }
}

fn with_company_config(ctx: &Ctx, fetch: bool) -> Result<(Config, LoadedCompanyConfig)> {
    let config = ctx.load_config()?;
    let Some(url) = config.company_config.clone() else {
        bail!("no company config set up; run `lo init <company-config-url>` first");
    };
    let loaded = membership::load_company_config(ctx, &Git::new(), &url, fetch)?;
    Ok((config, loaded))
}

fn resync(ctx: &Ctx, loaded: LoadedCompanyConfig) -> Result<MembershipChangeReport> {
    let config = ctx.load_config()?;
    let sync = engine::apply(
        ctx,
        &config,
        Options {
            company_config: Some(loaded),
            ..Options::new(Fetch::Remote)
        },
    )?;
    Ok(MembershipChangeReport {
        profile: profile_report(ctx, &ctx.load_config()?),
        sync,
    })
}

fn finish(ctx: &Ctx, r: MembershipChangeReport) -> Result<u8> {
    ctx.emit(&r)?;
    Ok(exit_code(&r.sync))
}

pub fn profile_report(ctx: &Ctx, config: &Config) -> ProfileReport {
    let state = membership::load_state(ctx).filter(|s| {
        config.company_config.as_deref().is_some_and(|c| {
            loadout_model::config::normalize_url(c)
                == loadout_model::config::normalize_url(&s.company_config)
        })
    });
    ProfileReport {
        company_config: config.company_config.clone(),
        profile: config
            .profile
            .iter()
            .map(|(k, v)| (k.clone(), v.0.clone()))
            .collect(),
        groups: state.as_ref().map(|s| s.groups.clone()).unwrap_or_default(),
        refreshed_at: state.map(|s| s.refreshed_at).filter(|t| *t > 0),
    }
}
