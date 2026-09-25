//! Implementation of the `lo` command-line interface.

pub mod audit;
pub mod commands;
pub mod ctx;
pub mod edit;
pub mod engine;
pub mod mcp;
pub mod membership;
pub mod paths;
pub mod plugins;
pub mod review;
pub mod scan;
pub mod schedule;
pub mod search;
pub mod secrets;
pub mod signing;
pub mod sources;
pub mod state;
pub mod store;
pub mod tui;
pub mod ui;
pub mod work;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::ctx::Ctx;

/// Process exit codes.
pub mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const PENDING: u8 = 2;
    pub const AUDIT_BLOCKED: u8 = 3;
    pub const CONFLICTS: u8 = 4;
}

/// Distribute AI-agent configuration (skills, MCP servers, subagents,
/// plugins, rules) from federated Git repos.
#[derive(Debug, Parser)]
#[command(name = "lo", version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress non-essential output.
    #[arg(long, short, global = true)]
    quiet: bool,

    /// Always exit 0; the outcome is in the output (for agent hooks and
    /// shims, which treat some exit codes specially).
    #[arg(long, global = true)]
    exit_zero: bool,

    /// Work on the project layer of the repo you're in (`.loadout/`)
    /// instead of your user configuration.
    #[arg(long, global = true)]
    project: bool,

    /// The project layer of the repo at this path (used in generated MCP
    /// configs).
    #[arg(long, global = true, hide = true, value_name = "PATH")]
    project_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// First-time setup from your company config repo.
    Init(commands::init::InitArgs),
    /// View or change your group membership.
    Profile(commands::profile::ProfileArgs),
    /// Join a group (`layer:group`) whose membership is opt-in.
    Join(commands::profile::GroupArgs),
    /// Leave a group (`layer:group`).
    Leave(commands::profile::GroupArgs),
    /// Subscribe to a source repo.
    Subscribe(commands::subscribe::SubscribeArgs),
    /// Remove a manual source subscription.
    Unsubscribe(commands::subscribe::UnsubscribeArgs),
    /// Fetch sources and install their items into your AI tools.
    Sync(commands::sync::SyncArgs),
    /// Last sync, pending changes, audit and conflict summary.
    Status(commands::review::StatusArgs),
    /// Show pending changes with content diffs.
    Diff(commands::review::DiffArgs),
    /// Apply pending changes.
    Approve(commands::review::ApproveArgs),
    /// List resolved items.
    List(commands::list::ListArgs),
    /// Explain how an item was resolved: candidates, winner, rule, targets.
    Why(commands::why::WhyArgs),
    /// Enable an item (refused if it isn't available to you).
    Enable(commands::toggle::ToggleArgs),
    /// Disable an item (refused for required or locked items).
    Disable(commands::toggle::ToggleArgs),
    /// Resolve an equal-rank conflict in favour of one source.
    Prefer(commands::toggle::PreferArgs),
    /// Audit your sources' items for prompt injection, dangerous commands and committed secrets.
    Audit(commands::audit::AuditArgs),
    /// Set up periodic `lo sync` with the OS scheduler.
    Schedule(commands::schedule::ScheduleArgs),
    /// Templates from higher sources: list them, or turn one into a skill in your source.
    Template(commands::template::TemplateArgs),
    /// Copy skills you already have (in your AI tools, or a folder) into a source to share them.
    Adopt(commands::adopt::AdoptArgs),
    /// Scaffold a new source repo (LOADOUT.md, an example skill, a CI audit).
    #[command(name = "new-source")]
    NewSource(commands::new_source::NewSourceArgs),
    /// Update lo to the latest release (verified against its SHA256SUMS).
    #[command(name = "self-update")]
    SelfUpdate(commands::self_update::SelfUpdateArgs),
    /// Local web dashboard to view and configure your setup (127.0.0.1 only).
    Ui(ui::UiArgs),
    /// Terminal UI: browse and filter items, sources, templates, groups and reviews.
    Tui(tui::TuiArgs),
    /// Commit edits in a source's working clone to a `loadout/…` branch and open a pull request.
    #[command(name = "export-source")]
    ExportSource(commands::export_source::ExportSourceArgs),
    /// Print a shortcode for your configuration (share it with `lo import`).
    Export(commands::share::ExportArgs),
    /// Apply a configuration from a shortcode.
    Import(commands::share::ImportArgs),
    /// Search items by name, description, tags and text.
    Search(commands::search::SearchArgs),
    /// Show a source's LOADOUT.md or an item's documentation.
    Info(commands::info::InfoArgs),
    /// List, enable or disable targets (AI tools), or set how items are placed.
    Targets(commands::targets::TargetsArgs),
    /// Check secret references; store or remove `secret://` values in the keychain.
    Secrets(commands::secrets::SecretsArgs),
    /// Diagnose problems with the local setup.
    Doctor(commands::doctor::DoctorArgs),
    /// Launch an MCP server with its secrets resolved (used by target configs).
    #[command(name = "mcp-run", hide = true)]
    McpRun(commands::mcp::McpArgs),
    /// Print an MCP server's secret headers as JSON (used by target configs).
    #[command(name = "mcp-headers", hide = true)]
    McpHeaders(commands::mcp::McpArgs),
}

/// Parses arguments and runs the CLI.
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.quiet);
    let exit_zero = cli.exit_zero;
    let is_init = matches!(cli.command, Command::Init(_));
    let ctx = match Ctx::from_env(cli.json, cli.quiet)
        .and_then(|ctx| ctx::select_layer(ctx, cli.project, cli.project_dir.as_deref(), is_init))
    {
        Ok(ctx) => ctx,
        Err(e) => {
            let code = ctx::report_error(cli.json, &e);
            return if exit_zero { ExitCode::SUCCESS } else { code };
        }
    };
    let result = match cli.command {
        Command::Init(args) => commands::init::run(&ctx, args),
        Command::Profile(args) => commands::profile::profile(&ctx, args),
        Command::Join(args) => commands::profile::join(&ctx, args),
        Command::Leave(args) => commands::profile::leave(&ctx, args),
        Command::Subscribe(args) => commands::subscribe::subscribe(&ctx, args),
        Command::Unsubscribe(args) => commands::subscribe::unsubscribe(&ctx, args),
        Command::Sync(args) => commands::sync::run(&ctx, args),
        Command::Status(args) => commands::review::status(&ctx, args),
        Command::Diff(args) => commands::review::diff(&ctx, args),
        Command::Approve(args) => commands::review::approve(&ctx, args),
        Command::List(args) => commands::list::run(&ctx, args),
        Command::Why(args) => commands::why::run(&ctx, args),
        Command::Enable(args) => commands::toggle::enable(&ctx, args),
        Command::Disable(args) => commands::toggle::disable(&ctx, args),
        Command::Prefer(args) => commands::toggle::prefer(&ctx, args),
        Command::Audit(args) => commands::audit::run(&ctx, args),
        Command::Schedule(args) => commands::schedule::run(&ctx, args),
        Command::Template(args) => commands::template::run(&ctx, args),
        Command::Adopt(args) => commands::adopt::run(&ctx, args),
        Command::NewSource(args) => commands::new_source::run(&ctx, args),
        Command::SelfUpdate(args) => commands::self_update::run(&ctx, args),
        Command::Ui(args) => ui::run(&ctx, args),
        Command::Tui(args) => tui::run(&ctx, args),
        Command::ExportSource(args) => commands::export_source::run(&ctx, args),
        Command::Export(args) => commands::share::export(&ctx, args),
        Command::Import(args) => commands::share::import(&ctx, args),
        Command::Search(args) => commands::search::run(&ctx, args),
        Command::Info(args) => commands::info::run(&ctx, args),
        Command::Targets(args) => commands::targets::run(&ctx, args),
        Command::Secrets(args) => commands::secrets::run(&ctx, args),
        Command::Doctor(args) => commands::doctor::run(&ctx, args),
        Command::McpRun(args) => commands::mcp::run(&ctx, args),
        Command::McpHeaders(args) => commands::mcp::headers(&ctx, args),
    };
    let code = match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => ctx::report_error(cli.json, &e),
    };
    if exit_zero { ExitCode::SUCCESS } else { code }
}

fn init_tracing(quiet: bool) {
    let default = if quiet { "error" } else { "warn" };
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOADOUT_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
