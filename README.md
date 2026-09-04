# hi

`hi` is a local-first terminal AI assistant. Chat with an OpenAI-compatible
model, resume previous conversations, and ask it to run terminal commands with
an approval policy you control.

## Quick start

```bash
git clone <repository-url>
cd hi
cargo run
```

On the first run, `hi` opens a setup guide for provider, API key, endpoint,
model, and command approval mode.

Build an optimized binary with:

```bash
cargo build --release
./target/release/hi
```

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
```

Inside chat, use:

```text
/help       show local commands
/sessions   list saved sessions
/clear      clear the current session's conversation
/model      show the active provider and model
/new        leave the current session
/quit       exit
```

Slash commands are handled locally and are never sent to the AI. Press Tab
after typing `/` to autocomplete them. The prompt supports arrow-key history
and multiline input by ending a line with `\\`.

See [docs/USAGE.md](docs/USAGE.md) for the full workflow.

## AI command execution

The assistant may propose a command through the `run_command` tool. `hi` shows
the exact command, its reason, shell mode, and risk level before executing it.

Default behavior is to ask for approval every time:

```text
⚙ AI command
  git status --short
✓ Safe Reason: inspect the current repository
  Allow this command? [y/N]
```

Simple commands run without a shell. Shell syntax requires explicit shell mode.
Commands involving `sudo`, destructive deletion, disk tools, or system power
operations are marked dangerous.

Approval modes are `always` (default), `safe-only`, and `never` (unsafe).
Command output is returned as only useful output or error, limited to 20,000
characters, with a 30-second timeout.

## Providers and environment variables

`hi` supports OpenAI and providers exposing an OpenAI-compatible chat
completions endpoint. Configuration precedence is:

```text
environment variables > config file > defaults
```

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

For compatible providers, verify the base URL includes the expected API prefix,
usually `/v1`, and check the model ID with `hi models`.

## Development

```bash
cargo fmt
cargo check --locked
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
```

See [hi.1](hi.1) for the man page and [TODO.md](TODO.md) for the roadmap.

## License

MIT.
