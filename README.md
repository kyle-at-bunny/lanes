<h1 align="center">Lanes</h1>

<p align="center">
  <strong>A kanban board for real, interactive Claude Code sessions.</strong><br>
  Each ticket gets its own Claude session per stage: plan, dev, review, QA. Stages hand work to each other through small, explicit artifacts, never through a shared transcript.
</p>

<p align="center">
  <a href="https://github.com/kyle-at-bunny/lanes/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/kyle-at-bunny/lanes/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Platform: Windows" src="https://img.shields.io/badge/platform-Windows-0078D6.svg">
  <img alt="Built with Rust and GPUI" src="https://img.shields.io/badge/built%20with-Rust%20%2B%20GPUI-orange.svg">
</p>

<p align="center">
  <img src="docs/images/board.png" alt="The Lanes board: tickets across Backlog, Plan, Dev, Review, QA and Done columns, each showing its Claude session's state and context usage" width="900">
</p>

## Why Lanes?

Long Claude Code conversations drift. A session that planned a feature carries all of that exploration into implementation, then into review, filling its context and blurring its judgement. Lanes gives every stage of a ticket its own focused session:

- **Planning** sees the ticket and the codebase.
- **Dev** sees the agreed `plan.md`, not the planning conversation.
- **Review** sees the plan and the branch, and reviews it with fresh eyes.

When a ticket moves *back* (review found problems, say), Lanes **resumes the earlier session for that stage** instead of starting over, and tells it what changed.

Lanes is a task manager, not an autopilot. You decide when a ticket moves. Every session is the real `claude` CLI running in an embedded terminal, so you can talk to it, approve tools, and use slash commands exactly as you would in your own terminal.

## Features

- **One session per ticket per stage.** Each column has its own prompt template, working directory, model and permission mode. Sessions resume by ID when a ticket comes back. **Request handoff** asks a running session to write its stage outputs, and **Suspend** stops it so you can resume it later.
- **Explicit hand-offs.** Columns declare the artifacts they `consume` and `produce`: a markdown file, a short piece of text, or an HTML page (mockups, reports) that opens in your browser. Each new session gets the ticket brief and its inputs as context and waits for your first message (or starts straight away with `start = "prompt"`). Lanes warns you if you move a ticket on before its outputs exist.
- **Artifacts tab.** A stage's inputs and outputs, who wrote each one and when, markdown rendering, inline editing of text artifacts, clickable PR links and repo folders, and PR state, CI results, review decision and unresolved comments via `gh`, shown next to each link and under the ticket header (with a card badge like "1 PR · 1 open · changes requested · 2 unresolved"). Collapse a long artifact such as the plan to its header with ▾; Lanes remembers which artifacts you keep collapsed.
- **Embedded terminal.** Each `claude` session runs in a GPU-rendered terminal backed by a pseudo-terminal (ConPTY on Windows) and `alacritty_terminal`, with selection, scrollback and bracketed paste. If you'd rather use an external terminal (Windows Terminal tabs on Windows), set `terminal = "external"`.
- **Live status at a glance.** Cards show whether a session is working, needs input or is waiting on you, plus its context-window usage. The top bar tracks your 5-hour and weekly plan limits.
- **Board assistant.** A dedicated Claude session that can operate the board for you: create and move tickets, add a column, rewrite a stage's prompt. Config changes hot-reload.
- **Works with any tracker.** Link a ticket to its Jira, Azure DevOps, GitHub, GitLab or Linear work item (or anything with a URL). The key appears as a link on the card and the URL is passed to Claude. To import work items, give the board assistant your tracker's MCP server and ask it. Lanes holds no tracker credentials.
- **Editable tickets.** The ticket panel shows the full description, scrolling when it's long, with **Edit description** to change it in place.
- **Resizable ticket panel.** Drag the divider, or switch between a wide and a narrow panel. Drag the line under the ticket details to give the session more height.
- **Token spend.** `lanes spend` and the ticket panel show what each ticket and stage cost, read from Claude Code's own transcripts. Optional lean mode keeps stage sessions small with a context limit and a **Hand off to a fresh session** button, and an A/B mode measures whether that helps. See [`[spend]`](docs/configuration.md#spend).
- **Timeline and transcripts.** See a ticket's moves, sessions, artifact writes and actions, and read any stage's conversation without reviving it.
- **Flexible working folders.** A column's folder can be a template, e.g. one git worktree per ticket. Any ticket can override it for all stages or just one.
- **Action buttons.** Templated one-click commands per column or board-wide, such as "Open in VS Code" or "create worktree".
- **Finish & clean up.** Stop a finished ticket's sessions, run cleanup commands (e.g. `git worktree remove`) and archive the ticket, with a confirmation step and restore.
- **Session housekeeping.** The **Sessions** view lists every Claude session Lanes has created, which ticket and stage it belongs to, and whether it's running. You can stop or forget any of them, and delete tickets outright.
- **Lives in the tray.** Closing the window keeps sessions running, and the tray icon shows when one needs you.
- **Plain files.** The board, prompts, tickets and artifacts are TOML and markdown under `~/.lanes`, easy to inspect, version or back up.

<table>
  <tr>
    <td><img src="docs/images/ticket-artifacts.png" alt="Ticket panel's Artifacts tab showing the Dev stage's rendered plan input and its optional inputs"></td>
    <td><img src="docs/images/ticket-timeline.png" alt="Ticket panel with the work item link, actions and the timeline of moves, sessions and artifact writes"></td>
  </tr>
  <tr>
    <td align="center"><sub>Artifacts tab: a stage's inputs and outputs</sub></td>
    <td align="center"><sub>Ticket panel: actions and timeline</sub></td>
  </tr>
  <tr>
    <td colspan="2"><img src="docs/images/ticket-transcript.png" alt="Ticket panel in light mode reading the Dev stage's Claude transcript"></td>
  </tr>
  <tr>
    <td colspan="2" align="center"><sub>Reading a stage's transcript (light theme)</sub></td>
  </tr>
</table>

## Quick start

> **Status: early (0.x).** Lanes is used daily on Windows by its author, but the board format and features may still change between releases.

### Requirements

- **Windows 10 or 11.** Lanes is developed and tested on Windows. macOS and Linux are untested; see [Platform support](#platform-support).
- **[Claude Code](https://docs.claude.com/en/docs/claude-code)**, installed and logged in. Run `claude`, then `/login`, once in a normal terminal. [OpenCode](https://opencode.ai) works too (`agent = "opencode"`), with fewer live details; see [Agents](docs/configuration.md#agents).
- **Optional:** the [GitHub CLI](https://cli.github.com) (`gh`), logged in, for pull request state, CI, reviews and unresolved comments on PR links. Set `pr_provider = "none"` to turn PR checks off.
- **Building from source** additionally needs:
  - [Rust](https://rustup.rs) (stable, MSVC toolchain);
  - the Visual Studio C++ build tools with a Windows 10/11 SDK (the "Desktop development with C++" workload).

### Download or build

Download `lanes.exe` from the [latest release](https://github.com/kyle-at-bunny/lanes/releases/latest) and run it. It isn't code-signed, so Windows SmartScreen may warn the first time; choose **More info → Run anyway**. Or build it yourself:

```powershell
git clone https://github.com/kyle-at-bunny/lanes.git
cd lanes
cargo build --release -p lanes-app
.\target\release\lanes.exe
```

### First steps

1. **Point the columns at your code.** On first run Lanes creates a starter board in `~/.lanes/board.toml`. Click **Edit board.toml**, set each stage's `cwd` to the folder its sessions should run in, and save. The board reloads automatically.
2. **Create a ticket.** Click **New ticket**, and describe the work. (Or ask the board assistant to import work items from your tracker.)
3. **Start working.** Drag the card to **Plan / Arch**, open it, and click **Start session**. A Claude session starts in that column's folder with the planning prompt and your ticket. Work with it until `plan.md` is written.
4. **Move it on.** Drag the card to **Dev** and click **Start session** again. A *new* session starts, seeded with the plan. Move the card back to Plan later and **Resume session** brings back the original planning conversation.

Moving a card never starts a session by itself. Sessions only start or resume when you click the button on the ticket.

## How it works

```
Backlog ──► Plan / Arch ──► Dev ──► Review ──► QA ──► Done
               │  plan.md   │ branch │ review_notes
               └────────────►└───────►└──► (back to Dev resumes the Dev session)
```

- **Starting a stage** (opening the ticket and clicking **Start session**) runs `claude --session-id <uuid>` in the column's `cwd`, with the column's prompt and the ticket brief appended to the system prompt. Claude waits for you to set the first task. Lanes remembers the session ID per (ticket, column).
- **Coming back** to a column and clicking **Resume session** runs `claude --resume <uuid>`. If any of the column's inputs changed since that session last ran, Lanes types a short re-entry note listing them into the input box, for you to edit and send.
- **Sessions write their outputs** to the paths Lanes gives them (file artifacts) or with `lanes artifact set` (text artifacts). Later stages only ever see those artifacts.
- **Status comes from Claude Code** through per-session hooks and a status line, injected with `--settings`, so your own Claude settings are untouched.

See [docs/architecture.md](docs/architecture.md) for the details.

## Your workflow, your columns

Plan → Dev → Review → QA is only the starter board. The columns are whatever you write in `board.toml`, so the board can follow how you already work:

- **Any stages, in any order.** Rename, add, remove or reorder columns, and run as many or as few as you like. A column with a `cwd` gets a Claude session per ticket; one without (like Backlog or Done) is a plain holding lane.
- **Your own hand-offs.** Define any artifacts you want stages to pass along, e.g. `spec`, `security_notes` or `release_notes`, and say which columns produce and consume them.
- **Per-stage behaviour.** Each column sets its own prompt, folder, model, effort, permission mode, whether Claude starts straight away, and its own action buttons and cleanup command.

For example, a lighter board for small changes:

```toml
[artifacts.spec]
type = "file"
description = "What to build and how we'll know it works."

[[columns]]
id = "inbox"
title = "Inbox"

[[columns]]
id = "spec"
title = "Spec"
cwd = "~/projects/my-app"
prompt = "prompts/spec.md"
produces = ["spec"]
permission_mode = "plan"

[[columns]]
id = "build"
title = "Build & ship"
cwd = "~/worktrees/{{ ticket.id | lower }}"
prompt = "prompts/build.md"
consumes = ["spec"]
model = "sonnet"

[[columns]]
id = "shipped"
title = "Shipped"
```

You can also ask the [board assistant](#board-assistant) to make these changes for you, and the board reloads as soon as `board.toml` is saved.

## Configuration

Everything lives in `~/.lanes/board.toml` (or `--home <dir>` / `$LANES_HOME`). A column looks like this:

```toml
[artifacts.plan]
type = "file"
inline = true
description = "Implementation plan: goal, approach, files to change, risks, and how to verify."

[[columns]]
id = "dev"
title = "Dev"
cwd = "~/projects/my-app"
prompt = "prompts/dev.md"
consumes = ["plan", "review_notes?"]   # "?" marks an optional input
produces = ["branch"]
model = "opus"
```

See [docs/configuration.md](docs/configuration.md) for every option, including:
- prompt template variables;
- action buttons and cleanup commands;
- work item links, themes and terminal settings.

Run `lanes config schema --write` once for completion and validation in editors that read `#:schema` (such as VS Code with Even Better TOML). New boards get it automatically, and `lanes config docs` prints the reference for your installed version.

## Board assistant

Click **Assistant** to open a Claude session that runs in your board folder and knows how Lanes works. Ask it things like:

> *"Create tickets for the three bugs in this list and put the login one straight into Plan."*
> *"Add a Security review column between Review and QA that consumes the branch and produces security_notes."*
> *"Make the Dev prompt insist on writing tests before code."*

It runs the [`lanes` CLI](docs/cli.md) (pre-approved) and edits `board.toml` and prompts. Those edits still ask for your approval, and invalid configs are rejected without disturbing the running board.

The assistant keeps one conversation and resumes it each time you open it. **Reset** (in its panel) drops that conversation so the next one starts fresh.

## Cleaning up sessions

Lanes never deletes Claude Code's own data. Claude Code keeps every conversation on disk and removes old ones itself after `cleanupPeriodDays` (30 days by default, set in `~/.claude/settings.json`). What Lanes manages is its *link* to each session:

- **Delete** on a ticket (on the board or in the archive) stops its sessions and permanently removes the ticket, its artifacts, its timeline and Lanes' records of its sessions.
- **Sessions** in the top bar lists every Claude session Lanes has created:
  - which ticket and stage each belongs to (or the assistant);
  - its folder, when it was last active, and whether it's running.
- **Stop** ends a running session; you can resume it later.
- **Forget** stops a session and unlinks it. For a ticket, that stage starts a fresh conversation next time.
- **Forget all unlinked** tidies up sessions that no ticket or the assistant refers to any more.

## Using the app

| | |
| --- | --- |
| **Close (✕)** | Hides to the system tray; sessions keep running. Click the tray icon to come back. |
| **Quit** | In the top bar or tray menu. Asks first only if Claude is mid-turn in a session; idle or waiting sessions are just stopped. Either way, sessions can be resumed later. |
| **Ctrl+Shift+C** / mouse selection | Copy from the terminal |
| **Ctrl+V** | Paste text. When the clipboard has no text (an image, say), it passes through to Claude. |
| **Drop files on the terminal** | Types their paths into Claude's input box, quoted when they contain spaces, as Windows Terminal does. |
| **Shift+PgUp / PgDn**, wheel | Scrollback. In full-screen apps such as Claude Code, the wheel sends PgUp / PgDn to the app. |
| **Theme button** | Cycles Auto (follows Windows) → Light → Dark |
| **Panel divider** | Drag the edge between the board and the ticket/assistant panel to resize it, or double-click it to reset to 50%. **Wider** / **Narrower** in the panel header switches between a 70% and a 35% panel. The width is remembered (in `~/.lanes/ui.toml`). |
| **Click the board** | Clicking an empty part of the board (anywhere but a card or a form) closes the open ticket or the assistant. Sessions keep running. |
| **Ticket info divider** | Drag the line between a ticket's details and its Session / Timeline / Transcript / Artifacts tabs to give the session more or less height; the details scroll when they no longer fit. Double-click it to fit the details to their content again. The height is remembered in `ui.toml` too. |

## Platform support

| Platform | Status |
| --- | --- |
| Windows 10/11 | Developed and tested daily |
| macOS, Linux | Builds (checked in CI) but untested. Everything OS-specific sits behind a platform interface (`lanes-core/src/platform`, `lanes-app/src/platform`) with best-effort macOS and Linux implementations. There is no tray on Linux, so closing the window quits. Contributions welcome. |

## FAQ

**Does Lanes call the Claude API directly?**
No. Every session is the `claude` CLI you already use, running in a pseudo-terminal, and usage counts against your normal Claude plan.

**What happens to running sessions if Lanes crashes?**
They stop, but each is resumable by ID from its card, and the conversation is intact. Only the turn in progress is interrupted.

**Can a stage see what earlier stages discussed?**
Only through artifacts. That's the point. The **Transcript** tab lets *you* read any stage's conversation.

**Where is my data?**
In `~/.lanes` (board, prompts, tickets, artifacts, Lanes state) and in Claude Code's own `~/.claude/projects` (session transcripts). Nothing is sent anywhere except by Claude Code itself (including any MCP servers you give it).

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for build, test and style notes.

## Acknowledgements

- [herdr](https://github.com/ogulcancelik/herdr), whose approach to supervising agent terminals inspired this project.
- [GPUI](https://github.com/zed-industries/zed) and [GPUI Kit](https://github.com/longbridge/gpui-kit) for the UI.
- [`alacritty_terminal`](https://github.com/alacritty/alacritty) for terminal emulation, and [`portable-pty`](https://github.com/wezterm/wezterm) for ConPTY.

Lanes is an independent project and is not affiliated with or endorsed by Anthropic.

## License

[MIT](LICENSE)
