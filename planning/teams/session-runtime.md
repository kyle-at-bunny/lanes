# Where sessions run

> Proposals and trade-offs, not decisions.

Lanes runs `claude` as a child process on the user's own machine, using their own Claude login and their own checkouts. A web app can't do that directly. Something has to host each session's process, filesystem and credentials, and stream it to a browser. This choice shapes cost, security and setup, so it comes first.

## What a session needs

- **A process:** the real `claude` CLI in a pseudo-terminal, interactive, long-lived (hours to days, mostly idle) and resumable by session ID.
- **A working copy** of the repo(s), with the toolchain to build and test them.
- **A Claude credential.** Whose it is decides who pays and whose plan limits apply.
- **A channel to the server:** terminal I/O, hook events (state), status line (context, cost) and artifact writes.
- **Persistence:** the transcript (`~/.claude/projects/...`) and the working tree, so it can resume later.

## Proposal: one generic worker, run wherever suits the user

A single **worker** program (a headless Lanes) runs sessions and connects *outbound* to the team's server over a WebSocket. The server asks it to start or resume a session for a ticket and column. The worker spawns `claude` exactly as Lanes does today, then streams the terminal, hook events and status back.

The worker doesn't care where it runs. The same binary works in two modes:

| | On the user's machine | As a container worker node |
| --- | --- | --- |
| Setup | Install, sign in to the server, done | Run an image (one per user) on a team host |
| Claude login | The user's existing `claude` login | The user signs in once inside their container; the login lives on a persistent volume |
| Repos | The user's existing checkouts, mapped by name | Cloned into the container's volume by the worker |
| Toolchain | Whatever the machine has | Baked into the image (a devcontainer-style definition per repo) |
| Availability | While the machine is awake | Always on |
| Best for | Getting started, solo use, laptops | Long sessions, watching from anywhere, consistent environments |

**Each worker belongs to one person and runs on that person's own subscription.** A user can have several workers, for example a laptop and a container, and pick one when starting a session. A session stays on the worker that started it, because that's where its transcript and working tree are.

Why this is attractive:
- **Easy start, smooth upgrade path.** Run it locally on day one; move to a container when always-on matters. Nothing else changes.
- **No shared credentials.** Everyone uses their own Claude plan, as with Lanes today. The server never holds Claude credentials.
- **Reuse.** Almost all of `lanes-core` (launch and resume, prompts, hooks, status, transcripts, spend) moves into the worker as-is.

Open points with this proposal:
- **Terms:** confirm that a personal subscription signed in inside a container the user controls is fine. This is the person's own use, but on a server rather than their laptop. Sharing one subscription across people is clearly not the model.
- **Container images:** who defines them per repo, and how secrets and network access are handled. This is the bulk of the work in the container mode.
- **Offline local workers:** when a laptop sleeps, its sessions are offline. Teammates still have the artifacts; the owner resumes when back.

## Alternatives considered

### Hosted by a vendor (SaaS)

A service runs the workers for every team, probably on API keys with API billing. It's always on and needs no setup, but it means becoming a compute, security and billing provider. Lanes' measurements show cache reads alone are 60–75% of spend, which makes API economics hard. **This is not the direction proposed here.** The proposal is for teams to self-host the server and run their own workers.

### Claude Code's own cloud sessions

Claude Code can already run sessions in Anthropic-hosted cloud environments (`claude --cloud`, claude.ai/code) and pull them into a terminal (`--teleport`). These run against the user's own plan.
- **Upside:** no infrastructure, own credentials, and Anthropic handles sandboxing.
- **Unknowns:** whether a third-party app can start, observe and stream these sessions, receive hook-equivalent state, and get transcripts programmatically. As of Nov 2026 this looks centred on Anthropic's own UIs. Hand-off is one-way from cloud to terminal, and a teleported copy diverges.
- **Possible later use:** a board could *link* a stage to a cloud session the user started (by ID), if an API allows it. It could be a third worker "mode" rather than a separate design.

## Questions to settle

- **Terms** for running a personal subscription inside a self-hosted container worker.
- **Remote session API:** does Anthropic expose (or plan) an API to drive cloud sessions?
- **Worker selection:** does the user choose per session, or is there a per-user default with per-column overrides (e.g. QA always on the container)?
- **Repo identity:** map the board's repo names to each worker's paths. For local workers, this is Lanes' working-folder templates per user; container workers clone.
