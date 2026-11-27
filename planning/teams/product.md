# Product

> A proposal, not a decision.

## Who uses it

| Role | What they do on the board |
| --- | --- |
| **Developer** | Picks up a stage of a ticket, works with its Claude session, writes the stage's outputs, hands on |
| **Reviewer / QA** | Owns the review or QA stage; often a different person from the developer |
| **Lead** | Shapes the board (columns, prompts, artifacts), watches flow and spend, unblocks |
| **Admin** | Runs the self-hosted server and manages the workspace: members, repos, worker images |

The first target is a small product team (3–10 developers) that already uses Claude Code day to day and wants it to be visible and consistent across the team.

## Core flows

### 1. A ticket moves through people, not just stages

1. A lead creates or imports a ticket (from Jira or Linear) into Backlog.
2. Alice drags it to **Plan** and clicks **Start session**. A Claude session starts on one of *her* workers (her laptop or her container, see [session-runtime.md](session-runtime.md)), on her own subscription, seeded with the plan prompt and the ticket.
3. When `plan.md` is written, Alice moves the ticket to **Dev** and assigns Bob. Bob starts a fresh Dev session seeded with the plan. He never needs Alice's transcript.
4. Carol owns **Review**. Her session sees the plan and the branch. Her `review_notes` send the ticket back to Dev.
5. The ticket returns to Bob, and **Resume session** brings back *his* Dev conversation, told what changed.

**Rule:** a session belongs to the person who started it. Moving a ticket to someone else never hands them your live session. They get the artifacts and start their own. That mirrors Lanes' stage isolation and avoids credential sharing.

### 2. Seeing what needs a human

- Cards show each session's state: working, needs input, your turn, parked.
- **My attention** view: every session of mine that is waiting on me, across tickets.
- **Team view:** what everyone is waiting on, and for how long. A session idle on "needs input" for two hours is a blocked ticket.
- Notifications (Slack or email) when your session needs input or a ticket lands in a column you own.

### 3. Joining someone else's session

- **Read:** anyone on the team can read a stage's transcript, as in Lanes' Transcript tab. A workspace setting can restrict this.
- **Watch:** a live, read-only view of a running session's terminal, for pairing or helping someone who's stuck. It needs the owner's worker to be online.
- **Take over:** deliberately out of scope for v1. It means running on someone else's credentials and machine.

### 4. The board as team process

- Board config (columns, prompts, artifacts, actions) lives in a git repo or in the app with version history. Changes are proposed and reviewed.
- Stage defaults (model, effort, permission mode, context limit) are set per column for the whole team.
- The board assistant still exists. It proposes config changes for a lead to approve.

### 5. Spend and flow

- Per ticket: spend per stage and per person, peak context, and number of sessions. This is Lanes' spend ledger, aggregated.
- Per board: cycle time per column, rework loops (Review → Dev bounces), and spend per finished ticket.
- The Lanes lean/baseline A/B test generalises to "board variants": two prompt or limit settings compared on real tickets.

## What carries over from Lanes

| Lanes (single user) | Lanes for teams |
| --- | --- |
| Board in `~/.lanes/board.toml` | Workspace board, versioned; same schema where possible |
| One session per ticket per stage | One session per ticket per stage **per owner** |
| Artifacts in the ticket folder | Artifacts stored by the server; files and text, with provenance (who wrote it, from which session) |
| Status from Claude Code hooks and status line | The same hooks, reported by the runtime to the server |
| Embedded terminal | Web terminal (xterm.js) streaming from the runtime |
| Timeline per ticket | Same, plus who did what |
| Jira import | Two-way tracker sync: status and links pushed back |
| Spend ledger from transcripts | Same, computed on the runtime and reported up |

## What's new

- Accounts, workspaces, membership and roles.
- Assignment: a stage has an owner; the card shows who.
- Real-time board updates for everyone (moves, states, artifacts).
- Notifications.
- Repo access managed per workspace: which repos, and how runtimes get checkouts.
- Audit log: who started which session in which repo, and who moved what.

## Non-goals (at least for v1)

- **An issue tracker.** Sync with Jira or Linear; don't replace them.
- **Autonomous agents moving tickets.** Opt-in column automation could come later.
- **Shared live sessions with multiple typists.** Read, watch, and hand off via artifacts instead.
- **Non-Claude agents.** Possible later. The artifact model is agent-agnostic, but v1 is Claude Code only.
- **Code review UI.** Link out to the PR; the review stage's artifact summarises findings.
