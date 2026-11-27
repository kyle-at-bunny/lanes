# Architecture sketch

> A proposal, not a decision.

This sketch follows the worker proposal in [session-runtime.md](session-runtime.md). The team self-hosts a server as the control plane. Each user runs one or more workers, on their machine or as a container, and the workers run the sessions on that user's own Claude subscription.

```
 Browser (board, web terminal)
        │  HTTPS + WebSocket
        ▼
 ┌──────────────────────────────┐
 │ Server (self-hosted)         │  accounts, boards, tickets, artifacts,
 │  API · realtime · scheduler  │  timeline, spend, tracker sync, notifications
 └──────────────┬───────────────┘
                │  WebSocket (worker connects out)
      ┌─────────┴─────────┐
      ▼                   ▼
 Worker (Alice, laptop)  Worker (Bob, container)   ← same binary either way
  claude sessions         claude sessions
  local checkouts         local checkouts
```

## Components

### Server

- **API:** boards, tickets, columns, artifacts, members and assignments.
- **Realtime:** pushes board changes and session state to browsers, and relays terminal streams between browser and agent.
- **Session registry:** each session's owner, agent, ticket, column, Claude session ID, state and last status.
- **Artifacts:** text and file artifacts with provenance (which person and session wrote them). These are small, so a database plus object storage is enough.
- **Integrations:** Jira and Linear sync, the GitHub API for PR state and CI (the same as Lanes' `gh` badges), and Slack or email notifications.
- **Spend:** stores per-session usage reported by agents; aggregates per ticket, column, person and board.

### Worker

A headless build of Lanes' core. It's the same binary on a laptop or in a container; only its folder mapping and how it gets repos differ:

- **Launch and resume:** `claude --session-id` / `--resume` with the stage's prompt, brief and `--settings` hooks (today's `session.rs` and `prompt.rs`).
- **PTY:** ConPTY or a Unix pty (containers are Linux), with output streamed to the server only while someone is viewing. It's buffered locally otherwise.
- **State:** today's hooks and status line, forwarded as events instead of written to `state/*.json`.
- **Artifacts:** the `lanes artifact set` CLI keeps working inside sessions. It writes to the worker, and the worker uploads.
- **Spend:** transcript parsing (`spend/usage.rs`) runs locally and only reports numbers. Transcripts stay on the machine unless the workspace opts in to sharing them for the Transcript tab.
- **Working folders:** local workers map the workspace's repo names to existing checkouts (Lanes' working-folder templates). Container workers clone into their volume.
- **Identity:** a worker is registered to one person and runs `claude` under that person's login. The server never stores Claude credentials.

## Protocol (worker ⇄ server)

These message types are a starting point:

| Direction | Message | Purpose |
| --- | --- | --- |
| server → worker | `start_session`, `resume_session` | Ticket, column, prompt, brief, folder and flags |
| server → worker | `input`, `resize`, `stop` | Terminal input from the owner's browser |
| worker → server | `output` | Terminal bytes (only while a viewer is attached) |
| worker → server | `state` | Hook events: working, needs input, idle |
| worker → server | `status` | Context tokens, model, plan limits |
| worker → server | `artifact_written` | Name, value or file contents, session |
| worker → server | `usage` | Per-session spend summary |
| both | `hello` / `heartbeat` | Worker version, mode (local or container), repos it can serve, online state |

Only the session owner's browser may send `input`. Others get `output` read-only, if the workspace allows watching.

## Data model (first cut)

- **Workspace** → members (roles) → boards.
- **Board** → columns (id, title, prompt, consumes, produces, model, effort, permission mode, context limit) and artifact definitions. This is the Lanes `board.toml` schema, versioned.
- **Ticket:** column, assignees per column, tracker link, history.
- **Worker:** owner, name, mode, repos, last seen.
- **Session:** ticket, column, owner, worker, Claude session ID, state, created, last active, retired (for spend), and usage.
- **Artifact:** ticket, name, value or blob, author (person and session), written at, and content hash (for re-entry change detection, as today).
- **Event:** the timeline: ticket and actor, kind, detail.

## Security notes

- **Workers connect out** and authenticate with a per-worker token the user creates. Users can revoke a worker.
- **The server can only ask a worker to run `claude` sessions** with board-defined prompts, in folders the user mapped. It can't run arbitrary commands. Board **action buttons** (which run shell commands) need a per-user opt-in, because a board edit by one person must not run commands on another person's machine. This is the biggest new risk versus single-user Lanes.
- **Prompt injection across people:** a board prompt or artifact written by one person shapes another person's session. That's inherent to the product, but it argues for permission modes being set per column and visible, and for prompt changes being reviewed.
- **Transcripts and terminal output can contain secrets.** Sharing transcripts is opt-in per workspace, and terminal relays are not stored.

## Technology (open)

The tech choices aren't the point yet. A few constraints:
- **The worker** should reuse `lanes-core` (Rust), so the CLI, hooks and spend code carry over. Ship it as a binary and as a container image.
- **The server** should be easy to self-host: one binary or a small docker-compose.
- **The browser terminal** is xterm.js, fed by the relay.
- **The server's stack** can be anything with good WebSocket support. Postgres for state and object storage for file artifacts is enough for a long while.
