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
  lanes-core   config, tickets, artifacts, prompts, session launch/resume, state, Jira, cleanup, CLI   (no UI)
  lanes-app    GPUI desktop app: board, terminal, tray, theme; builds the `lanes` binary
```

`lanes-core` holds all behaviour that can be unit-tested without a window. `lanes-app` renders it and owns processes.

## Session model

Each `(ticket, column)` pair maps to at most one Claude Code session ID, stored in the ticket's `ticket.toml`:

```toml
[sessions.dev]
id = "8f0c…"
cwd = 'C:\projects\my-app'
input_hashes = { plan = "3a9c…" }
```

- **First entry** to a column:
  1. Lanes generates a UUID and renders the column's prompt plus a generated *Stage outputs* section into `sessions/<column>.system.md`.
  2. It builds the first prompt: the ticket plus its *Inputs from earlier stages*.
  3. It launches:

     ```
     claude --session-id <uuid> --append-system-prompt-file <system.md> --settings <per-session.json> [--model …] "<first prompt>"
     ```
- **Re-entry:** Lanes runs `claude --resume <uuid> --settings … [flags]`. It compares the current hashes of the column's inputs with `input_hashes` from the last entry, and sends a re-entry message only if something changed.
- **Flags on every launch:** model, effort, permission mode, extra arguments and settings aren't persisted by Claude Code across `--resume`, so Lanes passes them every time. The appended system prompt *is* kept with the session, which is why a prompt edit affects only new sessions.
- **One process per session:** if a session's process is already running, opening it again focuses the existing terminal.

## Live state

Lanes doesn't scrape the terminal. Each session gets a `--settings` file containing:
- **Hooks** for `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Notification`, `Stop`, `StopFailure` and `SessionEnd`. Each runs `lanes hook`, which maps the event to *ready / working / needs input / your turn / exited* and writes `state/<uuid>.hook.json`.
- **A status line** that runs `lanes statusline`. It records the context-window percentage and the plan's 5-hour and 7-day limits to `state/<uuid>.status.json`, then prints a status line (or chains to the user's own).

The app polls these small files once a second to update badges, context meters, the usage bar, the tray badge and notifications. Because state comes from Claude Code itself, it works the same in embedded and external-terminal modes.

## Embedded terminal

- **Process:** each session runs in a Windows pseudo-console (ConPTY via `portable-pty`).
- **Emulation:** output feeds an `alacritty_terminal` grid. A custom GPUI element paints the grid as background quads and shaped text runs, roughly every frame when something changed.
- **Input:** keystrokes are encoded to xterm sequences, honouring application-cursor mode and bracketed paste.
- **Resizing:** follows the element's size.
- **When `claude` exits:** the pseudo-console is closed so its reader thread finishes.

## Hot reload and the board assistant

The app watches `board.toml`'s modification time. When it changes, the file is parsed and validated. A valid file replaces the active config; an invalid one is reported and ignored.

The board assistant is a regular Claude session with:
- a prompt in two parts:
  - the user's editable `prompts/assistant.md` (role and house rules);
  - a built-in reference of the CLI, the config format and the Jira workflow, compiled into Lanes so it always matches the installed version;
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
- **Quitting** asks for confirmation when sessions are running, then terminates them. Each one remains resumable by ID.
