# Landscape (Nov 2026)

This is a quick scan, not a market study. Most sources are vendor comparison pages, so verify the claims before relying on them.

## Nearby tools

| Kind | Examples | How they differ from this idea |
| --- | --- | --- |
| Local agent kanban boards | Vibe Kanban (company shut down Apr 2026; local tool still usable), Orchestrator.inc, Pivio, Emdash | Single-user boards that launch agents on tickets. Most treat a card as a task for an agent, not as stages with human-owned sessions |
| Team agent boards | AgentVera (team boards, hand a card to an agent), Nimbalyst (Teams tier in beta), NovumOS (agents as board members) | The closest to this idea. They tend toward "agent does the card" rather than stage-by-stage interactive sessions with artifact hand-offs |
| Trackers adding agents | Jira and Linear agent integrations, GitHub issues assigned to coding agents | Own the team's tickets already. Agents run as background jobs that return a PR |
| Hosted coding agents | Claude Code cloud sessions, Devin, Cursor background agents, Codex cloud | Run the session for you. Per-task and per-person, with no team board or stage model |

## Where this would sit

The defensible angle isn't "a board with agents". That's crowded. It's three things together:

1. **Stages with separate, human-owned, interactive sessions.** This is closer to how teams actually work (plan, build, review, QA by different people) than "assign the card to an agent".
2. **Artifacts as the contract between stages and between people.** These are small, reviewable hand-offs, with no transcript sharing needed.
3. **Spend and flow visibility per ticket and stage.** This is grounded in Lanes' measurements: most spend is cache reads on oversized sessions, which is a team-level problem worth managing.

It should **sit alongside the tracker**, with two-way sync, rather than compete with Jira or Linear. And as proposed it's **self-hosted on each person's own subscription** (see [session-runtime.md](session-runtime.md)). That means no hosted compute or API bill to fund, and code never leaves the team's own machines.

## Lessons from Vibe Kanban's shutdown

The reported story is a popular open-source local tool, followed by hosted features, and then the company folding. It suggests:
- a local tool alone is hard to monetise;
- hosted compute is expensive to give away.

Self-hosting avoids that trap: there's no hosted compute to pay for, so the tool can stay small and team-run.

## Sources

- [Vibe Kanban alternatives after the Bloop shutdown (agentvera.dev)](https://agentvera.dev/en/blog/vibe-kanban-alternatives/)
- [Best Vibe Kanban alternatives in 2026 (orchestrator.inc)](https://orchestrator.inc/compare/vibe-kanban-alternative/)
- [Best Vibe Kanban alternatives 2026 (nimbalyst.com)](https://nimbalyst.com/blog/best-vibe-kanban-alternatives-2026/)
- [Vibe Kanban alternative with a business model (novumos.app)](https://novumos.app/alternatives/vibe-kanban)
- [Claude Code `--teleport` guide (thepromptshelf.dev)](https://thepromptshelf.dev/blog/claude-code-teleport-flag-guide-2026/)
- [Claude Code cloud sessions explained (aq.dev)](https://aq.dev/guides/claude-code-cloud-sessions-explained/)
- [Hosting the Agent SDK (code.claude.com)](https://code.claude.com/docs/en/agent-sdk/hosting)
