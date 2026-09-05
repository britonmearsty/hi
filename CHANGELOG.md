# Changelog

## Unreleased

## v0.2.0 — plug-and-play providers and tools

- **Providers**: `hi` now supports OpenAI, Anthropic Claude, Google Gemini,
  OpenRouter, and local Ollama from a single `Provider` trait and factory.
  Each provider translates tool calls and results into its native wire format.
  Old configs migrate automatically: `openai-compatible` becomes `openai` and
  `local` becomes `ollama`.
- **Tools**: the assistant can now read (`read_file`, chunked), list
  (`list_dir`), write (`write_file`), delete (`delete`), and regex-search
  (`search`) files alongside `run_command`. Every tool reports a preview and
  risk and runs only under the configured approval policy.
- **Headless tool loop**: `hi "prompt"` now executes the tool loop with the
  configured approval policy instead of ignoring tool calls.
- Shelving boilerplate: removed tool-type names from the system prompt so the
  model relies on the tool schema, not prompt hints.
- Risk and previews: `run_command` gained a working-directory option; tool
  output keeps the 20,000-character cap, 30-second timeout, and secret
  redaction.

## v0.1.2 — release reliability

- Create the GitHub release before uploading binaries.
- Make installers resilient to GitHub API rate limits.

## v0.1.1 — summaries and installers

- Persisted conversation summarization with bounded context windows.
- One-command installation scripts for Unix and Windows.
- CI release builds on native Linux/macOS/Windows runners.
- Document supported Rust versions and platforms.

## v0.1.0 — initial release

- Terminal chat with an OpenAI-compatible provider.
- Streaming Markdown output with syntax highlighting.
- SQLite session memory, resume, titles, and deletion.
- Interactive `hi config` setup wizard and OS keychain storage.
- `hi doctor`, `hi models`, and shell completions.
- Approved `run_command` execution with risk levels, timeouts, and redaction.