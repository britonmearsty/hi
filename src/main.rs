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
async fn main() -> Result<()> {
    let cli = Cli::parse();

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
