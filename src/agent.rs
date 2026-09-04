use crate::{
    config, executor,
    providers::{FunctionCall, Message, OpenAiCompatibleProvider, Provider, ToolCall},
    sessions,
};
use anyhow::{Context, Result};
use rustyline::{
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    validate::Validator,
    Context as ReadlineContext, Editor, Helper,
};
use std::io::{self, Write};

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/sessions",
    "/clear",
    "/model",
    "/new",
    "/quit",
    "/exit",
];

#[derive(Clone, Copy, Default)]
struct SlashHelper;
impl Helper for SlashHelper {}
impl Hinter for SlashHelper {
    type Hint = String;
}
impl Highlighter for SlashHelper {}
impl Validator for SlashHelper {}
impl Completer for SlashHelper {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &ReadlineContext<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let prefix = &line[..pos];
        if !prefix.starts_with('/') || prefix.contains(' ') {
            return Ok((pos, Vec::new()));
        }
        let matches = SLASH_COMMANDS
            .iter()
            .filter(|command| command.starts_with(prefix))
            .map(|command| Pair {
                display: (*command).into(),
                replacement: (*command).into(),
            })
            .collect();
        Ok((0, matches))
    }
}

fn message(role: &str, content: Option<String>) -> Message {
    Message {
        role: role.into(),
        content,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    }
}

pub async fn run_repl(session: Option<String>) -> Result<()> {
    let session_id = match session {
        Some(id) => {
            sessions::load(&id).context("could not open session")?;
            id
        }
        None => sessions::create()?,
    };
    let mut messages = sessions::load(&session_id)?;
    if messages.is_empty() {
        messages.push(message("system", Some("You are hi, a helpful terminal assistant. You may propose shell commands with run_command, but the application will require explicit user approval before executing them. Never claim a command succeeded without its result.".into())));
    }
    let provider = OpenAiCompatibleProvider::new(config::load()?);
    println!("hi (session {session_id})\nType /help for commands or /quit to exit.");

    let mut editor = Editor::<SlashHelper, rustyline::history::DefaultHistory>::new()?;
    editor.set_helper(Some(SlashHelper));
    loop {
        let mut input = match editor.readline(&format!("\n{} ", crate::ui::prompt())) {
            Ok(input) => input,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
        };
        if !input.trim().is_empty() {
            editor.add_history_entry(input.as_str())?;
        }
        while input.trim_end().ends_with('\\') {
            input = input.trim_end().trim_end_matches('\\').to_owned();
            let continuation = match editor.readline("  ") {
                Ok(value) => value,
                Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
                Err(error) => return Err(error.into()),
            };
            input.push_str(&continuation);
        }
        let input = input.trim();
        if input.starts_with('/') {
            let command = input.split_whitespace().next().unwrap_or(input);
            match command {
                "/quit" | "/exit" => break,
                "/help" => {
                    println!("/help, /sessions, /clear, /model, /new, /quit\nSession: {session_id}")
                }
                "/sessions" => sessions::list()?,
                "/clear" => {
                    sessions::clear_messages(&session_id)?;
                    messages.retain(|message| message.role == "system");
                    println!("Session context cleared.");
                }
                "/model" => {
                    let config = config::load()?;
                    println!("Provider: {}\nModel: {}", config.provider, config.model);
                }
                "/new" => {
                    println!("Start another session with `hi` or `hi chat`.");
                    break;
                }
                _ => println!(
                    "Unknown local command `{command}`. Type /help for available commands."
                ),
            }
            continue;
        }
        match input {
            "" => continue,
            text => {
                let user = message("user", Some(text.into()));
                if messages.iter().all(|message| message.role == "system") {
                    sessions::set_title(&session_id, text)?;
                }
                sessions::save_message(&session_id, &user)?;
                messages.push(user);
                run_turn(&provider, &session_id, &mut messages).await?;
            }
        }
    }
    Ok(())
}

async fn run_turn(
    provider: &impl Provider,
    session_id: &str,
    messages: &mut Vec<Message>,
) -> Result<()> {
    loop {
        let loader = tokio::spawn(crate::ui::loader());
        let mut streamed_text = String::new();
        let mut on_text = |text: String| streamed_text.push_str(&text);
        let context = build_context(provider, session_id, messages).await?;
        let response = provider.respond(&context, &mut on_text).await;
        loader.abort();
        print!("\r\x1b[2K");
        let response = response?;
        let assistant = response.message;
        if !streamed_text.is_empty() {
            println!("\n{}", crate::render::render_markdown(&streamed_text));
        }
        sessions::save_message(session_id, &assistant)?;
        messages.push(assistant.clone());
        if let Some(calls) = assistant.tool_calls.clone() {
            for call in calls {
                handle_tool_call(session_id, messages, call).await?;
            }
            continue;
        }
        println!();
        break;
    }
    Ok(())
}

async fn build_context(
    provider: &impl Provider,
    session_id: &str,
    messages: &[Message],
) -> Result<Vec<Message>> {
    let mut system = messages
        .first()
        .cloned()
        .unwrap_or_else(|| message("system", None));
    if messages.len() > 41 {
        let older = &messages[1..messages.len() - 40];
        let summary = provider.summarize(older).await?;
        sessions::set_summary(session_id, &summary)?;
        system.content = Some(format!(
            "{}\n\nConversation summary:\n{}",
            system.content.unwrap_or_default(),
            summary
        ));
        let mut context = vec![system];
        context.extend(messages[messages.len() - 40..].iter().cloned());
        Ok(context)
    } else if let Some(summary) = sessions::get_summary(session_id)? {
        system.content = Some(format!(
            "{}\n\nConversation summary:\n{}",
            system.content.unwrap_or_default(),
            summary
        ));
        let mut context = vec![system];
        context.extend(messages.iter().skip(1).cloned());
        Ok(context)
    } else {
        Ok(messages.to_vec())
    }
}

async fn handle_tool_call(
    session_id: &str,
    messages: &mut Vec<Message>,
    call: ToolCall,
) -> Result<()> {
    if call.function.name != crate::tools::RUN_COMMAND_TOOL {
        anyhow::bail!("unknown tool: {}", call.function.name);
    }
    let args: serde_json::Value =
        serde_json::from_str(&call.function.arguments).context("invalid command tool arguments")?;
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .context("tool call did not include a command")?;
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("The assistant requested this command.");
    let shell = args
        .get("shell")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let risk = executor::assess(command, shell);
    println!(
        "\n{} AI command\n  {}\n{} Reason: {}",
        crate::ui::command_icon(),
        command,
        crate::ui::risk_label(risk),
        reason
    );
    if shell {
        println!("  shell mode enabled");
    }
    let approval_mode = config::load()?.approval_mode;
    let approved = if executor::auto_approved(&approval_mode, risk) {
        if approval_mode == "never" {
            println!("  Auto-approved by configured policy.");
        } else {
            println!("  Auto-approved as safe.");
        }
        true
    } else {
        print!("  Allow this command? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    };
    let result = executor::execute_if_approved(command, shell, approved).await?;
    println!("\n{result}");
    let tool_message = Message {
        role: "tool".into(),
        content: Some(result),
        name: Some(call.function.name),
        tool_call_id: Some(call.id),
        tool_calls: None,
    };
    sessions::save_message(session_id, &tool_message)?;
    messages.push(tool_message);
    Ok(())
}

#[allow(dead_code)]
fn _keep_function_call_visible(_: FunctionCall) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Response;

    struct MockProvider;

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn respond(
            &self,
            _messages: &[Message],
            on_text: &mut (dyn FnMut(String) + Send),
        ) -> Result<Response> {
            on_text("mock response".into());
            Ok(Response {
                message: message("assistant", Some("mock response".into())),
            })
        }

        async fn summarize(&self, _messages: &[Message]) -> Result<String> {
            Ok("mock summary".into())
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn agent_loop_accepts_mock_provider() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let session = sessions::create().unwrap();
        let mut messages = vec![message("user", Some("hello".into()))];
        run_turn(&MockProvider, &session, &mut messages)
            .await
            .unwrap();
        assert_eq!(
            messages.last().unwrap().content.as_deref(),
            Some("mock response")
        );
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn compacts_long_context_into_persisted_summary() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let session = sessions::create().unwrap();
        let mut messages = vec![message("system", Some("system instructions".into()))];
        for index in 0..45 {
            messages.push(message("user", Some(format!("message {index}"))));
        }
        let context = build_context(&MockProvider, &session, &messages)
            .await
            .unwrap();
        assert_eq!(context.len(), 41);
        assert!(context[0]
            .content
            .as_deref()
            .unwrap()
            .contains("mock summary"));
        assert_eq!(
            sessions::get_summary(&session).unwrap().as_deref(),
            Some("mock summary")
        );
        std::env::remove_var("XDG_DATA_HOME");
    }
}
