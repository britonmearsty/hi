# hi development TODO

## Milestone 0 — project foundation

- [x] Create Rust binary crate and module boundaries.
- [x] Add initial CLI subcommands.
- [x] Add README and this development checklist.
- [x] Add formatting, linting, and CI configuration.
- [x] Let's style everything up: colorize, make the UI/UX pretty and easy to use, improve titles, improve the prompt instead of `hi>`, use an icon, add a loader, and make AI-run commands clearly distinguishable.
- [x] Set minimum Rust version to 1.85 and document Linux, macOS, and Windows support.

## Milestone 1 — usable chat

- [x] Define provider-neutral request/response/message types.
- [x] Implement OpenAI-compatible HTTP provider.
- [x] Support `HI_API_KEY`, `HI_BASE_URL`, and `HI_MODEL`.
- [x] Add streaming assistant output.
- [x] Add friendly authentication, timeout, rate-limit, and network errors.
- [x] Add readline input, in-session history, multiline input, and slash-command Tab completion.

## Milestone 2 — session memory

- [x] Add SQLite database with migrations.
- [x] Store sessions, messages, tool calls, and tool results.
- [x] Generate readable session titles.
- [x] Implement `sessions`, `resume`, and `delete`.
- [x] Add `/clear` and `/model`; `/new` and `/resume` remain CLI workflows.
- [x] Add persisted AI-generated conversation summaries and bounded context windows.

## Milestone 3 — approved command execution

- [x] Define the `run_command` tool schema.
- [x] Implement the model/tool-call loop.
- [x] Show the exact command before execution.
- [x] Require explicit approval for every command by default.
- [x] Capture stdout, stderr, and exit status.
- [x] Add timeout and output-size limits.
- [x] Avoid shell execution by default; make shell mode explicit.
- [x] Add warnings for destructive or privileged commands.
- [x] Redact likely secrets from displayed and stored output.

## Milestone 4 — setup and polish

- [x] Build an interactive `hi config` setup wizard.
- [x] Add OS keychain support for credentials.
- [x] Implement `hi doctor` connectivity checks.
- [x] Add Markdown and syntax-highlighted code-block rendering.
- [x] Render Markdown pipe tables with aligned columns.
- [x] Add shell completions and man/help documentation.
- [x] Add configurable approval modes, keeping `always` as default.
- [x] Add tag-based release packaging for Linux, macOS, and Windows.

## Milestone 5 — quality and security

- [x] Unit-test renderer, risk assessment, and secret redaction.
- [x] Test provider stream parsing and session persistence mechanics.
- [x] Test provider HTTP error classification.
- [x] Test public session persistence, resume, and deletion flows.
- [x] Test command execution quoting, shell mode, timeout, and truncation.
- [x] Test pipes, redirects, Unicode, and multiline commands end-to-end.
- [x] Verify rejected commands cannot reach the executor.
- [x] Add mock-provider integration tests.
- [x] Audit command output and error messages for common credential leakage.
- [x] Add database migration and backup guidance.
