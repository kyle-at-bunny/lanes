# `lanes` CLI reference

The `lanes` executable is both the desktop app and a command-line tool. With no arguments it opens the board; with a subcommand it runs that command and exits. Claude sessions use the CLI to save artifacts, and the board assistant uses it to operate the board.

Every command accepts `--home <dir>` (default `$LANES_HOME`, then `~/.lanes`).

## Board and tickets

```text
lanes board                                   Summarise columns, their settings, artifacts and ticket counts
lanes ticket list [--column <id>] [--json]    List tickets
lanes ticket show <ID>                        Show a ticket: column, Jira link, sessions, artifact status
lanes ticket create "<title>" [--id <ID>] [--description "<text>"] [--column <id>] [--cwd <folder>] [--column-cwd <col>=<folder>]...
lanes ticket move <ID> <column> [--force]     Move a ticket
lanes ticket edit <ID> [--title "<t>"] [--description "<d>"] [--cwd <folder>] [--column-cwd <col>=<folder>]...
lanes config check                            Validate board.toml and the prompt files it references
```

**IDs.** Without `--id`, tickets are numbered `T-001`, `T-002`, …. IDs never repeat, even across archived tickets.

**Working folders.** `--cwd` sets the folder for the ticket's new sessions in every stage, and `--column-cwd qa=<folder>` sets it for one stage; both are repeatable templates. An empty value clears the override (`--cwd ""`, `--column-cwd qa=`). `ticket show` prints where each stage's new sessions would start and which setting decided it. See [Working folders](configuration.md#working-folders).

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
lanes hook          Records a Claude Code hook event (JSON on stdin) as session state
lanes statusline    Records status-line JSON (context, plan limits); prints a status line or chains to yours
lanes launch <uuid> Runs a prepared session in the current console (used by external terminal tabs)
```

If you already have a `statusLine` command in `~/.claude/settings.json`, `lanes statusline` runs it and prints its output, so your status line still appears inside Lanes sessions.
