use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::{
    process::Command,
    time::{timeout, Duration},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Caution,
    Dangerous,
}

pub fn assess(command: &str, shell: bool) -> Risk {
    let lower = command.to_ascii_lowercase();
    if lower.contains("sudo")
        || lower.contains("rm -rf")
        || lower.contains("mkfs")
        || lower.contains("dd if=")
        || lower.contains("shutdown")
        || lower.contains("reboot")
    {
        return Risk::Dangerous;
    }
    if shell
        || lower.contains(" > ")
        || lower.contains(" | ")
        || lower.contains("&&")
        || lower.contains(";")
        || lower.contains("chmod")
        || lower.contains("mv ")
        || lower.contains("install ")
    {
        return Risk::Caution;
    }
    Risk::Safe
}

pub fn auto_approved(mode: &str, risk: Risk) -> bool {
    mode == "never" || (mode == "safe-only" && risk == Risk::Safe)
}

pub async fn execute(command: &str, shell: bool) -> Result<String> {
    execute_with_timeout(command, shell, Duration::from_secs(30)).await
}

pub async fn execute_if_approved(command: &str, shell: bool, approved: bool) -> Result<String> {
    if !approved {
        return Ok("Command rejected by user.".into());
    }
    execute(command, shell).await
}

pub async fn execute_with_timeout(command: &str, shell: bool, limit: Duration) -> Result<String> {
    let mut process = if shell {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    } else {
        let parts = shell_words::split(command).context("could not parse command arguments")?;
        let (program, args) = parts.split_first().context("command was empty")?;
        let mut process = Command::new(program);
        process.args(args);
        process
    };
    let output = timeout(
        limit,
        process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .context("command timed out")??;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let mut result = if output.status.success() {
        if stdout.is_empty() {
            "Command completed successfully.".into()
        } else {
            stdout
        }
    } else if stderr.is_empty() {
        "Command failed without an error message.".into()
    } else {
        stderr
    };
    result.truncate(20_000);
    Ok(crate::security::redact(&result))
}

#[cfg(test)]
mod tests {
    use super::{assess, Risk};
    use std::time::Duration;

    #[test]
    fn detects_command_risk() {
        assert_eq!(assess("pwd", false), Risk::Safe);
        assert_eq!(assess("git status | head", true), Risk::Caution);
        assert_eq!(assess("sudo rm -rf build", true), Risk::Dangerous);
    }

    #[test]
    fn applies_approval_policy() {
        assert!(super::auto_approved("never", Risk::Dangerous));
        assert!(super::auto_approved("safe-only", Risk::Safe));
        assert!(!super::auto_approved("safe-only", Risk::Caution));
        assert!(!super::auto_approved("always", Risk::Safe));
    }

    #[tokio::test]
    async fn executes_structured_command_without_shell() {
        let result = super::execute("printf hello", false).await.unwrap();
        assert!(result.contains("hello"));
        assert!(!result.contains("stdout:"));
        assert!(!result.contains("exit_code:"));
    }

    #[tokio::test]
    async fn supports_quoting_and_explicit_shell_mode() {
        let quoted = super::execute("printf 'hello world'", false).await.unwrap();
        let piped = super::execute("printf hello | tr a-z A-Z", true)
            .await
            .unwrap();
        assert!(quoted.contains("hello world"));
        assert!(piped.contains("HELLO"));
    }

    #[tokio::test]
    async fn enforces_timeout_and_output_limit() {
        let timed_out =
            super::execute_with_timeout("sleep 1", true, Duration::from_millis(5)).await;
        assert!(timed_out.is_err());
        let command = format!("printf '{} '", "x".repeat(25_000));
        let output = super::execute(&command, true).await.unwrap();
        assert!(output.len() <= 20_000);
    }

    #[tokio::test]
    async fn rejected_commands_never_reach_executor() {
        let result = super::execute_if_approved("printf should-not-run", false, false)
            .await
            .unwrap();
        assert_eq!(result, "Command rejected by user.");
    }

    #[tokio::test]
    async fn handles_unicode_and_multiline_shell_output() {
        let output = super::execute("printf 'héllo\\nsecond line'", true)
            .await
            .unwrap();
        assert!(output.contains("héllo"));
        assert!(output.contains("second line"));
    }
}
