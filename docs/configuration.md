# Configuration reference

A Lanes board is a folder, `~/.lanes` by default. Use `lanes --home <dir>` or set `LANES_HOME` to use another one.

```
~/.lanes/
  board.toml              # columns, artifacts, actions, settings (this document)
  prompts/                # column prompt templates, plus assistant.md
  tickets/<ID>/           # ticket.toml, artifacts/, artifacts.toml, events.jsonl, sessions/
  tickets/_archive/<ID>/  # finished tickets
  state/                  # live session state written by Claude Code hooks (managed by Lanes)
  assistant.toml          # the board assistant's session id
```

Lanes reloads `board.toml` automatically when it changes. If the new file is invalid, the error is shown and the previous config stays active. Run `lanes config check` to validate the file from a terminal.

- [`[defaults]`](#defaults)
- [`[artifacts.<name>]`](#artifacts)
- [`[[columns]]`](#columns)
- [Prompt templates](#prompt-templates)
- [Actions and cleanup commands](#actions-and-cleanup-commands)
- [`[jira]`](#jira)

## `[defaults]`

| Key | Default | Meaning |
| --- | --- | --- |
| `claude_path` | `"claude"` | Claude Code executable. A bare name is resolved to its `.exe` on `PATH` (or `~/.local/bin`). |
| `model` | none | Default `--model` for every session (e.g. `"opus"`, `"sonnet"`). |
| `effort` | none | Default `--effort` for every session. |
| `terminal` | `"embedded"` | `"embedded"` runs sessions inside Lanes; `"external"` opens each one as a Windows Terminal tab. |
| `kickoff_inline_limit` | `6000` | First prompts longer than this many characters are written to a file, and Claude is asked to read it. |
| `theme` | `"system"` | `"system"`, `"light"` or `"dark"`. The top-bar button updates this. |
| `terminal_theme` | `"match"` | `"match"` follows the app theme; `"dark"` keeps terminals dark in light mode. |
| `cleanup_on_done` | `false` | Offer **Finish & clean up** automatically when a card lands in the last column. |
| `delete_transcripts` | `false` | Also delete Claude Code's transcript files during cleanup. They are listed and confirmed first. |

## Artifacts

Artifacts are the only thing that passes between stages. Each one is either a file or a short piece of text.

```toml
[artifacts.plan]
type = "file"              # "file" or "text"
inline = true              # files only: paste the content into the next stage's first prompt
path = "plan.md"           # files only: name inside the ticket's artifacts folder (default "<name>.md")
description = "Implementation plan: goal, approach, files to change, risks, and how to verify."

[artifacts.branch]
type = "text"
description = "Repository path and branch name holding the implementation, plus the PR URL if one was opened."
```

The `description` is shown to both sides: the producing stage is told what to write, and the consuming stage is told what it is reading.

- **File artifacts** live at `tickets/<ID>/artifacts/<path>`. The producing session is given the absolute path to write.
- **Text artifacts** are saved with `lanes artifact set --ticket <ID> <name> "<value>"`. Claude is given the exact command.
- **Optional inputs:** in `consumes`, a trailing `?` (e.g. `"review_notes?"`) marks the input as optional. Missing optional inputs are simply left out of the prompt.
- **Missing outputs:** moving a ticket forward when any non-optional `produces` artifact is missing asks for confirmation. It never blocks.

## `[[columns]]`

Columns appear on the board in the order they are listed. A column **with** a `cwd` runs a Claude session per ticket; a column **without** one (Backlog, Done) is a plain holding lane.

```toml
[[columns]]
id = "review"
title = "Review"
cwd = "~/projects/my-app"
prompt = "prompts/review.md"
consumes = ["plan", "branch"]
produces = ["review_notes"]
model = "opus"
permission_mode = "plan"
```

| Key | Meaning |
| --- | --- |
| `id` | Unique identifier, used in ticket files and the CLI. Changing it orphans tickets in that column. |
| `title` | Display name (defaults to `id`). |
| `cwd` | Folder the stage's sessions run in. `~` is expanded and templates work, e.g. `"~/worktrees/{{ ticket.id \| lower }}"`. Omit for holding lanes. Tickets can override it; see [Working folders](#working-folders). |
| `prompt` | Prompt template, relative to the board folder. Appended to Claude's system prompt when the session is created. |
| `consumes` / `produces` | Artifact names; see [Artifacts](#artifacts). |
| `model`, `effort` | Override the defaults for this stage. |
| `permission_mode` | Claude Code permission mode for this stage, e.g. `"plan"` or `"acceptEdits"`. |
| `extra_args` | Extra `claude` arguments, e.g. `["--add-dir", "~/projects/shared"]`. |
| `reentry` | Template for the message sent when a ticket returns to this stage with changed inputs. |
| `handoff` | Template for the message the **Request handoff** button sends, asking the session to write its outputs. |
| `cleanup` | Command run during **Finish & clean up** in this stage's session folder. |
| `[[columns.actions]]` | Action buttons shown on tickets in this column. |

Model, effort, permission mode and extra arguments are passed again on every launch, including resumes. The prompt is fixed when a session is created: editing a column's prompt affects new sessions only.

## Working folders

When a ticket's session for a stage is first created, Lanes picks its folder from the most specific setting that exists:

1. **The ticket's folder for this stage** (`[column_cwd]` in `ticket.toml`).
2. **The ticket's folder for all stages** (`cwd` in `ticket.toml`).
3. **The column's `cwd`** in `board.toml`.

All three are templates, using `ticket.id`, `ticket.title`, `ticket.jira_key`, `ticket.jira_url`, `column.id` and `column.title`, and `~` is expanded. The folder must exist when the session starts.

Set ticket folders in three places:
- the ticket panel ("New sessions start in … **Change**", then **All stages** or **<Stage> only**);
- the **New ticket** form;
- the CLI: `lanes ticket edit <ID> --cwd <folder> --column-cwd qa=<folder>`.

**Use column default** removes a ticket's overrides.

A session keeps the folder it was created in, even when its ticket's folders change later, because Claude Code ties a session's history to its folder. The ticket panel warns when the current stage's existing session runs somewhere other than the folder new sessions would use.

A typical per-ticket worktree setup:

```toml
[[columns]]
id = "dev"
cwd = "~/worktrees/{{ ticket.id | lower }}"
# cd out of the worktree first: Windows won't remove a folder a process is running in
cleanup = "cd /d %USERPROFILE% && git -C %USERPROFILE%/projects/my-app worktree remove --force {{ cwd }}"

[[columns.actions]]
label = "Create worktree"
command = "git -C %USERPROFILE%/projects/my-app worktree add -b {{ ticket.id | lower }} {{ cwd }}"
cwd = "~"
```

Commands run through `cmd.exe`, so use `%USERPROFILE%` rather than `~` inside them. `~` is only expanded in `cwd` settings.

## Prompt templates

Prompts are [minijinja](https://docs.rs/minijinja) templates. Lanes adds two generated sections of its own:
- **"Stage outputs"** goes into the system prompt. It lists each artifact the stage produces and how to write it.
- **"Inputs from earlier stages"** goes into the first prompt. It lists each artifact the stage consumes, inlined or by path.

| Variable | Available in | Value |
| --- | --- | --- |
| `ticket.id`, `ticket.title`, `ticket.description` | prompts, `reentry`, `handoff`, actions | The ticket |
| `ticket.jira_url` | prompts, `reentry`, `handoff`, actions | The linked Jira issue URL, if any |
| `column.id`, `column.title` | prompts, `reentry`, `handoff`, actions | The current column |
| `from` | `reentry` | Title of the column the ticket came back from |
| `changed` | `reentry` | List of input artifact names that changed |
| `cwd` | actions, `cleanup` | The folder the command runs in |
| `artifacts.<name>` | actions, `cleanup` | Text artifacts by value, file artifacts by path |

Example `prompts/dev.md`:

```jinja
You are in the DEVELOPMENT stage for ticket {{ ticket.id }}: {{ ticket.title }}.
{% if ticket.jira_url %}The Jira issue is {{ ticket.jira_url }}.{% endif %}

Implement the agreed plan on a dedicated branch. If review notes are present, address them first.
```

## Actions and cleanup commands

Actions are buttons on the ticket panel. They run through `cmd.exe`, without a window, in the ticket's session folder for its current column. A notification shows the result and the last lines of output.

```toml
# Shown on every ticket
[[actions]]
label = "Open in VS Code"
command = "code ."

# Shown on tickets in the dev column (place directly after that [[columns]] entry)
[[columns.actions]]
label = "Git status"
command = "git status --short --branch"
cwd = "{{ cwd }}"          # optional; a template like the command
```

A column's `cleanup` command uses the same templates, and runs during **Finish & clean up** for every column the ticket had a session in:

```toml
[[columns]]
id = "dev"
cwd = "~/projects/my-app"
cleanup = "git worktree remove --force ../wt-{{ ticket.id }}"
```

## `[jira]`

Enables **From Jira**, which imports issues as tickets. Lanes only reads from Jira and never changes issues.

```toml
[jira]
site = "https://your-site.atlassian.net"
email = "you@example.com"
token_env = "LANES_JIRA_TOKEN"     # name of the env var holding an API token (default shown)
open_issues_jql = "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC"
```

Create an API token at <https://id.atlassian.com/manage-profile/security/api-tokens>, store it in the environment variable named by `token_env`, then restart Lanes. The token is never written to disk by Lanes.

Imported tickets:
- use the issue key as their ID and the summary as their title;
- get the description converted to plain text;
- show an **Open in Jira** link.

The first prompt in each stage includes `Jira: <url>`. Issues that already exist as tickets, on the board or archived, are skipped.
