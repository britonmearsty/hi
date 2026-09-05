# hi

`hi` is a local-first terminal AI assistant. Chat with OpenAI, Anthropic
Claude, Google Gemini, OpenRouter, or a local Ollama server, resume previous
conversations, and let the assistant work with files and run terminal commands
under an approval policy you control.

## Quick start

Requires Rust 1.85 or newer. CI currently checks Linux, macOS, and Windows.

### One-command installation

Linux x86_64 and macOS Apple Silicon users can install the latest release with:

```bash
curl -fsSL https://raw.githubusercontent.com/britonmearsty/hi/main/scripts/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/britonmearsty/hi/main/scripts/install.ps1 | iex
```

Or build from source:

```bash
git clone https://github.com/britonmearsty/hi.git
cd hi
cargo run
```

On the first run, `hi` opens a setup guide where you pick a provider, enter
the API key, endpoint, model, and approval mode. Or configure non-interactively
with environment variables (see below). Run `hi config` any time to reconfigure.
`hi doctor` verifies the setup and connectivity.

## Basic usage

```bash
hi                         # start a new session
hi chat                    # start a new session
hi chat --session abc123   # continue a session
hi resume abc123           # continue a session
hi sessions                # list sessions
hi delete abc123           # delete one session
hi delete --all            # delete every session
hi config                  # rerun setup
hi doctor                  # check configuration and connectivity
hi models                  # list provider models
hi "ask a single question" # answer without starting the REPL
```

Inside chat, use:

```text
/help [command]   show help (or explain one command)
/sessions         list saved sessions
/resume <id>      switch to another session
/clear            clear the terminal screen
/reset            forget this session's conversation history
/model [name]     show or switch the model
/config           show provider, URL, model and approval settings
/doctor           check whether the provider is reachable
/new              leave this session and start afresh
/quit /exit       leave hi
```

Slash commands are handled locally and are never sent to the AI. Press Tab
after typing `/` to autocomplete them. The prompt supports arrow-key history
and multiline input by ending a line with `\\`.

See [docs/USAGE.md](docs/USAGE.md) for the full workflow.

## Tools and approval

The assistant is given tools it can call to get real work done:

| Tool          | Purpose                                      | Risk      |
|---------------|----------------------------------------------|-----------|
| `run_command` | Run a terminal command (shell syntax opt-in) | assessed   |
| `read_file`   | Read a file, optionally in chunks            | safe      |
| `write_file`  | Create, overwrite, or append to a file       | caution   |
| `list_dir`    | List a directory's entries                   | safe      |
| `search`      | Regex search across a file or directory tree | safe      |
| `delete`      | Remove a file or directory                   | dangerous |

Before anything runs, `hi` shows the preview, risk level, and reason:

```text
⚙ AI run_command
  git status --short
✓ Safe Reason: inspect the current repository
  Allow this action? [y/N]
```

Commands involving `sudo`, destructive deletion, disk tools, or system power
operations are marked dangerous; anything needing a shell or touching files is
marked caution; everything else is safe.

Approval modes:

- `always` (default): ask before every action.
- `safe-only`: auto-approve safe actions, ask for the rest.
- `never`: approve everything per the policy; for controlled environments only.

Action results are limited to 20,000 characters with a 30-second timeout, and
likely secrets (API keys, passwords, tokens) are redacted from output. In
headless mode (`hi "prompt"`), actions follow the configured policy without
interactive prompts; anything not auto-approved is declined.

## Providers

| Provider   | Preset base URL                        | Default model         |
|------------|----------------------------------------|-----------------------|
| `openai`   | `https://api.openai.com/v1`            | `gpt-4o-mini`         |
| `anthropic`| `https://api.anthropic.com`            | `claude-sonnet-4-5`   |
| `gemini`   | `https://generativelanguage.googleapis.com` | `gemini-2.5-flash` |
| `openrouter`| `https://openrouter.ai/api/v1`        | `openrouter/auto`     |
| `ollama`   | `http://localhost:11434`               | `llama3.2`            |

Ollama is local and keyless: start it with `ollama serve` and `hi` will use it.
Set a custom base URL for any OpenAI-compatible gateway that speaks the Chat
Completions protocol (OpenRouter shares the same wire format as OpenAI).

### Environment variables

Configuration precedence is: environment variables, then the config file, then
provider defaults.

```bash
export HI_API_KEY="..."
export HI_BASE_URL="https://api.openai.com/v1"
export HI_MODEL="gpt-4o-mini"
export HI_APPROVAL_MODE="always"
```

API keys are mirrored to the OS credential store when available and also kept
in the local configuration file with `0600` permissions for reliable recovery.
They are not displayed by `hi doctor` or intentionally included in session
messages.

## Shell completions

```bash
hi completions bash > ~/.local/share/bash-completion/completions/hi
hi completions zsh > ~/.zfunc/_hi
hi completions fish > ~/.config/fish/completions/hi.fish
hi completions powershell > hi-completions.ps1
```

## Local data

```text
~/.config/hi/config.env       provider settings and protected key fallback
~/.local/share/hi/sessions.db SQLite session database
```

Back up sessions while `hi` is not running:

```bash
cp ~/.local/share/hi/sessions.db ~/hi-sessions-backup.db
```

## Troubleshooting

For `401 Unauthorized`, rerun setup and test connectivity:

```bash
hi config
hi doctor
```

For OpenAI-compatible gateways, verify the base URL includes the expected API
prefix (usually `/v1`) and check the model ID with `hi models`. For Ollama,
make sure the local server is running (`ollama serve`) — `hi doctor` and error
messages detect local endpoints and suggest this.

## Development

```bash
cargo fmt
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
```

See [docs/USAGE.md](docs/USAGE.md) for the usage guide, [hi.1](hi.1) for the
man page, [CHANGELOG.md](CHANGELOG.md) for release notes, and
[TODO.md](TODO.md) for the roadmap.

## License

MIT.