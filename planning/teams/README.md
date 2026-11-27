# Lanes for teams (idea)

> **Status:** an idea being explored. Everything in this folder is a proposal, not a decision. It would be a separate product; Lanes itself stays a single-user desktop app. Written 26 Nov 2026.

## The idea in one paragraph

Lanes gives one developer a kanban board where each ticket gets its own focused Claude session per stage, and stages hand work on through small, explicit artifacts. **Lanes for teams** keeps that workflow and puts it on a shared board in a web app. Several people log in, see the same tickets, and own different stages: one person plans, another implements, a third reviews. The board shows every session's state and spend. The hand-off between *people* uses the same artifacts as the hand-off between *sessions*.

The proposed shape is **self-hosted, not SaaS**. A team runs the server itself, and each person runs Claude on their own subscription through a worker on their machine or in a container (see [session-runtime.md](session-runtime.md)).

## Why it might be worth building

- **The artifact model already fits teams.** In Lanes, a later stage never sees an earlier transcript, only its artifacts. That is exactly what a hand-off between two people needs. Nobody has to read a 900k-token conversation to pick up the ticket.
- **Teams lack visibility into agent work.** Today each developer's Claude sessions are invisible to everyone else. A shared board answers "who is working on what", "what is waiting on a human" and "what did this ticket cost".
- **Process as code.** A team's columns, prompts and artifact definitions are the team's development process, versioned and shared. Changing the review prompt changes how everyone's review stage behaves.
- **Spend becomes a team metric.** The per-ticket ledger in Lanes becomes cost per ticket, per stage and per person. That is useful for budgeting and for spotting stages that waste context.

## Why it might not be

- **It's a crowded space** (see [landscape.md](landscape.md)). Issue trackers are adding agents, and agent tools are adding boards. The nearest single-user tool, Vibe Kanban, lost its company in April 2026, which says something about the business.
- **Session hosting is the hard part.** Interactive sessions need a machine, a repo checkout and a Claude credential. The proposal keeps these with each user, but container workers still need images, secrets and a host (see [session-runtime.md](session-runtime.md)).
- **Teams already have a tracker.** A second board that duplicates Jira or Linear will lose. It has to sit alongside the tracker, not replace it.

## Documents

| File | What it covers |
| --- | --- |
| [product.md](product.md) | Who uses it, the core flows, what carries over from Lanes, what's new, and non-goals |
| [session-runtime.md](session-runtime.md) | Where Claude sessions run: the generic worker proposal, alternatives and trade-offs |
| [architecture.md](architecture.md) | Control plane, worker protocol, data model, auth and security |
| [landscape.md](landscape.md) | Similar tools and where this would sit |
| [phases.md](phases.md) | A possible staged path, with open questions |

## Principles carried over from Lanes

1. **People move tickets.** Agents don't move themselves. Automation is opt-in and per column.
2. **Real, interactive sessions.** A stage is a conversation a person can join, not a background job that returns a diff.
3. **Artifacts are the only thing that moves between stages.** That stays true when the stages belong to different people.
4. **The board is the process.** Columns, prompts and artifacts are team configuration, reviewed like code.
