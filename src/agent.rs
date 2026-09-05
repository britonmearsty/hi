use crate::{
    config, executor,
    providers::{FunctionCall, Message, Provider, ToolCall},
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
use std::io::{self, IsTerminal, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/sessions",
    "/resume",
    "/reset",
    "/clear",
    "/model",
    "/config",
    "/doctor",
    "/new",
    "/quit",
    "/exit",
];

const COMMAND_DOCS: &[(&str, &str)] = &[
    (
        "/help [command]",
        "Show help for all commands, or explain one",
    ),
    ("/sessions", "List saved sessions"),
    (
        "/resume <session-id>",
        "Switch to another session and keep chatting here",
    ),
    ("/clear", "Clear the terminal screen"),
    ("/reset", "Forget this session's conversation history"),
    (
        "/model [name]",
        "Show the current model, or switch to another one",
    ),
    ("/config", "Show provider, URL, model and approval settings"),
    ("/doctor", "Check whether the provider is reachable"),
    ("/new", "Leave this session and start afresh"),
    ("/quit /exit", "Leave hi"),
];

const SYSTEM_PROMPT: &str = "You are hi, a helpful terminal assistant. You are given a set of tools to accomplish tasks; inspect and modify state through them. The application requires explicit user approval before any action that affects the system. Never claim an action succeeded without its result. When you use a tool, briefly tell the user what you found or did.";

/// Render rows as a two-column table drawn with box-drawing characters,
/// colored to match the rest of the UI. Column widths are derived from the
/// content and clamped to the terminal width so the box never wraps.
fn help_table(rows: &[(&str, &str)]) -> String {
    let term_width = crate::render::terminal_width();
    let first = rows
        .iter()
        .map(|(a, _)| a.chars().count())
        .max()
        .unwrap_or(0);
    let longest = rows
        .iter()
        .map(|(_, b)| b.chars().count())
        .max()
        .unwrap_or(0);
    let second = longest.min(term_width.saturating_sub(first + 7).max(10));

    let pad = |text: &str, width: usize| -> String {
        if text.chars().count() > width {
            let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
            cut.push('…');
            cut
        } else {
            let mut value = text.to_owned();
            while value.chars().count() < width {
                value.push(' ');
            }
            value
        }
    };

    let border = |left: char, mid: char, right: char| {
        format!(
            "{0}{1}{2}{3}{4}{5}{6}",
            crate::ui::DIM,
            left,
            "─".repeat(first + 2),
            mid,
            "─".repeat(second + 2),
            right,
            crate::ui::RESET
        )
    };

    let mut out = String::new();
    out.push_str(&border('┌', '┬', '┐'));
    out.push('\n');
    for (index, (usage, description)) in rows.iter().enumerate() {
        let header = rows.len() > 1 && index == 0;
        let command = if header {
            pad(usage, first)
        } else {
            format!(
                "{0}{1}{2}",
                crate::ui::CYAN,
                pad(usage, first),
                crate::ui::RESET
            )
        };
        out.push_str(&format!("│ {command} │ {} │\n", pad(description, second)));
        if header {
            out.push_str(&border('├', '┼', '┤'));
            out.push('\n');
        }
    }
    out.push_str(&border('└', '┴', '┘'));
    out.push('\n');
    out
}

fn help_text(command: Option<&str>) -> String {
    let Some(requested) = command else {
        let mut rows: Vec<(&str, &str)> = COMMAND_DOCS.to_vec();
        rows.insert(0, ("Command", "Description"));
        return help_table(&rows);
    };
    match COMMAND_DOCS
        .iter()
        .find(|(usage, _)| usage.split_whitespace().next() == Some(requested))
    {
        Some(&(usage, description)) => help_table(&[(usage, description)]),
        None => format!("No help available for `{requested}`. Try /help.\n"),
    }
}

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

#[derive(Debug)]
enum LoopAction {
    Continue,
    Quit,
}

/// Handle one line of REPL input: a slash command or a message. A single
/// failed turn (e.g. a transient provider error) is reported by the caller and
/// does not tear down the session. Command errors are raised as Err so the
/// caller can display them and keep the REPL alive.
async fn process_line(
    provider: &mut Box<dyn Provider>,
    session_id: &mut String,
    messages: &mut Vec<Message>,
    input: &str,
) -> Result<LoopAction> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(LoopAction::Continue);
    }
    if !input.starts_with('/') {
        let user = message("user", Some(input.into()));
        if messages.iter().all(|message| message.role == "system") {
            sessions::set_title(session_id, input)?;
        }
        sessions::save_message(session_id, &user)?;
        messages.push(user);
        run_turn(provider.as_ref(), session_id, messages).await?;
        return Ok(LoopAction::Continue);
    }

    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or(input);
    let argument = parts.next();
    match command {
        "/quit" | "/exit" => return Ok(LoopAction::Quit),
        "/help" => {
            print!("{}", help_text(argument));
            let _ = io::stdout().flush();
            return Ok(LoopAction::Continue);
        }
        "/sessions" => {
            sessions::list()?;
            return Ok(LoopAction::Continue);
        }
        "/resume" => {
            let Some(id) = argument else {
                anyhow::bail!("usage: /resume <session-id>; find one with /sessions");
            };
            let mut resumed = sessions::load(id)
                .with_context(|| format!("no session `{id}`; list them with /sessions"))?;
            if resumed.is_empty() {
                resumed.push(message("system", Some(SYSTEM_PROMPT.into())));
            }
            *session_id = id.into();
            *messages = resumed;
            print!("{}", render_transcript(messages));
            let _ = io::stdout().flush();
            return Ok(LoopAction::Continue);
        }
        "/clear" => {
            print!("\x1b[2J\x1b[3J\x1b[H");
            let _ = io::stdout().flush();
            return Ok(LoopAction::Continue);
        }
        "/reset" => {
            sessions::clear_messages(session_id)?;
            messages.retain(|message| message.role == "system");
            println!("Session context cleared. The next message starts fresh.");
            return Ok(LoopAction::Continue);
        }
        "/model" => match argument {
            Some(name) => {
                let mut config = config::load()?;
                config.model = name.into();
                *provider = crate::providers::create(&config)?;
                println!("Model switched to `{name}`; it applies from the next message.");
            }
            None => {
                let config = config::load()?;
                println!("Provider: {}\nModel: {}", config.provider, config.model);
            }
        },
        "/config" => {
            let config = config::load()?;
            println!(
                "Provider: {}\nBase URL: {}\nModel: {}\nApproval: {}",
                config.provider, config.base_url, config.model, config.approval_mode
            );
        }
        "/doctor" => {
            config::doctor().await?;
        }
        "/new" => {
            println!("Start a fresh session with `hi` or `hi chat`.");
            return Ok(LoopAction::Quit);
        }
        _ => {
            println!("Unknown command `{command}`. Type /help for available commands.");
        }
    }
    Ok(LoopAction::Continue)
}

pub async fn run_repl(session: Option<String>) -> Result<()> {
    let mut session_id = match session {
        Some(id) => {
            sessions::load(&id).context("could not open session")?;
            id
        }
        None => sessions::create()?,
    };
    let mut messages = sessions::load(&session_id)?;
    if messages.is_empty() {
        messages.push(message("system", Some(SYSTEM_PROMPT.into())));
    }
    let config = config::load()?;
    let mut provider: Box<dyn Provider> = crate::providers::create(&config)?;
    println!(
        "{}",
        crate::ui::banner(&session_id, &config.provider, &config.model)
    );
    print!("{}", render_transcript(&messages));
    let _ = io::stdout().flush();
    let current_dir = std::env::current_dir().unwrap_or_default();

    let mut editor = Editor::<SlashHelper, rustyline::history::DefaultHistory>::new()?;
    editor.set_helper(Some(SlashHelper));
    loop {
        let prompt = crate::ui::Prompt::new(&session_id, &current_dir, &config.approval_mode);
        let mut input = match editor.readline(&format!("\n{}", prompt.render())) {
            Ok(input) => input,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(error) => return Err(error.into()),
        };
        if !input.trim().is_empty() {
            editor.add_history_entry(input.as_str())?;
        }
        while input.trim_end().ends_with('\\') {
            input = input.trim_end().trim_end_matches('\\').to_owned();
            match editor.readline("  ") {
                Ok(value) => input.push_str(&value),
                Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                    input.clear();
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        if input.trim().is_empty() {
            continue;
        }
        let action = match process_line(&mut provider, &mut session_id, &mut messages, &input).await
        {
            Ok(action) => action,
            Err(error) => {
                crate::ui::report_error(&error);
                LoopAction::Continue
            }
        };
        if matches!(action, LoopAction::Quit) {
            break;
        }
    }
    print!("{}", crate::ui::DIM);
    println!("Session `{session_id}` saved. To pick up where you left off, run:");
    println!("  hi resume {session_id}");
    print!("{}", crate::ui::RESET);
    Ok(())
}

async fn run_turn(
    provider: &(impl Provider + ?Sized),
    session_id: &str,
    messages: &mut Vec<Message>,
) -> Result<()> {
    loop {
        println!();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_loader = stop.clone();
        let loader = tokio::spawn(async move {
            crate::ui::loader_until(stop_loader).await;
        });
        let mut renderer = crate::render::StreamingRenderer::new();
        let outcome = async {
            let context = build_context(provider, session_id, messages).await?;
            let mut on_text = |text: String| {
                stop.store(true, Ordering::Relaxed);
                print!("\r\x1b[2K");
                let _ = io::stdout().flush();
                renderer.feed(&text);
                renderer.paint();
            };
            provider.respond(&context, &mut on_text).await
        }
        .await;
        stop.store(true, Ordering::Relaxed);
        loader.abort();
        print!("\r\x1b[2K");
        let _ = io::stdout().flush();
        let response = outcome?;
        let assistant = response.message;
        renderer.finish();
        renderer.paint();
        sessions::save_message(session_id, &assistant)?;
        messages.push(assistant.clone());
        if let Some(calls) = assistant.tool_calls.clone() {
            for call in calls {
                if let Err(error) = handle_tool_call(session_id, messages, call.clone()).await {
                    crate::ui::report_error(&error);
                    let tool_message = Message {
                        role: "tool".into(),
                        content: Some(format!(
                            "Running `{}` failed: {error:#}",
                            call.function.name
                        )),
                        name: Some(call.function.name),
                        tool_call_id: Some(call.id),
                        tool_calls: None,
                    };
                    sessions::save_message(session_id, &tool_message)?;
                    messages.push(tool_message);
                }
            }
            continue;
        }
        println!();
        break;
    }
    Ok(())
}

/// Render the saved conversation for display when resuming a session, so the
/// prior exchange is visible before the user starts typing.
fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for (index, message) in messages.iter().enumerate() {
        if index == 0 && message.role == "system" {
            continue;
        }
        match message.role.as_str() {
            "user" => {
                if let Some(content) = &message.content {
                    out.push_str(&format!(
                        "{0}❯{1} {2}\n",
                        crate::ui::CYAN,
                        crate::ui::RESET,
                        content
                    ));
                }
            }
            "assistant" => {
                if let Some(content) = &message.content {
                    out.push('\n');
                    let mut renderer = crate::render::StreamingRenderer::new();
                    out.push_str(&renderer.render_once(content));
                }
            }
            _ => {}
        }
    }
    out.trim_end().to_owned()
}

/// Answer a single prompt non-interactively and print the response, without
/// running the interactive REPL. The exchange is still saved as a session so
/// it can be resumed later.
pub async fn run_headless(prompt: &str) -> Result<()> {
    let config = config::load()?;
    let provider = crate::providers::create(&config)?;
    run_headless_with(provider.as_ref(), prompt).await?;
    Ok(())
}

async fn run_headless_with(provider: &(impl Provider + ?Sized), prompt: &str) -> Result<String> {
    let session_id = sessions::create()?;
    let mut messages = vec![message("system", Some(SYSTEM_PROMPT.into()))];
    let user = message("user", Some(prompt.into()));
    sessions::save_message(&session_id, &user)?;
    messages.push(user);

    for _ in 0..10 {
        let context = build_context(provider, &session_id, &messages).await?;
        let mut on_text = |_text: String| {};
        let response = provider.respond(&context, &mut on_text).await?;
        let assistant = response.message;
        sessions::save_message(&session_id, &assistant)?;
        if let Some(calls) = assistant.tool_calls.clone() {
            messages.push(assistant);
            for call in calls {
                let result = run_headless_tool_call(&call).await.unwrap_or_else(|error| {
                    format!("Running `{}` failed: {error:#}", call.function.name)
                });
                let tool_message = Message {
                    role: "tool".into(),
                    content: Some(result),
                    name: Some(call.function.name),
                    tool_call_id: Some(call.id),
                    tool_calls: None,
                };
                sessions::save_message(&session_id, &tool_message)?;
                messages.push(tool_message);
            }
            continue;
        }
        if let Some(content) = assistant.content.clone() {
            if !content.trim().is_empty() {
                sessions::set_title(&session_id, prompt)?;
            }
        }
        let mut renderer = crate::render::StreamingRenderer::new();
        let styled = renderer.render_once(&assistant.content.clone().unwrap_or_default());
        if io::stdout().is_terminal() {
            print!("{styled}");
        } else {
            print!("{}", assistant.content.unwrap_or_default());
        }
        io::stdout().flush()?;
        return Ok(session_id);
    }
    Ok(session_id)
}

/// Execute one tool call headless: apply the configured approval policy but
/// never prompt interactively. Unapproved actions report rejection so the
/// model can adapt.
async fn run_headless_tool_call(call: &ToolCall) -> Result<String> {
    let tool = crate::tools::get(&call.function.name)
        .with_context(|| format!("unknown tool: {}", call.function.name))?;
    let args: serde_json::Value =
        serde_json::from_str(&call.function.arguments).context("invalid tool arguments")?;
    let risk = tool.risk(&args);
    let approval_mode = config::load()?.approval_mode;
    if executor::auto_approved(&approval_mode, risk) {
        tool.execute(&args).await
    } else {
        Ok("Action rejected in headless mode; run `hi chat` to approve it interactively.".into())
    }
}

async fn build_context(
    provider: &(impl Provider + ?Sized),
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
    let tool = crate::tools::get(&call.function.name)
        .with_context(|| format!("unknown tool: {}", call.function.name))?;
    let args: serde_json::Value =
        serde_json::from_str(&call.function.arguments).context("invalid tool arguments")?;
    let reason = args
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("The assistant requested this action.");
    let risk = tool.risk(&args);
    println!(
        "\n{} AI {}\n  {}\n{} Reason: {}",
        crate::ui::command_icon(),
        tool.name(),
        tool.preview(&args),
        crate::ui::risk_label(risk),
        reason
    );
    let approval_mode = config::load()?.approval_mode;
    let approved = if executor::auto_approved(&approval_mode, risk) {
        if approval_mode == "never" {
            println!("  Auto-approved by configured policy.");
        } else {
            println!("  Auto-approved as safe.");
        }
        true
    } else {
        print!("  Allow this action? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        let read = io::stdin().read_line(&mut answer);
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") && read.is_ok()
    };
    let result = if approved {
        tool.execute(&args).await?
    } else {
        "Action rejected by user.".into()
    };
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
    use crate::providers::{FunctionCall, Response, ToolCall};
    use std::sync::Mutex;

    #[test]
    fn help_table_renders_square_bordered_grid() {
        let table = help_table(&[
            ("Command", "Description"),
            ("/help [x]", "Show help"),
            ("/resume <id>", "Switch"),
        ]);
        let plain = table
            .replace(crate::ui::DIM, "")
            .replace(crate::ui::CYAN, "")
            .replace(crate::ui::RESET, "");
        assert!(plain.starts_with("┌"));
        assert!(plain.contains("┤"));
        assert_eq!(plain.matches("│").count(), 9);
        let lines: Vec<&str> = plain.lines().collect();
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        assert!(widths.iter().all(|w| *w == widths[0]));
    }

    #[test]
    fn per_command_help_returns_table_with_single_row() {
        let table = help_text(Some("/resume"));
        let plain = table
            .replace(crate::ui::DIM, "")
            .replace(crate::ui::CYAN, "")
            .replace(crate::ui::RESET, "");
        assert!(plain.starts_with("┌"));
        assert!(plain.contains("/resume"));
        assert!(plain.contains("Switch to another session"));
        assert!(plain.trim_end().ends_with("┘"));
    }

    #[test]
    fn help_notes_unknown_commands() {
        assert!(help_text(Some("/nope")).contains("No help available for"));
    }

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

    struct MockFailProvider;

    #[async_trait::async_trait]
    impl Provider for MockFailProvider {
        async fn respond(
            &self,
            _messages: &[Message],
            _on_text: &mut (dyn FnMut(String) + Send),
        ) -> Result<Response> {
            anyhow::bail!("provider exploded")
        }

        async fn summarize(&self, _messages: &[Message]) -> Result<String> {
            Ok("mock summary".into())
        }
    }

    struct MockToolProvider {
        turns: Mutex<u32>,
    }

    #[async_trait::async_trait]
    impl Provider for MockToolProvider {
        async fn respond(
            &self,
            _messages: &[Message],
            on_text: &mut (dyn FnMut(String) + Send),
        ) -> Result<Response> {
            let mut turns = self.turns.lock().unwrap();
            *turns += 1;
            if *turns == 1 {
                return Ok(Response {
                    message: Message {
                        role: "assistant".into(),
                        content: None,
                        name: None,
                        tool_call_id: None,
                        tool_calls: Some(vec![ToolCall {
                            kind: "function".into(),
                            id: "call_1".into(),
                            function: FunctionCall {
                                name: crate::tools::RUN_COMMAND_TOOL.into(),
                                arguments: "not valid json".into(),
                            },
                        }]),
                    },
                });
            }
            on_text("recovered".into());
            Ok(Response {
                message: message("assistant", Some("recovered".into())),
            })
        }

        async fn summarize(&self, _messages: &[Message]) -> Result<String> {
            Ok("mock summary".into())
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn slash_commands_and_turn_failures_are_resilient() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let mut session = sessions::create().unwrap();
        let mut messages = vec![message("system", Some(SYSTEM_PROMPT.into()))];

        let mut fail_provider: Box<dyn Provider> = Box::new(MockFailProvider);
        let err = process_line(&mut fail_provider, &mut session, &mut messages, "why?")
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("provider exploded"));

        let mut provider: Box<dyn Provider> = Box::new(MockProvider);
        let quit = process_line(&mut provider, &mut session, &mut messages, "/quit")
            .await
            .unwrap();
        assert!(matches!(quit, LoopAction::Quit));

        let action = process_line(&mut provider, &mut session, &mut messages, "/bogus")
            .await
            .unwrap();
        assert!(matches!(action, LoopAction::Continue));

        let action = process_line(&mut provider, &mut session, &mut messages, "/clear")
            .await
            .unwrap();
        assert!(matches!(action, LoopAction::Continue));
        assert!(session.len() == 8);

        let err = process_line(&mut provider, &mut session, &mut messages, "/resume")
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("usage: /resume"));

        std::env::remove_var("XDG_DATA_HOME");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn resume_switches_session_within_repl() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let mut first = sessions::create().unwrap();
        let second = sessions::create().unwrap();
        let user = message("user", Some("hello from the second session".into()));
        sessions::save_message(&second, &user).unwrap();
        let mut messages = vec![message("system", Some(SYSTEM_PROMPT.into()))];
        let mut provider: Box<dyn Provider> = Box::new(MockProvider);
        let action = process_line(
            &mut provider,
            &mut first,
            &mut messages,
            &format!("/resume {second}"),
        )
        .await
        .unwrap();
        assert!(matches!(action, LoopAction::Continue));
        assert_eq!(first, second);
        assert!(messages
            .iter()
            .any(|m| m.content.as_deref() == Some("hello from the second session")));
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn tool_call_failure_becomes_tool_message() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let session = sessions::create().unwrap();
        let mut messages = vec![message("user", Some("run something".into()))];
        run_turn(
            &MockToolProvider {
                turns: Mutex::new(0),
            },
            &session,
            &mut messages,
        )
        .await
        .unwrap();
        let tool_index = messages.iter().position(|m| m.role == "tool").unwrap();
        assert!(messages[tool_index]
            .content
            .as_deref()
            .unwrap()
            .contains("failed"));
        assert_eq!(
            messages.last().unwrap().content.as_deref(),
            Some("recovered")
        );
        std::env::remove_var("XDG_DATA_HOME");
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

    #[test]
    fn transcript_includes_prior_exchange() {
        let messages = vec![
            message("system", Some(SYSTEM_PROMPT.into())),
            message("user", Some("first question".into())),
            message("assistant", Some("first answer".into())),
            message("user", Some("second question".into())),
            message("assistant", Some("second answer".into())),
        ];
        let transcript = render_transcript(&messages);
        assert!(transcript.contains("first question"));
        assert!(transcript.contains("first answer"));
        assert!(transcript.contains("second question"));
        assert!(transcript.contains("second answer"));
        assert_eq!(transcript.matches("❯").count(), 2);
        assert!(!transcript.contains("You are hi"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn headless_persists_exchange_as_session() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", directory.path());
        let session = run_headless_with(&MockProvider, "what is the capital of france?")
            .await
            .unwrap();
        let messages = sessions::load(&session).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].content.as_deref(),
            Some("what is the capital of france?")
        );
        assert_eq!(messages[1].content.as_deref(), Some("mock response"));
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
