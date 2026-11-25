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

## Step zero: what the transcripts say (measured 25 Nov 2026)

A read-only script went through every Claude Code transcript on the author's machine (`~/.claude/projects`). The data:
- July 31 to November 25 2026
- about 17,000 API requests in 49 sessions, 5 of them Lanes stage sessions
- weighted at API price ratios: uncached input 1, cache write 1.25 (5-minute) or 2 (1-hour), cache read 0.1, output 5

| Where spend went | Share |
| --- | --- |
| Cache reads: the whole context re-read on every turn | **60–75%** |
| Output | 8–13% |
| Warm cache writes and uncached input | 6–10% |
| Cold rebuilds after a gap of an hour or more | 5–8% |
| Cold rebuilds after a 5–60 minute gap, or a model change or compaction | 2–3% |
| Subagents (all their reads, writes and output) | 8–10% |

The range comes from weighting cache reads at 0.1 or 0.05.

**Spend by context size at the time of the request:**

| Context | Share of requests | Share of spend | Cost per request |
| --- | --- | --- | --- |
| under 100k | 17% | 6% | 1× |
| 100–200k | 22% | 12% | 1.4× |
| 200–400k | 24% | 22% | 2.4× |
| 400–700k | 23% | 34% | 3.9× |
| over 700k | 13% | 26% | 5.5× |

Sessions peak at a median of 330k tokens of context (90th percentile 934k). Auto-compaction on 1M-token models only starts at about 967k, so sessions run nearly to the limit.

**What this means:**
- **Session size is the main cost.** Spend is driven by how big sessions get, not by cold resumes. 60% of it happens on requests with more than 400k of context.
- **Cold resumes matter less than assumed.** They are real but secondary: 79 cold rebuilds at a median of 300k, about 5–8% of spend.
- **Fresh sessions per stage are the right instinct.** Lanes' core design already does this. The next step is to keep each stage's session small.

**Caveats:**
- The data is one person's history over about four months, and mostly not Lanes sessions. The five Lanes sessions show the same pattern; the Dev session peaked at 483k.
- The weights are API price ratios. How a subscription counts cache reads isn't documented.
- Costs are compared within a model, not across models.

The script is not checked in. Re-run the same analysis per ticket once the ledger (A1) exists.

## Theme A: token spend over a ticket's lifetime

Lanes is the only tool here that knows a session's whole arc: what stage it's in, how big it has grown, how long it has been parked, and what it hands off. The measurements put **context size** first and cold resumes second.

### A1. A per-ticket spend ledger
- **What the status line gives us:** Lanes already parses the status line. It currently keeps only context % and the plan limits, but the same JSON carries:
  - `cost.total_cost_usd`
  - cache read and write token counts
  - in recent versions, a `prompt_cache` block with hit ratio, miss causes, `expires_at` and `recache_tokens_if_cold`
- **Ledger:** keep the last snapshot per session, then roll it up per stage and per ticket. Show the cost on the card, per-stage figures in the timeline, and a board total.
- **Plan-limit impact:** record the 5-hour and 7-day limit % at session start and end. It's noisy when sessions overlap, but it's the only measure of plan usage a subscriber has.
- **Exact attribution (optional):** set `OTEL_RESOURCE_ATTRIBUTES=ticket.id=…,stage=…` at launch, so anyone running an OpenTelemetry collector gets exact figures, subagents included.

### A2. Keep stage sessions small (the main lever)
- **Context limit per column:**
  - Add a `max_context` key, passed as `--autocompact <tokens>` on every launch. It defaults to about 300k in the starter board instead of Claude Code's ~967k.
  - Every request above 400k costs roughly 4–5× one under 100k. A limit trades a compaction for that steady cost.
- **Size nudge:** when a stage session passes a threshold (say 250k), the card and terminal header suggest *Hand off and start fresh*. That action:
  1. asks the session to write its outputs and a short `stage_summary`
  2. ends the session
  3. starts a new session for the same stage, seeded with the brief, the artifacts and the summary

  This is the same mechanism as "Leave stage" (A3), used within a stage.
- **Re-injection after compaction:** use a `SessionStart` hook with source `compact` to put the ticket brief and current artifacts back. Compaction keeps only a few recent files and drops path-scoped rules, and the hook makes a lower limit safe.
- **Output trimming:** ship an optional "trim long tool output" hook, the docs' `PreToolUse` example, as a board setting. Long test and build logs are what inflate a session quickly.

### A3. Leaving a stage cleanly
- **Leave stage action:**
  1. While the cache is still warm, ask the session to write its outputs plus a short `stage_summary`: decisions, open questions, what it would do next.
  2. Then stop the session.
- **Cheap re-entry:** when the ticket comes back, the stage can start fresh from the summary and the new inputs instead of resuming a 400k transcript.

### A4. Warm vs cold resumes (secondary)
Cold rebuilds were 5–8% of measured spend: worth handling, but not first. A resumed session whose cache has expired re-reads its whole transcript at cache-write price. On a subscription the main conversation's cache lasts an hour.

- **Choice at resume:** for a session that is both cold and large, offer:
  - *Resume (re-reads ~N tokens)*, or
  - *Fresh session from the stage summary*.

  Lanes can compute the size from the last status snapshot, so this doesn't need the newer `prompt_cache` fields.
- **Prompts:** keep per-stage system prompt files byte-identical across launches. Claude Code ignores a changed `--append-system-prompt` on `--resume` until compaction. Lanes already sends re-entry notes as a message, which is the right approach; add a test that pins it.

### A5. Per-stage model and effort defaults
- **Starter board defaults:**
  - Opus with high effort for Plan.
  - Sonnet with medium effort for Dev and Review.
  - Sonnet or Haiku with low effort for QA.
  - Haiku for subagents (`CLAUDE_CODE_SUBAGENT_MODEL`).
- **Why per stage:** model and effort changes mid-session invalidate the cache, so they're chosen per stage, not per turn.
- **Idle work:** turn idle compaction off for parked sessions. Claude Code otherwise does some work in the background while a session is idle.

### A6. Spend-aware nudges (later)
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
- **In-app diff viewer.** Diffs are reviewed in the editor and on GitHub. Lanes links to them rather than rendering them.
- **Planning critic, bouncer agents, fan-out:** autonomous orchestration. Out of scope.

## Review of this draft

- **Positioning:** a feature that only reports session state competes with Claude Code itself. Features that use stages, artifacts or the ticket's lifetime don't.
- **Config, not code:** several items are mostly changes to the starter board and prompts:
  - a lower default context limit (A2)
  - the compaction re-injection hook (A2)
  - the stage summary (A3)
  - per-stage model and effort defaults (A5)
  - the evidence artifact (C)

  They're cheap, and can ship early alongside code changes.
- **Single-user scope:** merge-order hints and WIP limits matter most with many parallel tickets. Merge-order hints are the first candidate to cut. Multi-repo tickets stay, because the package-plus-consumer workflow is real.

## Suggested sequence

| Release | Focus |
| --- | --- |
| 0.2 | Per-column context limit and compaction hook (A2), spend ledger (A1), per-stage defaults and the other config-only items above |
| 0.3 | Size nudge with "Hand off and start fresh" (A2), Leave stage with summary (A3), cold-resume choice (A4) |
| 0.4 | Needs-you queue and toast notifications (B), evidence artifact and plan drift check (C) |
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
