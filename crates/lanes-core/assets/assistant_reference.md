# Lanes reference (built in; always matches this version of Lanes)

## Board commands (pre-approved; run them with Bash)
- `{{ lanes }} board` summarises columns, their settings, artifacts and ticket counts. Start here.
- `{{ lanes }} ticket list [--column <id>] [--json]`
- `{{ lanes }} ticket show <id>`
- `{{ lanes }} ticket create "<title>" [--id <ID>] [--description "<text>"] [--column <id>] [--jira <url-or-key>] [--cwd <folder>] [--column-cwd <column>=<folder>]`
- `{{ lanes }} ticket edit <id> [--title "<t>"] [--description "<d>"] [--jira <url-or-key>] [--cwd <folder>] [--column-cwd <column>=<folder>]`
  An empty value clears `--jira`, `--cwd` or a `--column-cwd` entry. Folder changes apply to new sessions only.
- `{{ lanes }} ticket move <id> <column> [--force]` refuses when the ticket is missing outputs its current
  column requires. Only use --force when the user agrees. Moving never starts a session; the user opens
  the ticket in Lanes and clicks Start session.
- `{{ lanes }} artifact get --ticket <id> <name>` / `artifact set --ticket <id> <name> "<value>"`
- `{{ lanes }} config check` validates board.toml and the prompt files it references.
- `{{ lanes }} config docs` prints the full configuration reference; `{{ lanes }} config schema` prints the JSON Schema
  for board.toml (every key, its type, default and meaning). Read them before any config edit you are unsure about.

## Jira
If you have Jira tools (for example an Atlassian MCP server), use them to read issues; Lanes doesn't need
its own API token for this.
- **Create a ticket from an issue:** fetch it, then
  `{{ lanes }} ticket create "<summary>" --id <KEY> --jira <browse URL> --description "<description as plain text>"`.
  Use the issue key as the ticket id. Check `ticket list` first and skip issues that already exist.
- **Link an existing ticket:** `{{ lanes }} ticket edit <id> --jira <browse URL>`.
- A linked ticket shows its issue key as a link on its board card and an "Open in Jira" link in the ticket panel,
  and every stage's ticket brief includes the URL.
- Use the browse URL (`https://<site>/browse/<KEY>`). A bare key only works when `[jira] site` is set in board.toml.
- Never change Jira itself unless the user explicitly asks.

## Configuration (hot-reloaded; `config docs` has the full reference)
- The board is defined in `{{ board_file }}` (TOML). Lanes reloads it a moment after it is saved.
- `[[columns]]` entries appear in order. Keys: `id`, `title`, `cwd` (folder the stage's sessions run in; a
  template such as `~/worktrees/{{ '{{' }} ticket.id {{ '}}' }}`; omit for holding lanes like Backlog/Done), `prompt` (file under
  `{{ home }}/prompts/`), `consumes` and `produces` (artifact names; a trailing `?` makes an input optional),
  `model`, `effort`, `permission_mode`, `extra_args`, `start`, `reentry`, `handoff`, `cleanup`, and `[[columns.actions]]`
  buttons (`label`, `command`, optional `cwd`).
- `start` (in `[defaults]` or per column): `"wait"` (default) gives a new session the ticket brief as context and
  waits for the user's first message; `"prompt"` sends the brief as the first message so Claude starts at once.
- `[artifacts.<name>]` has `type = "file"` or `"text"`, a `description`, and optional `inline = true`.
- `[defaults]` holds board-wide settings; `[[actions]]` are buttons on every ticket; `[jira]` has `site`
  (and `email`, only for Lanes' own API import).
- Column prompts are minijinja templates (`{{ '{{' }} ticket.id {{ '}}' }}`, `{{ '{{' }} ticket.title {{ '}}' }}`,
  `{{ '{{' }} ticket.jira_url {{ '}}' }}`, `{{ '{{' }} column.title {{ '}}' }}`). A prompt edit applies to sessions started after it;
  existing sessions keep the prompt they were created with, even when resumed.

To add a column: insert a `[[columns]]` block at the right position, create its prompt file, then run
`{{ lanes }} config check` and fix anything it reports. Always run the check after any config edit.

## Rules
- Change tickets only through the commands above; never edit files under `tickets/` or `state/` directly.
- Never delete tickets or Claude Code session data; the user does that from the Lanes UI.
- Before removing or renaming a column that still holds tickets, or deleting artifacts, ask the user.
- Keep the user's comments and layout in board.toml intact when editing it.
