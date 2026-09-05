use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "hi",
    version,
    about = "A local-first terminal AI assistant",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    /// Ask a single question and print the response, without starting the REPL.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start a chat session, optionally continuing an existing one.
    Chat {
        #[arg(long)]
        session: Option<String>,
    },
    /// List saved sessions.
    Sessions,
    /// Resume a saved session.
    Resume { id: String },
    /// Delete one session or all sessions.
    Delete {
        id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Display configuration guidance.
    Config,
    /// Check local configuration and provider connectivity.
    Doctor,
    /// List models exposed by the configured provider.
    Models,
    /// Generate shell completion scripts.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}
