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

## Local slash commands

Slash commands are intercepted before provider calls. `/help`, `/model`, and
unknown commands such as `/helo` never become AI messages.

```text
/help       display available commands
/sessions   list local sessions
/clear      remove non-system messages from this session
/model      display provider and model
/new        leave the current session
/quit       exit
```

Use Tab completion after `/`. Arrow keys recall input from the current REPL
session. For multiline input, end a line with `\\`:

```text
❯ Explain this command \\
  and include its security implications
```

## Setup and providers

Run `hi config` to change provider settings. OpenAI-compatible providers need
an API key, base URL, and model ID. Local endpoints may omit the key.

Use `hi models` to inspect models exposed by the endpoint. Some compatible
providers do not implement `/models`; enter the model ID manually in that case.

## Command policy

The assistant can request `run_command`, but the application owns execution.
Simple executable-plus-argument commands run directly. Shell syntax requires an
explicit shell request. The default approval mode is `always`:

- `always`: ask before every command.
- `safe-only`: automatically approve only safe commands.
- `never`: approve all commands; use only in controlled environments.

Every command has a 30-second timeout and a 20,000-character result limit.
Likely secret assignments and authorization values are redacted from returned
command output.

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
