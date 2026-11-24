use std::collections::BTreeSet;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Sizable, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::artifact_view::{self, ArtifactEntry, ArtifactPanel, BranchRow, ContentLine, Segment};
use lanes_core::config::ArtifactType;
use lanes_core::paths::{expand_tilde, now_unix};
use lanes_core::ticket::Ticket;
use lanes_core::timeline;

use super::{DetailTab, LanesApp, open_in_shell};

pub(super) struct ArtifactEdit {
    ticket_id: String,
    name: String,
    input: Entity<TextareaState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Input,
    Output,
    Other,
}

impl LanesApp {
    fn artifact_panel(&self, ticket: &Ticket) -> anyhow::Result<ArtifactPanel> {
        ArtifactPanel::load(&self.board.home, &self.board.config, ticket)
    }

    fn start_artifact_edit(&mut self, ticket_id: &str, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.board.artifacts(ticket_id).read(name).ok().flatten().unwrap_or_default();
        let input = cx.new(|cx| TextareaState::new(window, cx).rows(6).default_value(current));
        input.update(cx, |state, cx| state.focus(window, cx));
        self.artifact_edit = Some(ArtifactEdit { ticket_id: ticket_id.to_string(), name: name.to_string(), input });
        cx.notify();
    }

    fn save_artifact_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.artifact_edit.take() else { return };
        let value = edit.input.read(cx).value().to_string();
        self.write_artifact(&edit.ticket_id, &edit.name, &value, window, cx);
    }

    fn write_artifact(&mut self, ticket_id: &str, name: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.board.artifacts(ticket_id).write_by(name, value, Some(timeline::USER)) {
            self.notify_error(&err, window, cx);
        }
        cx.notify();
    }

    /// One line under the ticket header: how complete this stage's inputs and outputs are, and what is missing.
    pub(super) fn render_artifact_summary(&self, ticket: &Ticket, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.board.config.artifacts.is_empty() {
            return None;
        }
        let panel = self.artifact_panel(ticket).ok()?;
        let theme = cx.theme().clone();
        let missing_inputs = panel.inputs.iter().filter(|e| e.is_missing_required()).map(|e| format!("needs {}", e.name));
        let missing_outputs = panel.outputs.iter().filter(|e| e.is_missing_required()).map(|e| format!("{} to produce", e.name));
        let warnings: Vec<_> = missing_inputs.chain(missing_outputs).map(|label| warning_tag(label, &theme)).collect();
        let summary = panel.summary();
        let ticket_id = ticket.id.clone();
        Some(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .pt_2()
                .text_xs()
                .child(div().text_color(theme.muted_foreground).child("Artifacts"))
                .child(div().child(if summary.is_empty() { "none used in this stage".to_string() } else { summary }))
                .children(warnings)
                .child(Button::new("view-artifacts").label("View").ghost().xsmall().on_click(cx.listener(move |this, _, _, cx| {
                    this.show_tab(&ticket_id, DetailTab::Artifacts, cx)
                })))
                .into_any_element(),
        )
    }

    pub(super) fn render_artifacts_tab(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = match self.artifact_panel(ticket) {
            Ok(panel) => self.render_artifact_groups(ticket, &panel, cx).into_any_element(),
            Err(err) => div().text_sm().text_color(theme.danger).child(format!("{err:#}")).into_any_element(),
        };
        v_flex().id("artifacts-tab").flex_1().min_h_0().p_3().gap_3().overflow_y_scroll().child(body)
    }

    fn render_artifact_groups(&self, ticket: &Ticket, panel: &ArtifactPanel, cx: &mut Context<Self>) -> impl IntoElement {
        let column = self.board.column_title(&ticket.column);
        let inputs = self.render_artifact_group(ticket, &format!("Inputs · consumed by {column}"), &panel.inputs, Role::Input, cx);
        let outputs = self.render_artifact_group(ticket, &format!("Outputs · produced by {column}"), &panel.outputs, Role::Output, cx);
        let other = self.render_other_artifacts(ticket, &panel.other, cx);
        let empty = panel.inputs.is_empty() && panel.outputs.is_empty() && panel.other.is_empty();
        v_flex()
            .gap_3()
            .when(empty, |this| this.child(div().text_sm().text_color(cx.theme().muted_foreground).child("This board defines no artifacts.")))
            .when_some(inputs, |this, group| this.child(group))
            .when_some(outputs, |this, group| this.child(group))
            .when_some(other, |this, group| this.child(group))
    }

    fn render_artifact_group(&self, ticket: &Ticket, title: &str, entries: &[ArtifactEntry], role: Role, cx: &mut Context<Self>) -> Option<AnyElement> {
        if entries.is_empty() {
            return None;
        }
        let cards: Vec<_> = entries.iter().map(|entry| self.render_artifact_card(ticket, entry, role, cx)).collect();
        Some(v_flex().gap_2().child(group_heading(title, cx.theme())).children(cards).into_any_element())
    }

    fn render_other_artifacts(&self, ticket: &Ticket, entries: &[ArtifactEntry], cx: &mut Context<Self>) -> Option<AnyElement> {
        if entries.is_empty() {
            return None;
        }
        let expanded = self.show_other_artifacts;
        let toggle = Button::new("toggle-other-artifacts")
            .label(format!("{} Other ({})", if expanded { "▾" } else { "▸" }, entries.len()))
            .ghost()
            .xsmall()
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_other_artifacts = !this.show_other_artifacts;
                cx.notify();
            }));
        let cards: Vec<_> = match expanded {
            true => entries.iter().map(|entry| self.render_artifact_card(ticket, entry, Role::Other, cx)).collect(),
            false => Vec::new(),
        };
        Some(v_flex().gap_2().child(h_flex().child(toggle)).children(cards).into_any_element())
    }

    fn render_artifact_card(&self, ticket: &Ticket, entry: &ArtifactEntry, role: Role, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.artifact_edit.as_ref().filter(|e| e.ticket_id == ticket.id && e.name == entry.name);
        let theme = cx.theme().clone();
        let highlight = role == Role::Output && entry.is_missing_required();
        let content = match (editing, &entry.content, entry.kind) {
            (Some(edit), _, _) => self.render_artifact_editor(edit, cx).into_any_element(),
            (None, None, _) => div().text_xs().text_color(theme.muted_foreground).child("Not produced yet.").into_any_element(),
            (None, Some(text), ArtifactType::Text) => render_content_lines(&format!("{}-{}", ticket.id, entry.name), text, &theme),
            (None, Some(text), ArtifactType::File) => render_file_content(&format!("{}-{}", ticket.id, entry.name), text, &theme),
        };
        v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(if highlight { theme.warning } else { theme.border })
            .when(highlight, |this| this.bg(theme.warning.opacity(0.12)))
            .child(self.render_artifact_card_header(ticket, entry, role, editing.is_some(), cx))
            .when(!entry.description.is_empty(), |this| this.child(div().text_xs().text_color(theme.muted_foreground).child(entry.description.clone())))
            .child(content)
            .into_any_element()
    }

    fn render_artifact_card_header(&self, ticket: &Ticket, entry: &ArtifactEntry, role: Role, editing: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let provenance = entry.provenance.as_ref().map(|p| p.describe(&self.board.config, now_unix()));
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .text_xs()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(entry.name.clone()))
            .when_some(status_tag(entry, role, &theme), |this, tag| this.child(tag))
            .when_some(provenance, |this, text| this.child(div().text_color(theme.muted_foreground).child(text)))
            .child(div().flex_1())
            .when(!editing, |this| this.children(self.artifact_buttons(ticket, entry, cx)))
    }

    fn artifact_buttons(&self, ticket: &Ticket, entry: &ArtifactEntry, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let id = |action: &str| SharedString::from(format!("{action}-artifact-{}", entry.name));
        let (ticket_id, name) = (ticket.id.clone(), entry.name.clone());
        let mut buttons = Vec::new();
        match (&entry.path, entry.kind) {
            (Some(path), ArtifactType::File) => {
                let path = path.clone();
                buttons.push(
                    Button::new(id("open"))
                        .label("Open")
                        .ghost()
                        .xsmall()
                        .tooltip("Open in your editor")
                        .on_click(move |_, _, _| {
                            if !path.exists() {
                                let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
                                let _ = std::fs::write(&path, "");
                            }
                            open_in_shell(&path)
                        })
                        .into_any_element(),
                );
            }
            _ => {
                let (edit_ticket, edit_name) = (ticket_id.clone(), name.clone());
                buttons.push(
                    Button::new(id("edit"))
                        .label("Edit")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, window, cx| this.start_artifact_edit(&edit_ticket, &edit_name, window, cx)))
                        .into_any_element(),
                );
                if entry.is_present() {
                    buttons.push(
                        Button::new(id("clear"))
                            .label("Clear")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| this.write_artifact(&ticket_id, &name, "", window, cx)))
                            .into_any_element(),
                    );
                }
            }
        }
        buttons
    }

    fn render_artifact_editor(&self, edit: &ArtifactEdit, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(Textarea::new(&edit.input))
            .child(
                h_flex()
                    .gap_1()
                    .child(Button::new("save-artifact").label("Save").primary().xsmall().on_click(cx.listener(|this, _, window, cx| this.save_artifact_edit(window, cx))))
                    .child(Button::new("cancel-artifact").label("Cancel").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                        this.artifact_edit = None;
                        cx.notify();
                    }))),
            )
    }
}

fn group_heading(title: &str, theme: &Theme) -> impl IntoElement {
    div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(theme.muted_foreground).child(title.to_string())
}

fn status_tag(entry: &ArtifactEntry, role: Role, theme: &Theme) -> Option<AnyElement> {
    match (role, entry.is_present(), entry.required) {
        (_, true, _) | (Role::Other, false, _) => None,
        (Role::Input, false, true) => Some(warning_tag("missing", theme)),
        (Role::Output, false, true) => Some(warning_tag("not produced yet", theme)),
        (Role::Input | Role::Output, false, false) => Some(Tag::secondary().xsmall().outline().child("optional").into_any_element()),
    }
}

/// Text artifacts: `branch`-format lines become repository rows, everything else is text with clickable links and paths.
fn render_content_lines(id: &str, content: &str, theme: &Theme) -> AnyElement {
    let lines: Vec<_> = artifact_view::parse_lines(content)
        .into_iter()
        .enumerate()
        .map(|(index, line)| match line {
            ContentLine::Branch(row) => render_branch_row(&format!("{id}-{index}"), row, theme),
            ContentLine::Text(text) => render_segments(&format!("{id}-{index}"), &text, theme),
        })
        .collect();
    v_flex().gap_1().text_xs().children(lines).into_any_element()
}

fn render_branch_row(id: &str, row: BranchRow, theme: &Theme) -> AnyElement {
    let pr = match row.pr {
        Some(url) => link_button(&format!("{id}-pr"), &url).into_any_element(),
        None => div().text_color(theme.muted_foreground).child("no PR").into_any_element(),
    };
    h_flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(path_label(&row.repo, theme))
        .children(path_buttons(id, &row.repo))
        .child(Tag::secondary().xsmall().child(row.branch))
        .child(pr)
        .into_any_element()
}

fn render_segments(id: &str, text: &str, theme: &Theme) -> AnyElement {
    if text.trim().is_empty() {
        return div().h(px(6.)).into_any_element();
    }
    let pieces: Vec<AnyElement> = artifact_view::segments(text)
        .into_iter()
        .enumerate()
        .flat_map(|(index, segment)| match segment {
            Segment::Text(text) => vec![div().child(text).into_any_element()],
            Segment::Url(url) => vec![link_button(&format!("{id}-{index}"), &url).into_any_element()],
            Segment::Path(path) => path_with_buttons(&format!("{id}-{index}"), &path, theme),
        })
        .collect();
    h_flex().flex_wrap().items_center().gap_x_1().children(pieces).into_any_element()
}

/// File artifacts render as markdown; the URLs and paths they mention are listed below with buttons.
fn render_file_content(id: &str, content: &str, theme: &Theme) -> AnyElement {
    let mut seen = BTreeSet::new();
    let links: Vec<_> = content
        .lines()
        .flat_map(artifact_view::segments)
        .filter(|segment| !matches!(segment, Segment::Text(_)) && seen.insert(segment.clone()))
        .enumerate()
        .flat_map(|(index, segment)| match segment {
            Segment::Url(url) => vec![link_button(&format!("{id}-link-{index}"), &url).into_any_element()],
            Segment::Path(path) => path_with_buttons(&format!("{id}-link-{index}"), &path, theme),
            Segment::Text(_) => Vec::new(),
        })
        .collect();
    v_flex()
        .gap_2()
        .child(div().text_sm().child(TextView::markdown(SharedString::from(format!("{id}-markdown")), content.to_string()).selectable(true)))
        .when(!links.is_empty(), |this| {
            this.child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .child(div().text_color(theme.muted_foreground).child("Links"))
                    .children(links),
            )
        })
        .into_any_element()
}

fn link_button(id: &str, url: &str) -> impl IntoElement {
    let target = url.to_string();
    Button::new(SharedString::from(format!("url-{id}"))).label(url.to_string()).link().xsmall().tooltip("Open in browser").on_click(move |_, _, cx| cx.open_url(&target))
}

fn path_with_buttons(id: &str, path: &str, theme: &Theme) -> Vec<AnyElement> {
    let mut pieces = vec![path_label(path, theme)];
    pieces.extend(path_buttons(id, path));
    pieces
}

const PATH_DISPLAY_CHARS: usize = 48;

fn path_label(path: &str, theme: &Theme) -> AnyElement {
    div().font_family(theme.mono_font_family.clone()).child(artifact_view::display_path(path, PATH_DISPLAY_CHARS)).into_any_element()
}

/// Amber text on a faint amber fill: the stock warning tag's light text is unreadable on its fill.
fn warning_tag(label: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    Tag::custom(theme.warning.opacity(0.15), theme.warning, theme.warning.opacity(0.6)).xsmall().child(label.into()).into_any_element()
}

fn path_buttons(id: &str, path: &str) -> Vec<AnyElement> {
    let (folder, code) = (path.to_string(), path.to_string());
    vec![
        Button::new(SharedString::from(format!("folder-{id}")))
            .label("Open folder")
            .ghost()
            .xsmall()
            .on_click(move |_, _, _| open_in_shell(&artifact_view::folder_for(&folder)))
            .into_any_element(),
        Button::new(SharedString::from(format!("code-{id}"))).label("VS Code").ghost().xsmall().on_click(move |_, _, _| open_in_vscode(&code)).into_any_element(),
    ]
}

fn open_in_vscode(path: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("cmd").args(["/C", "code"]).arg(expand_tilde(path)).creation_flags(CREATE_NO_WINDOW).spawn();
}
