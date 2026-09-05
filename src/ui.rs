use crate::executor::Risk;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const CYAN: &str = "\x1b[36m";
pub const YELLOW: &str = "\x1b[33m";
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";

/// Print an error raised while the REPL is already running, keeping the
/// session alive. The whole cause chain is flattened onto a single line so the
/// user sees the actionable message without a stack dump.
pub fn report_error(error: &anyhow::Error) {
    eprintln!("{RED}hi:{RESET} {error:#}");
}

/// Print an error for a top-level CLI command, including the cause chain, and
/// exit non-zero.
pub fn report_cli_error(error: &anyhow::Error) {
    eprintln!("{RED}hi:{RESET} {error:?}");
}

/// Stateful prompt for the REPL: shows the working directory, session id and
/// the configured command-approval policy, colored by risk.
pub struct Prompt {
    cwd: String,
    session: String,
    approval: String,
}

impl Prompt {
    pub fn new(session: &str, cwd: &Path, approval: &str) -> Self {
        let name = cwd
            .file_name()
            .map(|part| part.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd.display().to_string());
        Self {
            cwd: name,
            session: session.to_owned(),
            approval: approval.to_owned(),
        }
    }

    fn approval_indicator(&self) -> String {
        match self.approval.as_str() {
            "always" => format!("{GREEN}ask{RESET}"),
            "safe-only" => format!("{YELLOW}safe{RESET}"),
            "never" => format!("{RED}auto{RESET}"),
            mode => format!("{DIM}{mode}{RESET}"),
        }
    }

    pub fn render(&self) -> String {
        format!(
            "{DIM}{}{RESET} {CYAN}{}{RESET} {} \x1b[3;36m❯\x1b[0m ",
            self.cwd,
            self.session,
            self.approval_indicator()
        )
    }
}

pub fn banner(session_id: &str, provider: &str, model: &str) -> String {
    format!(
        "Session: {CYAN}{session_id}{RESET} · Provider: {YELLOW}{provider}{RESET} · Model: {GREEN}{model}{RESET}\n\
         {DIM}Type /help for commands or /quit to exit.{RESET}"
    )
}
pub fn command_icon() -> &'static str {
    "⚙"
}
pub fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => "✓ Safe",
        Risk::Caution => "⚠ Caution",
        Risk::Dangerous => "✗ Dangerous",
    }
}
pub async fn loader_until(stop: Arc<AtomicBool>) {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut index = 0;
    while !stop.load(Ordering::Relaxed) {
        print!("\r{DIM}{} thinking...{RESET}", frames[index % frames.len()]);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        tokio::time::sleep(std::time::Duration::from_millis(90)).await;
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{banner, Prompt};
    use std::path::Path;

    #[test]
    fn renders_prompt_with_context() {
        let rendered =
            Prompt::new("abc123", Path::new("/tmp/hi-prompt-home/work"), "safe-only").render();
        assert!(rendered.contains("\x1b[2mwork\x1b[0m"));
        assert!(rendered.contains("abc123"));
        assert!(rendered.contains("\x1b[33msafe\x1b[0m"));
        assert!(rendered.contains("❯"));
    }

    #[test]
    fn colors_approval_by_policy() {
        let path = Path::new("/tmp");
        assert!(Prompt::new("s", path, "always")
            .render()
            .contains("\x1b[32mask"));
        assert!(Prompt::new("s", path, "safe-only")
            .render()
            .contains("\x1b[33msafe"));
        assert!(Prompt::new("s", path, "never")
            .render()
            .contains("\x1b[31mauto"));
    }

    #[test]
    fn renders_banner_with_colored_values() {
        let rendered = banner("abc123", "openai", "gpt-4o-mini");
        assert!(rendered.contains("\x1b[36mabc123\x1b[0m"));
        assert!(rendered.contains("\x1b[33mopenai\x1b[0m"));
        assert!(rendered.contains("\x1b[32mgpt-4o-mini\x1b[0m"));
    }
}
