//! The `--json` schemas in `schemas/cli/` must match the output types.
//! Regenerate with `UPDATE_SCHEMAS=1 cargo test -p loadout-cli --test schemas`.

use std::path::PathBuf;

use loadout_cli::commands::{
    adopt, audit, doctor, export_source, info, init, list, new_source, profile, project, review,
    schedule, search, secrets, share, subscribe, targets, template, why,
};
use loadout_cli::ctx::ErrorReport;
use loadout_cli::engine::ApplyReport;
use schemars::{JsonSchema, schema_for};

fn check<T: JsonSchema>(name: &str, stale: &mut Vec<String>) {
    check_in::<T>("cli/", name, stale);
}

fn check_in<T: JsonSchema>(sub: &str, name: &str, stale: &mut Vec<String>) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../schemas/{sub}"));
    let path = dir.join(format!("{name}.schema.json"));
    let mut schema = schema_for!(T);
    schema.insert(
        "$id".into(),
        format!("https://github.com/Drewdl3/loadout/schemas/{sub}{name}.schema.json").into(),
    );
    let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let current = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if current != text {
        stale.push(path.display().to_string());
    }
}

#[test]
fn cli_schemas_are_up_to_date() {
    let mut stale = Vec::new();
    check::<subscribe::SubscriptionReport>("subscribe", &mut stale);
    check::<subscribe::SubscriptionReport>("unsubscribe", &mut stale);
    for apply in ["sync", "enable", "disable", "prefer"] {
        check::<ApplyReport>(apply, &mut stale);
    }
    check::<why::WhyReport>("why", &mut stale);
    check::<init::InitReport>("init", &mut stale);
    check::<init::InitCreateReport>("init-new-source", &mut stale);
    check::<subscribe::SubscribePreview>("subscribe-preview", &mut stale);
    check::<template::TemplateListReport>("template-list", &mut stale);
    check::<template::TemplateShowReport>("template-show", &mut stale);
    check::<template::TemplateUseReport>("template-use", &mut stale);
    check::<adopt::AdoptReport>("adopt", &mut stale);
    check::<project::ProjectInitReport>("init-project", &mut stale);
    check::<profile::ProfileReport>("profile", &mut stale);
    for change in ["profile-refresh", "profile-set", "join", "leave"] {
        check::<profile::MembershipChangeReport>(change, &mut stale);
    }
    check::<list::ListReport>("list", &mut stale);
    check::<doctor::DoctorReport>("doctor", &mut stale);
    check::<secrets::SecretsCheckReport>("secrets-check", &mut stale);
    for change in ["secrets-set", "secrets-clear"] {
        check::<secrets::SecretChangeReport>(change, &mut stale);
    }
    check::<audit::AuditReport>("audit", &mut stale);
    check::<review::StatusReport>("status", &mut stale);
    check::<review::DiffReport>("diff", &mut stale);
    check::<ApplyReport>("approve", &mut stale);
    check::<schedule::ScheduleReport>("schedule", &mut stale);
    check::<share::ExportReport>("export", &mut stale);
    check::<new_source::NewSourceReport>("new-source", &mut stale);
    check::<info::InfoReport>("info", &mut stale);
    check::<loadout_cli::ui::UiStarted>("ui", &mut stale);
    check::<loadout_cli::commands::self_update::SelfUpdateReport>("self-update", &mut stale);
    check::<export_source::ExportSourceReport>("export-source", &mut stale);
    check::<search::SearchReport>("search", &mut stale);
    check::<targets::TargetsReport>("targets", &mut stale);
    check::<share::ImportReport>("import", &mut stale);
    check::<ErrorReport>("error", &mut stale);
    check_in::<loadout_model::Lock>("", "lock", &mut stale);
    check_in::<loadout_model::Manifest>("", "manifest", &mut stale);
    check_in::<loadout_model::LoadoutBlock>("", "item-block", &mut stale);
    check_in::<loadout_model::template::TemplateFrontmatter>("", "template", &mut stale);
    check_in::<loadout_model::Config>("", "config", &mut stale);
    assert!(
        stale.is_empty(),
        "stale schemas (run with UPDATE_SCHEMAS=1): {stale:#?}"
    );
}
