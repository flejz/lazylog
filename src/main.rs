#![allow(dead_code, unused_variables, unused_imports)]

mod app;
mod browser;
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

    /// Browse a directory and pick log files interactively.
    /// Start dir: $LAZYLOG_DIR if set, else current dir.
    #[arg(short, long)]
    browse: bool,

    /// Override the initial browse directory (also available as $LAZYLOG_DIR).
    #[arg(long, value_name = "DIR")]
    browse_dir: Option<PathBuf>,

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

fn resolve_browse_dir(arg: Option<PathBuf>) -> PathBuf {
    if let Some(p) = arg {
        return p;
    }
    if let Ok(env) = std::env::var("LAZYLOG_DIR") {
        let p = PathBuf::from(env);
        if p.is_dir() {
            return p;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Register) = cli.command {
        return register::register();
    }

    let stdin_mode = !std::io::stdin().is_terminal() && cli.files.is_empty();

    // Browser launch: explicit -b flag only. Files on CLI win over -b.
    if cli.browse && cli.files.is_empty() && !stdin_mode {
        let start = resolve_browse_dir(cli.browse_dir);
        return browser::run(browser::Args {
            start_dir: start,
            follow: cli.follow,
            config_path: cli.config,
        });
    }

    if !stdin_mode && cli.files.is_empty() {
        eprintln!("Usage: lazylog <file.log> [file2.log ...] [--follow]");
        eprintln!("       lazylog --browse  (or set LAZYLOG_DIR)");
        eprintln!("       lazylog register");
        eprintln!("       cat app.log | lazylog");
        std::process::exit(1);
    }

    app::run(app::Args {
        file_paths: cli.files,
        follow: cli.follow,
        stdin_mode,
        config_path: cli.config,
        embedded: false,
    })
}
