//! CLI command routing for envo.
//!
//! Defines the clap command structure and dispatches to the
//! command implementations in `commands.rs`. This module is a
//! thin routing layer — all business logic lives in the library modules.

pub mod commands;

use clap::{Parser, Subcommand};

/// envo — Nix-based developer environments with lazy fetch and instant activation.
#[derive(Parser, Debug)]
#[command(name = "envo", version, about, long_about = None)]
pub struct Cli {
    /// Enable verbose output for debugging.
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available commands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a new envo environment in the current directory.
    Init {
        /// Use a pre-built template. Use "list" to see available templates.
        #[arg(long)]
        template: Option<String>,
    },

    /// Install one or more packages into the environment.
    Install {
        /// Package names to install. If none given, resolves packages already in manifest.
        packages: Vec<String>,
    },

    /// Remove a package from the environment.
    Uninstall {
        /// Package name to remove.
        #[arg(required = true)]
        package: String,
    },

    /// Activate the environment (print a sourceable script).
    Activate {
        /// Print the snapshot contents to stdout instead of the file path.
        #[arg(long)]
        inline: bool,

        /// Shell type (bash, zsh, fish). Auto-detected from $SHELL if omitted.
        #[arg(long)]
        shell: Option<String>,
    },

    /// Deactivate the environment (print an unset script).
    Deactivate {
        /// Print the deactivation script to stdout.
        #[arg(long)]
        inline: bool,

        /// Shell type (bash, zsh, fish). Auto-detected from $SHELL if omitted.
        #[arg(long)]
        shell: Option<String>,
    },

    /// Search for packages in nixpkgs.
    Search {
        /// Search query.
        #[arg(required = true)]
        query: String,

        /// Output results as JSON (for programmatic consumers).
        #[arg(long)]
        json: bool,
    },

    /// Run a command inside the activated environment.
    Run {
        /// Command and arguments to run.
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Update all packages to their latest versions.
    Update,

    /// Export environment data.
    Export {
        /// Export format (currently only "sbom").
        #[arg(required = true)]
        format: String,

        /// Output file path (defaults to stdout).
        #[arg(long, short)]
        output: Option<String>,
    },

    /// Update envo itself to the latest version.
    SelfUpdate {
        /// Only check for updates without installing.
        #[arg(long)]
        check: bool,
    },

    /// Show envo version, install location, and system info.
    Version {
        /// Output as JSON (for programmatic consumption by IDE extensions and MCP server).
        #[arg(long)]
        json: bool,
    },
}
