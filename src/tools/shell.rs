use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::executor::Risk;

/// The `run_command` tool: run a terminal command under the user's approval
/// policy. Risk is assessed from the command text, mirror of the shell itself.
pub struct ShellTool;

struct CommandArgs<'a> {
    command: &'a str,
    shell: bool,
    cwd: Option<&'a str>,
}

fn command_args(args: &Value) -> CommandArgs<'_> {
    CommandArgs {
        command: args
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or(""),
        shell: args
            .get("shell")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        cwd: args.get("cwd").and_then(|value| value.as_str()),
    }
}

#[async_trait]
impl super::Tool for ShellTool {
    fn name(&self) -> &'static str {
        super::RUN_COMMAND_TOOL
    }

    fn description(&self) -> &'static str {
        "Run a terminal command after user approval. Prefer a simple executable with arguments; set shell=true only for shell syntax (pipes, redirects, variables, chains). Use read_file, write_file, search and list_dir instead of commands when they fit, and pass cwd when a relative path matters."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to run." },
                "reason": { "type": "string", "description": "What the command is for, shown to the user." },
                "cwd": { "type": "string", "description": "Working directory to run in (default: the current directory)." },
                "shell": {
                    "type": "boolean",
                    "description": "Use the user's shell for pipes, redirects, chains, or shell operators.",
                },
            },
            "required": ["command", "reason"],
        })
    }

    fn risk(&self, args: &Value) -> Risk {
        let args = command_args(args);
        crate::executor::assess(args.command, args.shell)
    }

    fn preview(&self, args: &Value) -> String {
        let args = command_args(args);
        let mut preview = args.command.to_string();
        if args.shell {
            preview.push_str("\n  shell mode enabled");
        }
        if let Some(cwd) = args.cwd {
            preview.push_str(&format!("\n  in {cwd}"));
        }
        preview
    }

    async fn execute(&self, args: &Value) -> Result<String> {
        let args = command_args(args);
        match args.cwd {
            Some(cwd) => {
                crate::executor::execute_in(
                    args.command,
                    args.shell,
                    Some(std::path::Path::new(cwd)),
                )
                .await
            }
            None => crate::executor::execute(args.command, args.shell).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;

    fn args(command: &str, shell: bool) -> Value {
        json!({ "command": command, "reason": "test", "shell": shell })
    }

    #[test]
    fn assesses_risk_from_command_text() {
        let tool = ShellTool;
        assert_eq!(tool.risk(&args("pwd", false)), Risk::Safe);
        assert_eq!(tool.risk(&args("git status | head", true)), Risk::Caution);
        assert_eq!(tool.risk(&args("sudo rm -rf /", true)), Risk::Dangerous);
    }

    #[test]
    fn preview_flags_shell_mode() {
        let tool = ShellTool;
        assert_eq!(tool.preview(&args("ls", false)), "ls");
        assert!(tool
            .preview(&args("ls | grep x", true))
            .contains("shell mode enabled"));
        let cwd_args = json!({ "command": "ls", "reason": "test", "cwd": "/tmp" });
        assert!(tool.preview(&cwd_args).contains("in /tmp"));
    }

    #[tokio::test]
    async fn executes_a_command() {
        let tool = ShellTool;
        let result = tool.execute(&args("printf hello", false)).await.unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn respects_working_directory() {
        let tool = ShellTool;
        let result = tool
            .execute(&json!({ "command": "pwd", "reason": "test", "cwd": "/tmp" }))
            .await
            .unwrap();
        assert!(result.contains("/tmp"));
    }
}
