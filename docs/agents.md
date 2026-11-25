# Adding an agent CLI

Lanes runs every session as a real, interactive coding agent CLI in a terminal. Which CLI is behind the `AgentCli` trait in `crates/lanes-core/src/agent/`. Claude Code (`claude.rs`) and OpenCode (`opencode.rs`) are the two adapters today. This page is for adding another one.

## What Lanes needs from an agent

Required:
- **An interactive TUI** that runs in a pseudo-terminal, accepts pasted text, and submits on Enter.
- **Resume by id.** Either Lanes picks the session id (Claude Code takes a UUID; OpenCode takes an id starting with `ses`) or the agent assigns one that Lanes can read back.

Everything else is a capability (`Capabilities`). When one is missing, Lanes degrades rather than failing:

| Capability | Used for | Without it |
| --- | --- | --- |
| `system_prompt` | Appending the stage prompt (and, with `start = "wait"`, the brief) to the system prompt | The stage prompt and brief go in the first message, and `start = "wait"` behaves like `"prompt"` |
| `live_state` | Working / needs input / your turn on cards, attention toasts, the quit check | Cards show **running** while the terminal is open |
| `usage` | "ctx N%" on cards and the 5h / 7d plan meters | Not shown |
| `transcript` | The Transcript tab and session sizes in the Sessions view | The tab says transcripts aren't available |

## The trait

`crates/lanes-core/src/agent/mod.rs` has the full signatures. Most methods have defaults, so an adapter only overrides what its CLI supports:

- `kind`, `display_name`, `default_program` and `capabilities`. `default_program` is the name looked up on PATH when `[agents.<kind>] path` isn't set.
- `new_session_id(lanes_id)`: the agent's own id for a new session. It's stored on the session record (`agent_session_id`) when it differs from Lanes' id.
- `launch_args(request)`: the arguments for a new or resumed session. `LaunchRequest` carries:
  - the agent's session id and whether this is a resume;
  - the system prompt file (new sessions only);
  - an initial message (the brief, or a re-entry note);
  - `model`, `effort`, `permission_mode` and `extra_args`.
  - `max_context`, set for lean tickets under `[spend]`: compact the conversation at that many tokens (Claude Code's `--autocompact`).

  Write any per-session files the CLI needs here. Claude's `--settings` hooks file is an example.
- `ignored_settings()`: stage settings the CLI can't apply. `lanes config check` warns about them.
- `assistant_args(lanes_command)`: extras for the board assistant, such as pre-approving the `lanes` CLI.
- `parse_hook(event)` and `edited_file(event)`: if the CLI can run a command on its events, point it at `lanes hook --agent <kind>`. Then map the JSON it sends to a Lanes session id and an `AgentStatus`. Map any file the agent wrote too, for artifact attribution.
- `parse_status_line(input)` and `status_line_text(raw, snapshot)`: the same idea for a status line feed (`lanes statusline --agent <kind>`).
- `transcript_file(source)` and `read_transcript(source)`: find the conversation and convert it to `TranscriptEntry` values. Agents that keep transcripts in a database can shell out to their own export command, as OpenCode does.
- `retention_days()`: how long the agent keeps its own conversations, for the note in the Sessions view.
- `leaked_env_vars(names)`: variables a parent session of this agent leaves in Lanes' environment when Lanes is started from inside one (Claude Code's `CLAUDECODE`, `CLAUDE_CODE_*`). They are stripped from every session Lanes launches, whatever its agent.
- `is_compaction_start(event)` and `token_usage(agent_session_id)`: what `[spend]` needs. The first marks a hook event as the session continuing after compaction, so Lanes can put the ticket brief back. The second reports the session's token use for the spend ledger.

## Registering a new agent

1. Add a variant to `AgentKind` with its config name: `#[serde(rename = "…")]` and `#[value(name = "…")]`.
2. Add `agent/<name>.rs` with a unit struct implementing `AgentCli`, and return it from `agent::get`.
3. Add its executable setting to `AgentsConfig` and `BoardConfig::agent_program` in `config.rs`.
4. Add tests for `launch_args` and for parsing whatever it reports (hooks, transcript export) from fixtures.
5. Add a row to the agents table in `docs/configuration.md`, and mention the value in the starter `board.toml` and the assistant reference.

Nothing in `session.rs`, the app or the CLI needs to change. Existing sessions keep the agent that created them, so a board can switch agents at any time.

## Worked example: Codex (not implemented)

Codex isn't installed on the development machine, so this is a sketch from its documentation. It is **unverified**.

- **Session ids:** Codex assigns its own. Lanes would store its own UUID as usual, then learn Codex's id later. One option is the `notify` hook: `notify = ["lanes", "--home", "…", "hook", "--agent", "codex"]` in `~/.codex/config.toml`, or passed per launch with `-c notify=[…]`, which receives a JSON payload after each turn. The other is to scan the newest rollout file under `~/.codex/sessions/`. The adapter would write the id onto the session record, and resume with `codex resume <id>`.
- **Instructions:** Codex reads `AGENTS.md` rather than taking a system-prompt flag, so `system_prompt = false`. The stage prompt and brief go in as the first message (`codex "<prompt>"`), as with OpenCode.
- **Settings:**
  - `model` maps to `-m`.
  - `permission_mode` could map to `-a/--ask-for-approval` or `-s/--sandbox`.
  - `effort` would go through `-c model_reasoning_effort=…`.
- **Live state:** `notify` only fires when a turn completes, so it can give **your turn** but not **working** or **needs input**. Partial live state would need a small change: an adapter-level "turn finished" status.
- **Transcript:** the rollout JSONL under `~/.codex/sessions/YYYY/MM/DD/` holds the messages.
