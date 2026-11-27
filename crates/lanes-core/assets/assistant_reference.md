# Lanes reference (built in; always matches this version of Lanes)

## Board commands (pre-approved; run them with Bash)
- `{{ lanes }} board` summarises columns, their settings, artifacts and ticket counts. Start here.
- `{{ lanes }} ticket list [--column <id>] [--json]`
- `{{ lanes }} ticket show <id>`
- `{{ lanes }} ticket create "<title>" [--id <ID>] [--description "<text>"] [--column <id>] [--link <work item URL>] [--link-key <KEY>] [--cwd <folder>] [--column-cwd <column>=<folder>]`
- `{{ lanes }} ticket edit <id> [--title "<t>"] [--description "<d>"] [--link <work item URL>] [--link-key <KEY>] [--cwd <folder>] [--column-cwd <column>=<folder>]`
  An empty value clears `--link`, `--cwd` or a `--column-cwd` entry. Folder changes apply to new sessions only.
- `{{ lanes }} ticket move <id> <column> [--force]` refuses when the ticket is missing outputs its current
  column requires. Only use --force when the user agrees. Moving never starts a session; the user opens
  the ticket in Lanes and clicks Start session.
- `{{ lanes }} artifact get --ticket <id> <name>` / `artifact set --ticket <id> <name> "<value>"`
- `{{ lanes }} config check` validates board.toml and the prompt files it references.
- `{{ lanes }} config docs` prints the full configuration reference; `{{ lanes }} config schema` prints the JSON Schema
  for board.toml (every key, its type, default and meaning). Read them before any config edit you are unsure about.

## Work items from a tracker
Lanes has no built-in tracker integration. If you have tracker tools (for example a Jira, Azure DevOps, Linear or
GitHub MCP server), use them to read work items.
- **Create a ticket from a work item:** fetch it, then
  `{{ lanes }} ticket create "<title>" --id <KEY> --link <work item URL> --description "<description as plain text>"`.
  Use the item's key as the ticket id where it makes a valid id (e.g. PROJ-12; for numeric ids like Azure DevOps 4521,
  use a prefix such as AB-4521). Check `ticket list` first and skip items that already exist.
- **Link an existing ticket:** `{{ lanes }} ticket edit <id> --link <work item URL>`.
- Lanes works out a short key from Jira, Azure DevOps, GitHub, GitLab and Linear URLs. For any other URL add
  `--link-key <KEY>`.
- A linked ticket shows its key as a link on its board card and an "Open <KEY>" link in the ticket panel, and every
  stage's ticket brief includes the URL.
- Never change the tracker itself unless the user explicitly asks.

## Configuration (hot-reloaded; `config docs` has the full reference)
- The board is defined in `{{ board_file }}` (TOML). Lanes reloads it a moment after it is saved.
- `[[columns]]` entries appear in order. Keys: `id`, `title`, `cwd` (folder the stage's sessions run in; a
  template such as `~/worktrees/{{ '{{' }} ticket.id {{ '}}' }}`; omit for holding lanes like Backlog/Done), `prompt` (file under
  `{{ home }}/prompts/`), `consumes` and `produces` (artifact names; a trailing `?` makes an input optional),
  `model`, `effort`, `permission_mode`, `extra_args`, `start`, `reentry`, `handoff`, `cleanup`, and `[[columns.actions]]`
  buttons (`label`, `command`, optional `cwd`).
- `start` (in `[defaults]` or per column): `"wait"` (default) gives a new session the ticket brief as context and
  waits for the user's first message; `"prompt"` sends the brief as the first message so Claude starts at once.
- `[artifacts.<name>]` has `type = "file"`, `"text"` or `"html"`, a `description`, and optional `inline = true` (markdown files only). HTML artifacts are self-contained pages the user opens in their browser.
- `[defaults]` holds board-wide settings (including `agent = "claude" | "opencode"`, the CLI that runs new sessions); `[[actions]]` are buttons on every ticket.
- Column prompts are minijinja templates (`{{ '{{' }} ticket.id {{ '}}' }}`, `{{ '{{' }} ticket.title {{ '}}' }}`,
  `{{ '{{' }} ticket.link_url {{ '}}' }}`, `{{ '{{' }} ticket.link_key {{ '}}' }}`, `{{ '{{' }} column.title {{ '}}' }}`). A prompt edit applies to sessions started after it;
  existing sessions keep the prompt they were created with, even when resumed.

To add a column: insert a `[[columns]]` block at the right position, create its prompt file, then run
`{{ lanes }} config check` and fix anything it reports. Always run the check after any config edit.

## Rules
- Change tickets only through the commands above; never edit files under `tickets/` or `state/` directly.
- Never delete tickets or Claude Code session data; the user does that from the Lanes UI.
- Before removing or renaming a column that still holds tickets, or deleting artifacts, ask the user.
- Keep the user's comments and layout in board.toml intact when editing it.
