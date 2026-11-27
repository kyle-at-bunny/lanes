# Configuration reference

A Lanes board is a folder, `~/.lanes` by default. Use `lanes --home <dir>` or set `LANES_HOME` to use another one.

```
~/.lanes/
  board.toml              # columns, artifacts, actions, settings (this document)
  board.schema.json       # JSON Schema for board.toml, for editor completion (managed by Lanes)
  prompts/                # column prompt templates, plus assistant.md
  tickets/<ID>/           # ticket.toml, artifacts/, artifacts.toml, events.jsonl, sessions/
  tickets/_archive/<ID>/  # finished tickets
  state/                  # live session state written by Claude Code hooks (managed by Lanes)
  assistant.toml          # the board assistant's session id
```

Lanes reloads `board.toml` automatically when it changes. If the new file is invalid, the error is shown and the previous config stays active. Run `lanes config check` to validate the file from a terminal, `lanes config schema --write` for completion in your editor (see [CLI](cli.md)), and `lanes config docs` to print this reference.

- [`[defaults]`](#defaults)
- [`[artifacts.<name>]`](#artifacts)
- [`[[columns]]`](#columns)
- [Prompt templates](#prompt-templates)
- [Agents](#agents)
- [Actions and cleanup commands](#actions-and-cleanup-commands)
- [Work item links](#work-item-links)

## `[defaults]`

| Key | Default | Meaning |
| --- | --- | --- |
| `agent` | `"claude"` | The coding agent CLI that runs new sessions. Sessions that already exist keep the agent that created them. See [Agents](#agents). |
| `claude_path` | `"claude"` | Claude Code executable; `[agents.claude] path` does the same and wins when both are set. A bare name is looked up on `PATH` (or `~/.local/bin`). |
| `model` | none | Default `--model` for every session (e.g. `"opus"`, `"sonnet"`). |
| `effort` | none | Default `--effort` for every session. |
| `terminal` | `"embedded"` | `"embedded"` runs sessions inside Lanes; `"external"` opens each one in an external terminal: a Windows Terminal tab, Terminal.app on macOS, or `$TERMINAL` (else `x-terminal-emulator`) on Linux. |
| `start` | `"wait"` | How a new session starts. `"wait"` puts the ticket brief (title, description, work item link, inputs from earlier stages) into the system prompt and opens Claude on an empty input box, so you set the first task. `"prompt"` sends the brief as the first message, so Claude starts working straight away. Columns can override it. |
| `kickoff_inline_limit` | `6000` | With `start = "prompt"`, briefs longer than this many characters are written to a file, and Claude is asked to read it. |
| `theme` | `"system"` | `"system"`, `"light"` or `"dark"`. The top-bar button updates this. |
| `terminal_theme` | `"match"` | `"match"` follows the app theme; `"dark"` keeps terminals dark in light mode. |
| `pr_provider` | `"github"` | Where pull request status comes from: `"github"` (the GitHub CLI, `gh`) or `"none"` (PR links stay plain links). See [Pull request status](#in-the-app). |
| `cleanup_on_done` | `false` | Offer **Finish & clean up** automatically when a card lands in the last column. |

## Artifacts

Artifacts are the only thing that passes between stages. Each one is a markdown file, a short piece of text, or an HTML page.

```toml
[artifacts.plan]
type = "file"              # "file", "text" or "html"
inline = true              # markdown files only: paste the content into the next stage's first prompt
path = "plan.md"           # file and html only: name inside the ticket's artifacts folder (default "<name>.md" or "<name>.html")
description = "Implementation plan: goal, approach, files to change, risks, and how to verify."

[artifacts.branch]
type = "text"
description = "One line per repository, exactly in this format: `<absolute repo path> | <branch> | <PR URL or \"no PR\">`. No other text."

[artifacts.mockup]
type = "html"
description = "A clickable mockup of the new settings page."
```

The `description` is shown to both sides: the producing stage is told what to write, and the consuming stage is told what it is reading.

- **File artifacts** live at `tickets/<ID>/artifacts/<path>`. The producing session is given the absolute path to write.
- **Text artifacts** are saved with `lanes artifact set --ticket <ID> <name> "<value>"`. Claude is given the exact command.
- **HTML artifacts** live at `tickets/<ID>/artifacts/<path>` like file artifacts. The producing session is asked for a single self-contained page (inline CSS and scripts), and later stages are given its path, never its content (`inline` is ignored). Use them for mockups, reports and anything better viewed than read.
- **Optional inputs:** in `consumes`, a trailing `?` (e.g. `"review_notes?"`) marks the input as optional. Missing optional inputs are simply left out of the prompt.
- **Missing outputs:** moving a ticket forward when any non-optional `produces` artifact is missing asks for confirmation. It never blocks.

### In the app

The ticket panel shows a one-line summary (e.g. "Inputs 2/3 · Outputs 0/1") and flags missing required inputs and outputs. The **Artifacts** tab shows them in full, grouped into **Inputs** (consumed by the current column), **Outputs** (produced by it; missing required ones are highlighted) and a collapsed **Other**.

- **Provenance:** each artifact shows who last wrote it and when, e.g. "written by Dev · 5m ago". `lanes artifact set` records the stage it ran in, and a session's own edits to a file artifact are recorded through its hooks. A file changed outside any session shows "updated … ago".
- **Text artifacts** are shown in full, with **Edit** and **Clear**. **File artifacts** are rendered as markdown, with **Open** for your editor. **HTML artifacts** show their size, with **Open in browser** (the program Windows opens `.html` files with, normally your default browser) and **Open folder**.
- **Repository rows:** a line in the form `<absolute repo path> | <branch> | <PR URL or "no PR">` is shown as a row with an **Open folder** button, a branch chip and a clickable PR link. The starter `branch` artifact asks for exactly this format. Other lines are shown as text.
- **Links:** URLs open in the browser, and local paths (`C:\…` and `\\server\…` on Windows, `/…` elsewhere, and `~/…`) get **Open folder**, plus **Open** for files. Both use whatever program your OS has set for that file type.
- **Pull request status:** with `pr_provider = "github"` (the default), pull request links (`https://<host>/<owner>/<repo>/pull/<n>`) in any artifact are checked with the [GitHub CLI](https://cli.github.com) (`gh pr view`), so `gh` must be installed and logged in. Set `pr_provider = "none"` to turn this off. Each link gets a chip such as "open · checks failing", "draft" or "merged". While a PR is open it also gets its review decision ("review needed", "approved" or "changes requested") and a count of unresolved review comments, fetched with `gh api graphql`. The ticket panel lists every PR under the artifacts line as a short link (e.g. `app#12`) with the same chips. Cards get a row of chips under the title, like "2 PRs · 1 failing", "changes requested", "3 unresolved", which wrap on narrow cards and turn red for failing checks or requested changes, and **Finish & clean up** warns about PRs that are still open. Open PRs are rechecked every minute, merged and closed ones every 10 minutes. If `gh` fails, the link shows "status unavailable".

## Agents

Each session runs a coding agent CLI in a terminal. `[defaults] agent` picks which one runs new sessions; the board assistant uses it too. Changing it doesn't affect existing sessions: a stage that already has a session always resumes it with the agent that created it.

| Agent | `agent` | Live status (working / needs input) | Context and plan usage | Transcript tab |
| --- | --- | --- | --- | --- |
| Claude Code | `"claude"` | yes, from hooks | yes, from the status line | yes |

Set an agent's executable under `[agents.<agent>]`:

```toml
[agents.claude]
path = "C:/tools/claude.exe"
```

`model`, `effort`, `permission_mode` and `extra_args` are passed in the agent's own form. [docs/agents.md](agents.md) explains how to add another agent.

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
| `start` | `"wait"` or `"prompt"`; overrides `[defaults] start` for this stage (e.g. `"prompt"` for a QA stage that should just run). |
| `reentry` | Template for the message used when a ticket returns to this stage with changed inputs. With `start = "wait"` it is typed into Claude's input box without being sent, so you can edit it first. In external terminals, and with `"prompt"`, it is sent. |
| `handoff` | Template for the message the **Request handoff** button sends, asking the session to write its outputs. |
| `cleanup` | Command run during **Finish & clean up** in this stage's session folder. |
| `[[columns.actions]]` | Action buttons shown on tickets in this column. |

Model, effort, permission mode and extra arguments are passed again on every launch, including resumes. The prompt is fixed when a session is created: editing a column's prompt affects new sessions only.

## Working folders

When a ticket's session for a stage is first created, Lanes picks its folder from the most specific setting that exists:

1. **The ticket's folder for this stage** (`[column_cwd]` in `ticket.toml`).
2. **The ticket's folder for all stages** (`cwd` in `ticket.toml`).
3. **The column's `cwd`** in `board.toml`.

All three are templates, using `ticket.id`, `ticket.title`, `ticket.link_key`, `ticket.link_url`, `column.id` and `column.title`, and `~` is expanded. The folder must exist when the session starts.

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

Commands run through the system shell: `cmd.exe` on Windows (so use `%USERPROFILE%` rather than `~`), `sh` elsewhere. `~` in `cwd` settings is always expanded.

## Prompt templates

Prompts are [minijinja](https://docs.rs/minijinja) templates. Lanes adds two generated sections of its own:
- **"Stage outputs"** goes into the system prompt. It lists each artifact the stage produces and how to write it.
- **"Inputs from earlier stages"** is part of the ticket brief. It lists each artifact the stage consumes, inlined or by path. With `start = "wait"` the brief goes into the system prompt, under "Ticket brief"; with `"prompt"` it is the first message.

| Variable | Available in | Value |
| --- | --- | --- |
| `ticket.id`, `ticket.title`, `ticket.description` | prompts, `reentry`, `handoff`, actions | The ticket |
| `ticket.link_url`, `ticket.link_key` | prompts, `reentry`, `handoff`, actions | The linked work item's URL and key, if any (`ticket.jira_url` and `ticket.jira_key` still work) |
| `column.id`, `column.title` | prompts, `reentry`, `handoff`, actions | The current column |
| `from` | `reentry` | Title of the column the ticket came back from |
| `changed` | `reentry` | List of input artifact names that changed |
| `cwd` | actions, `cleanup` | The folder the command runs in |
| `artifacts.<name>` | actions, `cleanup` | Text artifacts by value, file artifacts by path |

Example `prompts/dev.md`:

```jinja
You are in the DEVELOPMENT stage for ticket {{ ticket.id }}: {{ ticket.title }}.
{% if ticket.link_url %}The work item is {{ ticket.link_url }}.{% endif %}

Implement the agreed plan on a dedicated branch. If review notes are present, address them first.
```

## Actions and cleanup commands

Actions are buttons on the ticket panel. They run through the system shell (`cmd.exe` on Windows, `sh` elsewhere), without a window, in the ticket's session folder for its current column. A notification shows the result and the last lines of output.

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

## Work item links

A ticket can link to its work item in whatever tracker you use: Jira, Azure DevOps, GitHub or GitLab issues, Linear, or anything else with a URL. Lanes has no tracker integration of its own and never talks to a tracker. It stores the link and shows it:
- the key as a link on the board card, and **Open <key>** in the ticket panel;
- `Work item <key>: <url>` in every stage's ticket brief;
- `ticket.link_url` and `ticket.link_key` in templates.

Set it with `lanes ticket create … --link <url>` or `lanes ticket edit <ID> --link <url>` (`--link ""` removes it). The key is worked out from Jira (`/browse/PROJ-12`), Azure DevOps (`/_workitems/edit/4521` or `?workitem=4521`), GitHub and GitLab (`/issues/12`) and Linear (`/issue/ENG-42`) URLs. For anything else, add `--link-key <KEY>`.

**Importing work items.** Give Claude Code your tracker's MCP server (for example Atlassian's for Jira, or Azure DevOps'), then ask the **Assistant**:
- "import my open work items";
- "create a ticket for PROJ-123";
- "link T-007 to https://dev.azure.com/org/project/_workitems/edit/4521".

It reads the items with its own tools, then runs `lanes ticket create … --link <url>` or `lanes ticket edit … --link <url>`. Lanes needs no token.

Boards and tickets from earlier versions keep working: a `[jira]` section in `board.toml` is ignored, and tickets linked to Jira issues load as work item links.
