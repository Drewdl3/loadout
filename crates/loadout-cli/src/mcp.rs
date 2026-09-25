//! Installing MCP servers into target configs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use loadout_core::mcp::{self, Delivered, Piece, Plan, Value};
use loadout_core::resolve::Resolution;
use loadout_model::{ItemId, ItemKind};
use loadout_secrets::{ExposeSecret, SecretResolver};
use loadout_targets::mcp::{Format, apply};
use loadout_targets::owned::{OwnedFile, OwnedMcp};
use loadout_targets::{TargetDef, expand_path};

use crate::engine::{PathReport, TargetReport, display_path};
use crate::scan::ScannedItem;
use crate::secrets::System;

/// Inputs shared by every target.
#[derive(Clone)]
pub struct McpEnv<'a> {
    pub home: &'a Path,
    pub items: &'a BTreeMap<&'a ItemId, &'a ScannedItem>,
    /// Source name → checkout directory (for `sops://`).
    pub source_dirs: &'a BTreeMap<String, PathBuf>,
    pub loadout_bin: &'a str,
    /// Global arguments before `mcp-run` (project mode: `--project-dir`).
    pub loadout_args: &'a [String],
    /// Project mode: install into the repo's project paths.
    pub project_root: Option<&'a Path>,
    pub allow_secrets_on_disk: bool,
    /// Built lazily: only needed to write secrets to disk.
    pub system: &'a dyn Fn() -> anyhow::Result<&'a System>,
    /// Plan only: read configs, write nothing, resolve no secrets.
    pub dry_run: bool,
}

/// [`apply`], or with `dry_run` only the merge (nothing written).
#[allow(clippy::too_many_arguments)]
fn apply_or_plan(
    format: Format,
    path: &Path,
    display: &str,
    key: &str,
    desired: &BTreeMap<String, serde_json::Value>,
    owned: &BTreeSet<String>,
    private: bool,
    dry_run: bool,
) -> Result<loadout_targets::mcp::Merged, loadout_targets::mcp::McpError> {
    if !dry_run {
        return apply(format, path, display, key, desired, owned, private);
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    loadout_targets::mcp::merge(format, display, &text, key, desired, owned)
}

/// Where and how a target keeps MCP servers.
struct Spec {
    renderer: String,
    format: Format,
    path: PathBuf,
    key: String,
}

fn spec(target: &TargetDef, home: &Path, project: Option<&Path>) -> Result<Option<Spec>, String> {
    let Some(r) = &target.mcp else {
        return Ok(None);
    };
    let format = Format::from_renderer(&r.renderer).map_err(|e| e.to_string())?;
    let path = match project {
        Some(root) => {
            let Some(rel) = r.project.as_deref() else {
                return Ok(None);
            };
            crate::engine::project_path(root, rel).ok_or_else(|| {
                format!(
                    "[mcp] project path {rel:?} of target {} must be relative",
                    target.id
                )
            })?
        }
        None => {
            let path = r
                .path
                .as_deref()
                .ok_or_else(|| format!("[mcp] of target {} has no path", target.id))?;
            expand_path(path, home).ok_or_else(|| {
                format!(
                    "[mcp] path {path:?} of target {} must start with ~/ or be absolute",
                    target.id
                )
            })?
        }
    };
    Ok(Some(Spec {
        renderer: r.renderer.clone(),
        format,
        path,
        key: r
            .key
            .clone()
            .unwrap_or_else(|| format.default_key().to_owned()),
    }))
}

/// Brings the target's MCP config in line with its enabled MCP winners.
pub fn reconcile_target(
    env: &McpEnv<'_>,
    target: &TargetDef,
    resolution: &Resolution,
    owned: &mut OwnedFile,
    report: &mut TargetReport,
    warnings: &mut Vec<String>,
) {
    let spec = match spec(target, env.home, env.project_root) {
        Ok(Some(s)) => s,
        Ok(None) => {
            remove_all(env.home, &target.id, owned, report, warnings, env.dry_run);
            return;
        }
        Err(e) => {
            warnings.push(e);
            return;
        }
    };
    let prev = owned.mcp.get(&target.id).cloned().unwrap_or_default();
    if !prev.servers.is_empty()
        && (prev.path != spec.path || prev.key != spec.key || prev.renderer != spec.renderer)
    {
        // The target's config location changed: clean up the old one first.
        remove_all(env.home, &target.id, owned, report, warnings, env.dry_run);
    }
    let prev = owned.mcp.get(&target.id).cloned().unwrap_or_default();

    let mut desired = BTreeMap::new();
    let mut on_disk = BTreeSet::new();
    for r in resolution.items.iter().filter(|r| r.enabled) {
        let Some(id) = &r.winner else { continue };
        if id.key().kind() != ItemKind::Mcp {
            continue;
        }
        let item = env.items[id];
        let Some(server) = &item.mcp else { continue };
        let name = id.key().name().to_owned();
        let plan = mcp::plan(&mcp::Input {
            id,
            server,
            caps: spec.format.caps(),
            loadout_bin: env.loadout_bin,
            loadout_args: env.loadout_args,
            allow_secrets_on_disk: env.allow_secrets_on_disk,
        });
        match plan {
            Plan::Skip(reason) => {
                warnings.push(format!("{}: skipped {}: {reason}", target.id, id.key()))
            }
            Plan::Install {
                server,
                secrets_on_disk,
            } => {
                let server = if secrets_on_disk && !env.dry_run {
                    match materialize(env, id, server) {
                        Ok(s) => {
                            on_disk.insert(name.clone());
                            s
                        }
                        Err(e) => {
                            warnings.push(format!("{}: skipped {}: {e}", target.id, id.key()));
                            continue;
                        }
                    }
                } else {
                    server
                };
                desired.insert(name, spec.format.render(&server));
            }
        }
    }

    let display = display_path(&spec.path, env.home);
    match apply_or_plan(
        spec.format,
        &spec.path,
        &display,
        &spec.key,
        &desired,
        &prev.servers,
        !on_disk.is_empty(),
        env.dry_run,
    ) {
        Ok(m) => {
            let key = |n: &String| format!("mcp/{n}");
            report.added.extend(m.added.iter().map(key));
            report.updated.extend(m.updated.iter().map(key));
            report.removed.extend(m.removed.iter().map(key));
            report.unchanged += m.unchanged;
            for c in &m.collisions {
                warnings.push(format!(
                    "{}: {display} already has an MCP server named {c:?} that Loadout doesn't manage; skipped mcp/{c}",
                    target.id
                ));
                report.collisions.push(PathReport {
                    item: key(c),
                    path: display.clone(),
                    message: "exists and is not managed by Loadout".into(),
                });
            }
            let secrets_on_disk = on_disk.intersection(&m.owned).cloned().collect();
            owned.set_mcp(
                &target.id,
                OwnedMcp {
                    renderer: spec.renderer.clone(),
                    path: spec.path.clone(),
                    key: spec.key.clone(),
                    servers: m.owned,
                    secrets_on_disk,
                },
            );
        }
        Err(e) => report.errors.push(PathReport {
            item: "mcp".into(),
            path: display,
            message: e.to_string(),
        }),
    }
}

/// Removes every server Loadout manages for `target_id` (target
/// disabled, or its MCP config moved).
pub fn remove_all(
    home: &Path,
    target_id: &str,
    owned: &mut OwnedFile,
    report: &mut TargetReport,
    warnings: &mut Vec<String>,
    dry_run: bool,
) {
    let Some(prev) = owned.mcp.get(target_id).cloned() else {
        return;
    };
    let display = display_path(&prev.path, home);
    let format = match Format::from_renderer(&prev.renderer) {
        Ok(f) => f,
        Err(e) => {
            warnings.push(format!("{target_id}: {e}; left {display} unchanged"));
            return;
        }
    };
    match apply_or_plan(
        format,
        &prev.path,
        &display,
        &prev.key,
        &BTreeMap::new(),
        &prev.servers,
        false,
        dry_run,
    ) {
        Ok(m) => {
            report
                .removed
                .extend(m.removed.iter().map(|n| format!("mcp/{n}")));
            owned.mcp.remove(target_id);
        }
        Err(e) => warnings.push(format!(
            "{target_id}: could not remove MCP servers from {display}: {e}"
        )),
    }
}

/// Resolves the [`Piece::Secret`]s in `server` (allow_secrets_on_disk).
fn materialize(env: &McpEnv<'_>, id: &ItemId, server: Delivered) -> anyhow::Result<Delivered> {
    let system = (env.system)()?;
    let resolvers = system.resolvers(env.source_dirs.get(id.source()).cloned());
    let resolve = |v: Value| -> anyhow::Result<Value> {
        let mut out = String::new();
        let mut pieces = Vec::new();
        for p in v.0 {
            match p {
                Piece::Lit(l) => out.push_str(&l),
                Piece::Secret(r) => out.push_str(resolvers.resolve(&r)?.expose_secret()),
                Piece::Env(e) => {
                    pieces.push(Piece::Lit(std::mem::take(&mut out)));
                    pieces.push(Piece::Env(e));
                }
            }
        }
        pieces.push(Piece::Lit(out));
        pieces.retain(|p| !matches!(p, Piece::Lit(l) if l.is_empty()));
        Ok(Value(pieces))
    };
    Ok(match server {
        Delivered::Remote {
            transport,
            url,
            headers,
            headers_helper,
        } => Delivered::Remote {
            transport,
            url: resolve(url)?,
            headers: headers
                .into_iter()
                .map(|(k, v)| Ok((k, resolve(v)?)))
                .collect::<anyhow::Result<_>>()?,
            headers_helper,
        },
        other => other,
    })
}
