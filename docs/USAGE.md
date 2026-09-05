# hi usage guide

## Start and resume

Run `hi` or `hi chat` to start a new session. The session ID is printed when it
starts, and the first user message becomes its title.

```bash
hi sessions
hi resume 7fa8bcf8
```

Messages and tool results are stored locally as the conversation runs.
When a session grows beyond the working context, older messages are summarized
by the configured model and the summary is persisted with the session.

Answer a single question without entering the REPL:

```bash
hi "what is in the current directory?"
```

## Local slash commands

Slash commands are intercepted before provider calls. `/help`, `/model`, and
unknown commands such as `/helo` never become AI messages.

```text
/help [command]   show help (or explain one command)
/sessions         list saved sessions
/resume <id>      switch to another session
/clear            clear the terminal screen
/reset            forget this session's conversation history
/model [name]     show or switch the model
/config           show provider, URL, model, approval mode
/doctor           check whether the provider is reachable
/new              leave this session and start afresh
/quit /exit       leave hi
```

Use Tab completion after `/`. Arrow keys recall input from the current REPL
session. For multiline input, end a line with `\\`:

```text
❯ Explain this command \\
  and include its security implications
```

## Providers

Run `hi config` to change provider settings. Choose between OpenAI, Anthropic
Claude, Google Gemini, OpenRouter, and Ollama (local). Each ships with preset
base URLs and models listed by `hi models`; a custom base URL can point at any
OpenAI-compatible Chat Completions endpoint.

For Ollama, download a model first:

```bash
ollama pull llama3.2
ollama serve          # keep this running
hi config             # pick provider 5 (Ollama), no API key needed
```

Configuration precedence is environment variables, then the config file, then
provider presets:

```bash
export HI_API_KEY="..."
export HI_BASE_URL="..."
export HI_MODEL="..."
export HI_APPROVAL_MODE="always"   # always | safe-only | never
```

## Tools and approval

The assistant can call these tools; the application owns execution.

```text
run_command   Run a terminal command (shell syntax is opt-in)
read_file     Read a file, in chunks if it is large
write_file    Create, overwrite, or append to a file
list_dir      List a directory's entries
search        Regex search across a file or directory tree
delete        Delete a file or directory (recursive option)
```

Before anything runs, `hi` shows the exact preview, its risk, and why. The
risk level is derived per invocation: a benign single command is safe, shell
syntax or file writes are caution, and `sudo`, destructive deletion, disk tools,
or power operations are dangerous.

Approval policy:

- `always`: ask before every action (default).
- `safe-only`: automatically approve only safe actions.
- `never`: approve the recorded risk without asking; for controlled environments.

Every tool result has a 30-second timeout and a 20,000-character limit. Likely
secret assignments and authorization values are redacted from returned output.

In headless mode (`hi "prompt"`) tools follow the configured policy without
interactive prompts; anything that would need approval is declined and reported
so the model can adapt. Use `hi chat` when you want to approve actions
interactively.

## Markdown output

Responses support headings, emphasis, inline code, links, ordered and nested
lists, block quotes, horizontal rules, aligned pipe tables, and fenced code
blocks. Recognized fenced languages are syntax-highlighted in the terminal.
Tables use full borders in wide terminals and a stacked layout in narrow panes.

## Data and recovery

```text
~/.config/hi/config.env
~/.local/share/hi/sessions.db
```

The database schema is versioned with SQLite's `user_version` pragma. Back it
up while `hi` is stopped:

```bash
cp ~/.local/share/hi/sessions.db ~/hi-sessions-backup.db
```

Deleting a session removes its messages. `hi delete --all` is irreversible
unless a database backup exists.