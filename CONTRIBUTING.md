# Contributing to Lanes

Thanks for your interest! Bug reports, ideas and pull requests are all welcome.

## Building

You need stable Rust (MSVC toolchain on Windows) and the Visual Studio C++ build tools with a Windows SDK.

```powershell
cargo build -p lanes-app          # debug build: target\debug\lanes.exe
cargo test --workspace            # unit tests for both crates
```

Run against a throwaway board so you don't touch your real one:

```powershell
.\target\debug\lanes.exe --home C:\temp\lanes-dev
```

## Project layout

| Path | What it is |
| --- | --- |
| `crates/lanes-core` | Everything that isn't UI: board config, tickets, artifacts, prompt building, session launch/resume, hook and status-line state, Jira, cleanup, and the `lanes` CLI. Fully unit-tested, with no GPUI dependency. |
| `crates/lanes-core/assets` | The starter `board.toml` and default prompts written on first run. |
| `crates/lanes-app` | The GPUI desktop app: board view, embedded ConPTY terminal, tray, theme. Builds the `lanes` binary, which also serves the CLI. |
| `docs/` | User and architecture docs, plus README screenshots. |

## Guidelines

- **Keep logic in `lanes-core`** with tests; keep `lanes-app` to rendering and wiring.
- **Structure code readably.** Entry points read as a short sequence of well-named calls, and each function can be understood on its own.
- **Comments** are one line, and only when the *why* isn't obvious from the code.
- **Before opening a PR,** run `cargo test --workspace` and make sure `cargo build` has no warnings.
- **For UI changes,** include a screenshot in the PR description.

## Reporting bugs

Please include:
- your OS version;
- your Claude Code version (`claude --version`);
- what you expected and what happened;
- the relevant part of your `board.toml` if config is involved.

Never paste API tokens.
