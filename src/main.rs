mod agent;
mod cli;
mod config;
mod executor;
mod providers;
mod render;
mod security;
mod sessions;
mod tools;
mod ui;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;

use crate::cli::{Cli, Command};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        crate::ui::report_cli_error(&error);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Some(prompt) = cli.prompt {
        config::ensure_configured()?;
        return agent::run_headless(&prompt).await;
    }

    match cli.command.unwrap_or(Command::Chat { session: None }) {
        Command::Chat { session } => {
            config::ensure_configured()?;
            agent::run_repl(session).await?
        }
        Command::Sessions => sessions::list()?,
        Command::Resume { id } => {
            config::ensure_configured()?;
            agent::run_repl(Some(id)).await?
        }
        Command::Delete { id, all } => sessions::delete(id, all)?,
        Command::Config => config::show(),
        Command::Doctor => config::doctor().await?,
        Command::Models => config::models().await?,
        Command::Completions { shell } => {
            generate(shell, &mut Cli::command(), "hi", &mut std::io::stdout())
        }
    }

    Ok(())
}
