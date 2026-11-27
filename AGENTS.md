# AGENTS.md

Guidance for AI coding agents (and humans) working on Lanes. Read [README.md](README.md) for what the app does and [docs/architecture.md](docs/architecture.md) for how it works.

## Commands

```powershell
cargo build -p lanes-app                 # debug build: target\debug\lanes.exe (app + CLI)
cargo test --workspace                   # all unit tests; must pass before any commit
cargo test -p lanes-core <filter>        # e.g. `cargo test -p lanes-core jira`
cargo fmt; cargo clippy --workspace --all-targets -- -D warnings   # CI fails on either
.\target\debug\lanes.exe --home C:\temp\lanes-dev   # run against a throwaway board, never ~/.lanes
```

- **Toolchain:** stable Rust, MSVC toolchain, and the Visual Studio C++ build tools with a Windows SDK.
- **Locked exe:** a running `lanes.exe` locks the binary, so close it (or Quit from the tray) before rebuilding.
- **Warnings:** builds must be warning-free.

## Layout

| Path | Contents |
| --- | --- |
| `crates/lanes-core/src` | All non-UI behaviour, unit-tested: `config`, `ticket`, `artifacts`, `prompt`, `session` (launch/resume), `agent` (the agent CLI behind `AgentCli`; see `docs/agents.md`), `platform` (OS-specific behaviour), `pr_status` (PR providers), `state` (session state files), `actions`, `timeline`, `transcript`, `cleanup`, `link` (work item links), `assistant`, `config_schema` (`lanes config schema/docs`), `ui_prefs`, `cli` + `board_cli` |
| `crates/lanes-core/assets` | Starter `board.toml` and default prompts, embedded with `include_str!` and written on first run |
| `crates/lanes-app/src` | GPUI app. `board_view.rs` is the main view; its child modules in `board_view/` hold feature areas, e.g. `archive`, `artifacts`, `assistant`, `description`, `jira_import`, `lifecycle` (tray, quit), `split` (panel divider) |
| `crates/lanes-app/src/{pty,terminal_view,keys,palette}.rs` | Embedded terminal: ConPTY process, grid rendering, key encoding, colours |
| `docs/` | User and architecture docs; `docs/images/` holds README screenshots |

## Rules of the codebase

- **Behaviour goes in `lanes-core` with tests.** `lanes-app` only renders state and wires events to core calls. If you need logic in the app, ask whether it can move to core.
- **State is plain files** under the board folder. Write with `paths::write_atomic`. Append-only logs (`events.jsonl`) let the CLI and GUI write concurrently. Never make the app the only writer of something the CLI also touches.
- **Sessions are the real `claude` CLI.** Don't add headless or API-driven flows. Flags Claude Code doesn't persist across `--resume` (model, effort, permission mode, `--settings`, extra args) must be passed on every launch; see `session.rs`.
- **Hooks and the status line must never fail loudly.** `lanes hook` / `lanes statusline` swallow errors, because a failing hook can block Claude Code.
- **Config changes must stay valid.** New `board.toml` keys need serde defaults, so existing boards keep loading. Document every new option in `docs/configuration.md` and, where useful, in the starter `assets/board.toml`.
- **Secrets never touch disk.** Tokens come from environment variables (see `[jira] token_env`). Don't log or persist them.

## Code style

- **Orchestrators read as a sequence of well-named calls.** Each function should be reviewable on its own; split or rename one that needs surrounding context to understand.
- **Comments:** at most one line, and only for a non-obvious *why* (a hidden constraint or a workaround). Don't restate the code.
- **Keep files focused.** When `board_view.rs` grows, add a child module under `board_view/`. Children can use the parent's private fields.
- **Tests sit next to the code** (`#[cfg(test)] mod tests`). Use `tempfile` for board folders, and the local `TcpListener` mock in `jira.rs` for HTTP.

## GPUI gotchas

GPUI comes through the `gpui-kit` facade, which pins a `gpui-pre` snapshot. Its API changes between versions, so check signatures in `~/.cargo/registry/src/*/gpui-pre-*` rather than guessing.

- **Imports:**
  - `main.rs` has `extern crate gpui_kit as gpui;`, which GPUI's macros need.
  - Import `gpui_kit::*` and `gpui_kit::prelude::FluentBuilder` (for `.when` / `.when_some`).
  - Import component items by name. `gpui_kit::component::*` collides with GPUI's `Size` and `Edges`.
- **Borrows:**
  - `cx.theme()` borrows `cx`. Clone it (`cx.theme().clone()`) or copy the colours you need before calling `&mut self` helpers that take `cx`.
  - To focus another entity, get its handle first and then focus: `let h = entity.read(cx).focus_handle(cx); h.focus(window, cx);`.
- **Method shadowing:** `Window` has an inherent `window_handle()` that shadows the raw-window-handle trait method. Call `HasWindowHandle::window_handle(window)` explicitly.
- **Key bindings:** the Root context binds Tab, Shift+Tab and Ctrl+C. The terminal unbinds them with `NoAction` in its `"Terminal"` key context (`main.rs`); add new pass-through keys there.
- **Background work:** use `cx.background_executor().spawn(...)` for blocking work (HTTP, processes), then `this.update_in(cx, ...)` to apply results and show notifications.
- **Layout:** size regions as a share of the window with a pixel minimum (`flex_1().min_w(px(..))`, `w(relative(..)).min_w(px(..))`), never fixed widths. Rows of buttons and banners should `flex_wrap()`.

## Platform notes

Everything OS-specific goes through `lanes_core::platform::current()` (processes, opening files, external terminals, path rules) or `lanes-app/src/platform` (console, window hiding, terminal font, clipboard keys). Don't add `cfg(windows)` elsewhere. Windows is the tested platform; macOS and Linux are compile-checked in CI only.

- **Program lookup:** `portable-pty` picks an extensionless npm shim over `claude.exe`, so `Platform::resolve_program` resolves the `.exe` explicitly. `.cmd` shims are never used, because cmd.exe cuts arguments off at the first newline; an agent with no `.exe` on PATH (OpenCode from npm) needs `[agents.<kind>] path`, and `BoardConfig::agent_program` says so.
- **ConPTY doesn't EOF when the child exits.** The exit watcher drops the master so the reader thread ends.
- **Nested Claude sessions:** when Lanes itself runs inside a Claude Code session, the inherited `CLAUDECODE` / `CLAUDE_CODE_*` variables make child sessions misbehave. `launch::inherited_session_vars` strips them (each agent lists its own in `AgentCli::leaked_env_vars`); keep it in every spawn path.
- **Console subcommands:** the binary uses the Windows GUI subsystem, so CLI subcommands call `AttachConsole` to print output.

## Verifying UI changes

Unit tests don't cover rendering. For UI work:
1. Build, then run against a scratch board (`--home`).
2. Exercise the feature.
3. Look at the result.

Use `terminal = "external"` plus hand-written `state/<uuid>.hook.json` and `state/<uuid>.status.json` files to show session states without a logged-in Claude CLI.

Screenshots for docs must use demo data only: no real paths, usernames, tickets or company names. Capture exactly the window frame (DWM extended frame bounds) so nothing from behind the window leaks in.

## Commits and PRs

- **Before committing:** run `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` and a warning-free build.
- **Releases:** push a `v<version>` tag matching `Cargo.toml`; `.github/workflows/release.yml` builds and attaches `lanes.exe`.
- **Keep the docs in step.** Update `README.md` and `docs/` in the same change as the feature.
- **Never commit** boards, `state/` files, tokens, or anything from a real `~/.lanes`.
