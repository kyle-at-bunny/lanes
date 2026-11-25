# `lanes` CLI reference

The `lanes` executable is both the desktop app and a command-line tool. With no arguments it opens the board; with a subcommand it runs that command and exits. Claude sessions use the CLI to save artifacts, and the board assistant uses it to operate the board.

Every command accepts `--home <dir>` (default `$LANES_HOME`, then `~/.lanes`).

## Board and tickets

```text
lanes board                                   Summarise columns, their settings, artifacts and ticket counts
lanes ticket list [--column <id>] [--json]    List tickets
lanes ticket show <ID>                        Show a ticket: column, work item link, sessions, artifact status
lanes ticket create "<title>" [--id <ID>] [--description "<text>"] [--column <id>] [--link <url> [--link-key <KEY>]] [--cwd <folder>] [--column-cwd <col>=<folder>]...
lanes ticket move <ID> <column> [--force]     Move a ticket
lanes ticket edit <ID> [--title "<t>"] [--description "<d>"] [--link <url> [--link-key <KEY>]] [--cwd <folder>] [--column-cwd <col>=<folder>]...
lanes config check                            Validate board.toml and the prompt files it references
lanes config schema [--write]                 Print board.toml's JSON Schema; --write saves board.schema.json and points board.toml at it
lanes config docs                             Print the configuration reference for this version of Lanes
lanes spend [--ticket <ID>]                   Token spend per ticket and stage, and the [spend] A/B comparison
```

**IDs.** Without `--id`, tickets are numbered `T-001`, `T-002`, …. IDs never repeat, even across archived tickets.

**Working folders.** `--cwd` sets the folder for the ticket's new sessions in every stage, and `--column-cwd qa=<folder>` sets it for one stage; both are repeatable templates. An empty value clears the override (`--cwd ""`, `--column-cwd qa=`). `ticket show` prints where each stage's new sessions would start and which setting decided it. See [Working folders](configuration.md#working-folders).

**Work item links.** `--link` links a ticket to its work item in any tracker, given its URL. `--link ""` removes the link. Lanes works out a short key from Jira, Azure DevOps, GitHub, GitLab and Linear URLs; for others, pass `--link-key`. Linked tickets show the key as a link on their board card and an **Open <key>** link in the ticket panel, and every stage's ticket brief includes the URL. `--jira` still works as another name for `--link`. See [Work item links](configuration.md#work-item-links).

**Editor completion.** `config schema --write` saves `board.schema.json` next to `board.toml` and adds a `#:schema ./board.schema.json` line to the top of `board.toml`. Editors that read it (e.g. VS Code with Even Better TOML) then complete and validate keys. New boards are set up this way, and Lanes rewrites the schema when a new version adds options.

**Spend.** `spend` reads Claude Code's own transcripts for every session each ticket has had, including sessions replaced by **Start fresh from summary**. It counts subagents too. Figures are input-token equivalents at API price ratios. See [`[spend]`](configuration.md#spend).

**Moving tickets.** `ticket move` refuses to move a ticket forward while its current column's required outputs are missing; `--force` moves it anyway. Moving from the CLI doesn't start the new stage's session. Open the ticket in Lanes to start or resume it.

## Artifacts

```text
lanes artifact set [--ticket <ID>] <name> "<value>"   Save a text or file artifact
lanes artifact set [--ticket <ID>] <name> -           Read the value from stdin
lanes artifact get [--ticket <ID>] <name>             Print an artifact
```

`--ticket` defaults to `$LANES_TICKET`, which Lanes sets in every session's environment.

## Internal commands

These are used by Claude Code and Lanes itself; you won't normally run them.

```text
lanes hook [--agent <agent>]        Records an agent hook event (JSON on stdin) as session state
lanes statusline [--agent <agent>]  Records status-line JSON (context, plan limits); prints a status line or chains to yours
lanes launch <uuid> Runs a prepared session in the current console (used by external terminal tabs)
```

If you already have a `statusLine` command in `~/.claude/settings.json`, `lanes statusline` runs it and prints its output, so your status line still appears inside Lanes sessions.
