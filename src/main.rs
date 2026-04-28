#![allow(dead_code, unused_variables, unused_imports)]

mod app;
mod buffer;
mod filter;
mod index;
mod parser;
mod poller;
mod register;
mod search;
mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "lazylog", about = "Tiny portable TUI log viewer")]
struct Cli {
    /// Log file to open
    file: Option<PathBuf>,

    /// Follow mode: scroll as file grows
    #[arg(short, long)]
    follow: bool,

    /// Force log format (auto-detected by default)
    #[arg(long, value_name = "json|text")]
    format: Option<String>,

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

    let stdin_mode = !std::io::stdin().is_terminal() && cli.file.is_none();

    if !stdin_mode && cli.file.is_none() {
        eprintln!("Usage: lazylog <file.log> [--follow]");
        eprintln!("       lazylog register");
        eprintln!("       cat app.log | lazylog");
        std::process::exit(1);
    }

    app::run(app::Args {
        file_path: cli.file,
        follow: cli.follow,
        stdin_mode,
    })
}
