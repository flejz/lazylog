#![allow(dead_code, unused_variables, unused_imports)]

mod app;
mod buffer;
mod config;
mod filter;
mod index;
mod parser;
mod poller;
mod presets;
mod register;
mod search;
mod time_parse;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lazylog", about = "Tiny portable TUI log viewer for any log format")]
struct Cli {
    /// Log files to open (multiple files merged chronologically)
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Follow mode: scroll as file grows
    #[arg(short, long)]
    follow: bool,

    /// Force log format (auto-detected by default)
    #[arg(long, value_name = "json|text")]
    format: Option<String>,

    /// Path to a TOML config file. Defaults to <config_dir>/lazylog/config.toml.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register .log file association for this binary
    Register,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Register) = cli.command {
        return register::register();
    }

    let stdin_mode = !std::io::stdin().is_terminal() && cli.files.is_empty();

    if !stdin_mode && cli.files.is_empty() {
        eprintln!("Usage: lazylog <file.log> [file2.log ...] [--follow]");
        eprintln!("       lazylog register");
        eprintln!("       cat app.log | lazylog");
        std::process::exit(1);
    }

    app::run(app::Args {
        file_paths: cli.files,
        follow: cli.follow,
        stdin_mode,
        config_path: cli.config,
    })
}
