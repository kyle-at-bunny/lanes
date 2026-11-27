# Architecture

Lanes is a desktop app that supervises ordinary `claude` CLI sessions. It never talks to the Claude API itself: Claude Code does all model work, and Lanes decides **which session runs where, with what context**.

## Design goals

- **Fresh context per stage.** Planning, implementation, review and QA each get their own session, so no stage inherits another's exploration.
- **Explicit hand-offs.** Stages communicate only through declared artifacts, a file or a short piece of text. Because a language model reads them, they need no schema.
- **Resumable stages.** A ticket returning to a stage resumes that stage's earlier session instead of starting over.
- **Real interactive sessions.** The user talks to each session directly; nothing is driven headlessly.
- **Plain, inspectable state.** TOML, markdown and JSON files that can be read, versioned and repaired by hand.

## Crates

```
crates/
  lanes-core   config, tickets, artifacts, prompts, session launch/resume, state, work item links, cleanup, CLI   (no UI)
                 agent/      the coding agent CLI behind AgentCli (claude.rs, opencode.rs)
                 platform/   OS-specific processes, opening files, terminals and path rules (Windows, Unix)
                 pr_status/  pull request status behind PrProvider (github.rs, none.rs)
  lanes-app    GPUI desktop app: board, terminal, tray, theme; builds the `lanes` binary
```

`lanes-core` holds all behaviour that can be unit-tested without a window. `lanes-app` renders it and owns processes.

Three interfaces keep Lanes independent of what it runs on and talks to: `AgentCli` for the session CLI (see [agents.md](agents.md)), `Platform` for the operating system, and `PrProvider` for pull request status.

## Session model

Each `(ticket, column)` pair maps to at most one agent session, stored in the ticket's `ticket.toml` with the agent that created it (and the agent's own id, when it differs from Lanes' UUID):

```toml
[sessions.dev]
id = "8f0c…"
cwd = 'C:\projects\my-app'
input_hashes = { plan = "3a9c…" }
agent = "claude"
```

The flags below are Claude Code's, from `agent/claude.rs`. Other agents get the same request in their own form; OpenCode, for example, has no system-prompt flag, so the stage prompt and brief become its first message.

- **First entry** to a column:
  1. Lanes generates a UUID and builds the *ticket brief*: title, work item link, description and the *Inputs from earlier stages*.
  2. It renders the column's prompt plus a generated *Stage outputs* section into `sessions/<column>.system.md`. With `start = "wait"` (the default), the brief goes in there too.
  3. It launches:

     ```
     claude --session-id <uuid> --append-system-prompt-file <system.md> --settings <per-session.json> [--model …]
     ```

     With `start = "wait"` there is no positional prompt: Claude opens on an empty input box with the ticket already in context, and you send the first message. With `start = "prompt"`, the brief is passed as the first message instead, and Claude starts working immediately. Either way the brief is fixed when the session is created.
- **Re-entry:** Lanes runs `claude --resume <uuid> --settings … [flags]`. It compares the current hashes of the column's inputs with `input_hashes` from the last entry, and prepares a re-entry note only if something changed. In wait mode with the embedded terminal, the note is typed into Claude's input box with no Enter once its UI has settled, so you can edit it before sending. Otherwise it is passed as the prompt.
- **Flags on every launch:** model, effort, permission mode, extra arguments and settings aren't persisted by Claude Code across `--resume`, so Lanes passes them every time. The appended system prompt *is* kept with the session, which is why a prompt edit affects only new sessions.
- **One process per session:** if a session's process is already running, opening it again focuses the existing terminal.

## Live state

Lanes doesn't scrape the terminal. For Claude Code, each session gets a `--settings` file containing:
- **Hooks** for `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Notification`, `Stop`, `StopFailure` and `SessionEnd`. Each runs `lanes hook`, which maps the event to *ready / working / needs input / your turn / exited* and writes `state/<uuid>.hook.json`.
- **A status line** that runs `lanes statusline`. It records the context-window percentage and the plan's 5-hour and 7-day limits to `state/<uuid>.status.json`, then prints a status line (or chains to the user's own).

The app polls these small files once a second to update badges, context meters, the usage bar, the tray badge and notifications. Because state comes from the agent itself, it works the same in embedded and external-terminal modes. Agents without hooks (OpenCode today) have no live state: their cards show "running".

## Embedded terminal

- **Process:** each session runs in a pseudo-terminal via `portable-pty` (ConPTY on Windows).
- **Emulation:** output feeds an `alacritty_terminal` grid. A custom GPUI element paints the grid as background quads and shaped text runs, roughly every frame when something changed.
- **Input:** keystrokes are encoded to xterm sequences, honouring application-cursor mode and bracketed paste. Files dropped from Explorer are pasted as their paths.
- **Resizing:** follows the element's size.
- **When `claude` exits:** the pseudo-console is closed so its reader thread finishes.

## Hot reload and the board assistant

The app watches `board.toml`'s modification time. When it changes, the file is parsed and validated. A valid file replaces the active config; an invalid one is reported and ignored.

The board assistant is a regular Claude session with:
- a prompt in two parts:
  - the user's editable `prompts/assistant.md` (role and house rules);
  - a built-in reference of the CLI, the config format and how to import work items with tracker tools, compiled into Lanes so it always matches the installed version;
- the board folder as its working directory;
- `--allowedTools "Bash(<lanes> --home <board>:*)"`, so it can run the CLI without prompting. Its file edits still go through Claude Code's normal approval.

## Session housekeeping

Lanes never deletes Claude Code's data, which lives in `~/.claude`, expires under Claude Code's own `cleanupPeriodDays` retention, and may be shared with sessions you ran outside Lanes. Housekeeping only changes Lanes' side:

- **The inventory** (`lanes-core/src/sessions.rs`) gathers every session Lanes created. It combines ticket session records (on the board and archived), the assistant's `assistant.toml`, and the per-session settings files in `state/`, which Lanes writes for every launch. A session found only through a settings file is *unlinked*.
- **Forgetting** a session removes its `state/` files and its link from the ticket or `assistant.toml`. The next start in that stage creates a new session.
- **Deleting a ticket** stops its sessions, then removes the ticket folder and its sessions' state files.

## Window lifecycle

The app owns the session processes:
- **Closing the window** hides it to the notification area. The process, and every session, keeps running.
- **Quitting** terminates every session; each one remains resumable by ID. It asks first only when a session is mid-turn: its hooks report "working" *and* its terminal produced output in the last 10 seconds (Claude's spinner redraws continuously while it works). The output check filters out a stale "working" left by an interrupted turn.
