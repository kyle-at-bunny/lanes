# Phases

> A possible path, not a plan of record.

Each phase should answer a question before the next one spends more effort.

## Phase 0: prove the team workflow with Lanes as it is (weeks)

**Question:** do stage hand-offs between *people* via artifacts actually work in practice?

- Two or three developers share a board folder (a synced folder or a git repo), each running desktop Lanes against it.
- Lanes already tolerates concurrent writers to the same board: append-only event logs, and the CLI and the GUI both write. Check what breaks with two GUIs.
- Add only what's needed: an `owner` per session record, and showing it on cards.

**Exit:** the team keeps using it for a month, and hand-offs happen without people reading each other's transcripts.

## Phase 1: server and board in the browser, sessions still local (1–2 months)

**Question:** does a shared, live board change how the team coordinates?

- Server: accounts, one workspace, boards, tickets, artifacts, timeline, realtime updates.
- Desktop Lanes connects to the server as its board store instead of `~/.lanes`, and keeps running sessions locally as today.
- The web UI is the board plus ticket panels (artifacts, timeline, owner, state). There's no web terminal yet. "Open session" jumps to the owner's desktop app.

**Exit:** people check the web board to see what's blocked or waiting, instead of asking in chat.

## Phase 2: worker and web terminal (2–3 months)

**Question:** is working in a browser good enough to replace the desktop app for the session itself?

- Split a headless **worker** out of Lanes (`lanes-core` plus a WebSocket client), following the [architecture](architecture.md) protocol. Ship it as a binary first; add the container image once the protocol settles.
- Web terminal via xterm.js. Watch mode for teammates.
- Notifications: needs input, assigned to you, and moved into your column.
- Spend reporting per ticket and per person.

**Exit:** most sessions are started and driven from the browser.

## Phase 3: product (open-ended)

- Two-way Jira and Linear sync, GitHub PR and CI state, and Slack.
- Board config versioning and review, plus per-column permission policies.
- Container worker images per repo (toolchain, secrets, network policy).
- Flow and spend analytics, and board variant experiments.

## Open questions

- **Name and repo:** a separate repo, and probably a separate name. Lanes stays the single-user app, and the worker shares `lanes-core`.
- **Claude terms** for running a personal subscription in a self-hosted container worker. Settle this before shipping container workers (see [session-runtime.md](session-runtime.md)).
- **How much is the transcript shared?** Is it privacy-sensitive per person, or team-visible by default?
- **Ownership edge cases:** a stage's owner is away and the ticket is urgent. The answer is probably "start your own session from the artifacts", with the board making that one click.
- **Is phase 0 enough** for small teams on its own? If a shared folder gets most of the value, that's worth knowing before building a server.
