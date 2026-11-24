# Roadmap to 1.0 (draft)

This is a draft built from research in November 2026. The [open questions](#open-questions) at the end decide the order. Nothing here is committed to yet.

Lanes is an organisational tool for a person working with agents, not an autonomous multi-agent harness. Every item below either saves that person attention or saves tokens over a ticket's lifetime, or it isn't on the path to 1.0.

## Where Lanes stands

Claude Code now ships its own **Agent view** (`claude agents`, `--bg` sessions) and a **Desktop sidebar**. Both show which session is working, waiting or done, and send notifications. Monitoring sessions alone is no longer a reason to use Lanes.

What Lanes has that those tools don't:
- **Stages:** one focused session per stage, with explicit artifact hand-offs and resume per stage.
- **Ticket lifetime:** a ticket is the unit, not a chat, and it lives from Jira all the way to archive.
- **Windows-first:** Conductor is Mac-only, and most rivals ship on Mac first.

The closest analogue is Nimbalyst: session cards on a kanban board, but without per-stage sessions or artifacts. Vibe Kanban, the best-known kanban-for-agents tool, shut down commercially in April 2026, so being a sustainable side project matters more than features.

## 1.0 gate: what has to be stable

1.0 is a promise that upgrades won't break your board. Before tagging it:

- **Board format:**
  - Add a `version` key to `board.toml`.
  - Add `lanes config migrate` and test it against fixture boards from every 0.x release.
  - Ticket and state files need the same treatment.
- **Integration surface:** freeze `lanes hook`, `lanes statusline`, `lanes artifact set` and the per-session settings file. Users' own Claude settings and prompts depend on them.
- **Releases:**
  - At least three 0.x releases go through the pipeline cleanly.
  - Add an in-app "new version available" check, with no auto-update.
- **Claude Code drift:**
  - Record the minimum supported Claude Code version.
  - Warn on startup if the installed one is older.
  - Statusline fields Lanes reads must degrade gracefully when they're missing.
- **Platform statement:** 1.0 is Windows-only by design. macOS is the first post-1.0 goal, not a gate.

## Step zero: measure before building

Theme A rests on an assumption: that cold resumes and oversized stage sessions are where tokens go. Claude Code's transcripts (`~/.claude/projects/**/*.jsonl`) record input, output, cache read and cache write tokens for every turn. A one-off read-only script over real Lanes sessions can answer that before any feature is built:
- **Cold resumes:** what share of spend was cache writes on the first turn after a resume?
- **Session size:** how large do sessions get, by stage?
- **Rework:** how often do tickets go back to an earlier stage?

If cold resumes turn out to be small, Theme A shrinks to the ledger (A1) and per-stage defaults (A4).

## Theme A: token spend over a ticket's lifetime

This is the most distinctive direction. Lanes is the only tool here that knows a session's whole arc: what stage it's in, how long it has been parked, and what it hands off.

### A1. Measure first: a per-ticket spend ledger
- **What the status line gives us:** Lanes already parses the status line. It currently keeps only context % and the plan limits, but the same JSON carries:
  - `cost.total_cost_usd`
  - cache read and write token counts
  - in recent versions, a `prompt_cache` block with hit ratio, miss causes, `expires_at` and `recache_tokens_if_cold`
- **Ledger:** keep the last snapshot per session, then roll it up per stage and per ticket. Show the cost on the card, per-stage figures in the timeline, and a board total.
- **Plan-limit impact:** record the 5-hour and 7-day limit % at session start and end. It's noisy when sessions overlap, but it's the only measure of plan usage a subscriber has.
- **Exact attribution (optional):** set `OTEL_RESOURCE_ATTRIBUTES=ticket.id=…,stage=…` at launch, so anyone running an OpenTelemetry collector gets exact figures, subagents included.

### A2. Warm vs cold sessions
Prompt caching dominates cost. A resumed session whose cache has expired re-reads its whole transcript at cache-write price. On subscriptions the main conversation has a 1-hour time to live. Tickets sit in Review overnight, so most resumes in Lanes are cold.

- **Card indicator:** show whether a parked session's cache is warm, with "warm for 12 min" from `prompt_cache.expires_at`.
- **Choice at resume:** a cold, large session prompts:
  - *Resume (re-reads ~180k tokens)*, or
  - *Fresh session seeded with this stage's artifacts and a summary*.

  Lanes' artifact model makes the fresh path cheap. The plain fix is a *stage summary* artifact the session writes before it's parked (A3).
- **Prompts:** keep per-stage system prompt files byte-identical across launches, because Claude Code ignores a changed `--append-system-prompt` on `--resume` until compaction. Lanes already sends re-entry notes as a message, which is the right approach. Add a test that pins it.

### A3. Leaving a stage while the cache is warm
- **Leave stage action:**
  1. While the cache is still warm, ask the session to write its outputs plus a short `stage_summary`: decisions, open questions, what it would do next.
  2. Then stop the session.

  This replaces "leave the session open and compact it later", which is the expensive pattern.
- **After compaction:** use a `SessionStart` hook with source `compact` to re-inject the ticket brief and current artifacts. Compaction keeps only a few recent files and drops path-scoped rules.

### A4. Per-stage model, effort and context defaults
- **Starter board defaults:**
  - Opus with high effort for Plan.
  - Sonnet with medium effort for Dev and Review.
  - Sonnet or Haiku with low effort for QA.
  - Haiku for subagents (`CLAUDE_CODE_SUBAGENT_MODEL`).
  - Document why: model and effort changes mid-session invalidate the cache, so choose per stage, not per turn.
- **Per-column settings:** expose `autocompact` (the window, e.g. 200k for Dev) and idle-compaction off for parked sessions. Claude Code runs some work in the background while a session is idle.
- **Output trimming:** ship an optional "trim long tool output" hook, the docs' `PreToolUse` example, as a board setting.

### A5. Spend-aware nudges (later)
- **Over budget:** "This ticket has used 3× the board median" on the card. It's a prompt for a human decision, not a limit.
- **Re-runs:** flag stage re-entries where most input tokens were spent re-reading, not working.

**Caveats:**
- How cache reads count against subscription limits is not officially documented. Treat changes in the limit % as ground truth.
- Several fields and behaviours (the `prompt_cache` block, TTL rules) are recent and version-gated. Verify each against the installed Claude Code (2.1.331 here) before building on it.

## Theme B: human attention

Every piece of writing on parallel agents agrees that the scarce resource is the person. Three well-supervised threads beat six half-watched ones.

- **Needs-you queue:** one list sorted by what you have to do:
  1. answer a question or approve
  2. failed CI
  3. ready for review
  4. running
  5. parked

  It replaces scanning the board.
- **OS notifications:** Windows toast notifications on "needs input", "finished" and "CI failed", with click-through to the ticket. The tray badge alone is easy to miss.
- **WIP limits:** set per column or board-wide, framed as protecting focus, with a soft warning rather than a block.
- **Re-orientation card:** when you open a ticket after a break, lead with what changed since you last looked: new artifacts, the last messages, CI, time parked.

## Theme C: reviewing and trusting output

Reviewing is the bottleneck once agents write the code.

- **Diff tab:** branch vs base per ticket, with comments that become the next re-entry note for the Dev session. Every rival has this, so it's the clearest table-stakes gap.
- **Evidence on exit:** a standard `evidence` artifact for the Dev and QA stages: files touched, tests run and their results, known gaps. "Done" becomes a claim with receipts, and the reviewer starts from it.
- **Plan drift check:** before Review, compare the diff with `plan.md` and ask the Dev session to update the plan or explain the drift. Stale plans are the top complaint about spec-driven work.

## Theme D: parallel tickets without collisions

- **Overlap warning:** when two active tickets' worktrees touch the same files (from `git diff --name-only` against base), badge both cards.
- **Merge order hint:** in Review, suggest which ticket to land first and flag ones that will need a rebase.
- **Multi-repo tickets:** one ticket spanning several repos or worktrees, e.g. a package plus its consumer. Reviewers call this out as a weak spot in Claude Desktop, and it's a real workflow here.

## Theme E: tracker round-trip

- **Jira write-back (opt-in):** moving a card transitions the issue, and the PR link and stage summary are posted as a comment. Linear now runs coding sessions natively from issues, so being the tracker's companion matters.
- **More sources:** import from GitHub Issues, Linear later.

## Nice to have, deliberately not the focus

- **Agents moving tickets.** A column option such as `allow_session_moves = ["review"]` would let a Dev session run `lanes ticket move` when its outputs exist. The human still starts every session, so it stays organisational.
- **Auto-start the next stage** after an agent-initiated move: off by default.
- **Other agent CLIs** (Codex). Rivals increasingly support several, but Lanes' hooks and status line are Claude-specific, so this is post-1.0.
- **Planning critic, bouncer agents, fan-out:** autonomous orchestration. Out of scope.

## Review of this draft

- **Positioning:** a feature that only reports session state competes with Claude Code itself. Features that use stages, artifacts or the ticket's lifetime don't.
- **Config, not code:** several items are mostly changes to the starter board and prompts:
  - per-stage model and effort defaults (A4)
  - the evidence artifact (C)
  - the stage summary (A3)
  - the compaction hook (A3)

  They're cheap, and can ship early alongside code changes.
- **Diff tab cost:** a good diff viewer in GPUI is weeks of work. Start with "Open diff in VS Code" plus a comment box that sends review comments to the Dev session as its next re-entry note. Build it in-app only if switching windows proves to be real friction.
- **Single-user scope:** merge-order hints and WIP limits matter most with many parallel tickets. Merge-order hints are the first candidate to cut. Multi-repo tickets stay, because the package-plus-consumer workflow is real.

## Suggested sequence

| Release | Focus |
| --- | --- |
| 0.2 | Step zero measurement, spend ledger (A1), per-stage defaults and the config-only items above |
| 0.3 | Warm/cold indicator and resume choice (A2), Leave stage with summary (A3) |
| 0.4 | Needs-you queue and toast notifications (B), diff via VS Code plus comments to Dev (C) |
| 0.5 | Overlap warnings, multi-repo tickets, Jira write-back if wanted (D, E) |
| 0.9 | Board versioning and migrations, contract freeze, min Claude Code version check |
| 1.0 | Three clean releases on 0.9, docs pass, at least one outside user set up from the README |

## Open questions

1. **Who is 1.0 for?**
   - If it's mainly the author and a few colleagues, the stability gates (migrations, contract freeze, three clean releases) are overhead. 1.0 then means "used daily for a month without friction".
   - If strangers are meant to adopt it, the gates stay.
2. **What hurts most today?** Spend and limits, losing track of which ticket needs you, or reviewing agent output? The answer should set the 0.x order, not the table above.
3. **Jira write-back:** should Lanes transition issues when cards move, or is Jira only a source of tickets?
4. **Agents moving tickets:** keep it as a per-column opt-in under nice-to-have, or drop it for now?

## Sources

- **Claude Code docs:**
  - [costs](https://code.claude.com/docs/en/costs)
  - [prompt caching](https://code.claude.com/docs/en/prompt-caching)
  - [status line](https://code.claude.com/docs/en/statusline)
  - [monitoring](https://code.claude.com/docs/en/monitoring-usage)
  - [model config](https://code.claude.com/docs/en/model-config)
  - [context window](https://code.claude.com/docs/en/context-window)
  - [CLI reference](https://code.claude.com/docs/en/cli-reference)
  - [agent view](https://code.claude.com/docs/en/agent-view)
- **Anthropic:**
  - [effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
  - [API pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- **Other tools:**
  - [Nimbalyst](https://nimbalyst.com/kanban-for-claude-code/)
  - [Vibe Kanban shutdown](https://vibekanban.com/blog/shutdown)
  - [Conductor](https://www.conductor.build/docs/guides/parallel-agents/run-multiple-claude-code-sessions)
  - [Codex app](https://developers.openai.com/codex/app/features.md)
  - [Linear coding sessions](https://linear.app/changelog/2026-06-11-coding-sessions)
  - [Claude Desktop review](https://dev.to/stravukarl/claude-code-desktop-after-the-redesign-what-anthropic-shipped-and-what-power-users-still-miss-150h)
- **Supervising agents:**
  - [Osmani on parallel agents](https://addyosmani.com/blog/cognitive-parallel-agents/)
  - [review as a queue of evidence](https://dev.to/agentis/review-multiple-coding-agents-as-a-queue-of-evidence-1p62)
  - [agentic drift and merge cost](https://dev.to/helgesverre/agentic-drift-its-hard-to-be-multiple-developers-at-once-4872)
  - [spec drift](https://www.augmentcode.com/blog/what-spec-driven-development-gets-wrong)
