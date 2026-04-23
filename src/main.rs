use anyhow::Result;
use clap::Parser;
use envo::cli::commands;
use envo::cli::{Cli, Commands};
use envo::telemetry;

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    // Extract command name for telemetry before moving cli
    let command_name = command_label(&cli.command);

    let result = run(cli);

    if let Err(err) = result {
        // Track the error via telemetry
        telemetry::track_error("cli", &command_name, "command_failed", &format!("{err:#}"), verbose);

        // Print clean error messages — no stack traces in release mode
        eprintln!("✗ {err:#}");
        std::process::exit(1);
    }
}

/// Get a telemetry-safe label for the command (no arguments, no sensitive data).
fn command_label(cmd: &Commands) -> String {
    match cmd {
        Commands::Init { .. } => "init",
        Commands::Install { .. } => "install",
        Commands::Uninstall { .. } => "uninstall",
        Commands::Activate { .. } => "activate",
        Commands::Deactivate { .. } => "deactivate",
        Commands::Search { .. } => "search",
        Commands::Run { .. } => "run",
        Commands::Update => "update",
        Commands::Export { .. } => "export",
        Commands::SelfUpdate { .. } => "self-update",
        Commands::Version { .. } => "version",
    }
    .to_string()
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { template } => {
            commands::cmd_init(template.as_deref(), cli.verbose)
        }

        Commands::Install { packages } => {
            commands::cmd_install(&packages, cli.verbose)
        }

        Commands::Uninstall { package } => {
            commands::cmd_uninstall(&package, cli.verbose)
        }

        Commands::Activate { inline, shell } => {
            commands::cmd_activate(inline, shell.as_deref(), cli.verbose)
        }

        Commands::Deactivate { inline, shell } => {
            commands::cmd_deactivate(inline, shell.as_deref(), cli.verbose)
        }

        Commands::Search { query, json } => {
            commands::cmd_search(&query, json, cli.verbose)
        }

        Commands::Run { command } => {
            commands::cmd_run(&command, cli.verbose)
        }

        Commands::Update => commands::cmd_update(cli.verbose),

        Commands::Export { format, output } => {
            commands::cmd_export(&format, output.as_deref(), cli.verbose)
        }

        Commands::SelfUpdate { check } => {
            commands::cmd_self_update(check, cli.verbose)
        }

        Commands::Version { json } => commands::cmd_version(json, cli.verbose),
    }
}
