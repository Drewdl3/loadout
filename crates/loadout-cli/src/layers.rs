//! The layer model in effect (see `loadout_core::layers::merge`): during a
//! sync from the loaded sources, and afterwards from what the last sync
//! recorded in `resolved.json`.

use anyhow::Result;
use loadout_core::layers::{Inputs, merge};
use loadout_git::Git;
use loadout_model::{CompanyConfig, Config, Layer, LayerModel};

use crate::ctx::Ctx;
use crate::membership;
use crate::sources::Fetched;
use crate::state::Resolved;

/// Each loaded source's `layers:` (root and nested manifests), by name.
pub fn suggestions(fetched: &[Fetched]) -> Vec<(String, Vec<Layer>)> {
    let mut out: Vec<(String, Vec<Layer>)> = fetched
        .iter()
        .flat_map(|f| {
            std::iter::once(&f.scanned.manifest)
                .chain(f.scanned.nested.iter().map(|n| &n.manifest))
                .filter(|m| !m.layers.is_empty())
                .map(|m| (m.name.clone(), m.layers.clone()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The merged model and its warnings.
pub fn model(
    config: &Config,
    company: Option<&CompanyConfig>,
    suggestions: &[(String, Vec<Layer>)],
) -> (LayerModel, Vec<String>) {
    let (model, warnings) = merge(&Inputs {
        sources: suggestions,
        company: company.map(|c| c.layers.as_slice()),
        local: &config.layers,
        allow_local_ranks: company.is_none_or(|c| c.policy.allow_local_ranks),
    });
    (model, warnings.iter().map(ToString::to_string).collect())
}

/// The model for the current `config.toml`, the company config's local
/// checkout, and the sources of the last sync.
pub fn current(ctx: &Ctx) -> Result<(LayerModel, Vec<String>)> {
    let config = ctx.load_config()?;
    let company = match &config.company_config {
        // A project layer has its own sources only.
        Some(url) if ctx.paths.project.is_none() => {
            membership::load_company_config(ctx, &Git::new(), url, false)
                .ok()
                .map(|c| c.company_config)
        }
        _ => None,
    };
    let suggestions = Resolved::load(&ctx.paths.resolved_file())?
        .map(|r| r.layer_suggestions)
        .unwrap_or_default();
    Ok(model(&config, company.as_ref(), &suggestions))
}
