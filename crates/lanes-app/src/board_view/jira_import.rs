use std::collections::BTreeSet;

use anyhow::{Context as _, Result};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::config::JiraConfig;
use lanes_core::jira::{self, ImportReport, JiraClient, JiraIssue};

use super::LanesApp;

pub(super) struct JiraForm {
    keys: Entity<InputState>,
    issues: Option<Vec<JiraIssue>>,
    selected: BTreeSet<String>,
    busy: bool,
}

impl LanesApp {
    pub(super) fn show_jira_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.board.config.jira.as_ref().is_some_and(|j| j.has_api_credentials()) {
            let hint = "Lanes has no Jira API token. Open the Assistant and ask it to import or link issues; it can use your Jira tools (e.g. an Atlassian MCP server). To use this button instead, set [jira] site and email in board.toml and a token in LANES_JIRA_TOKEN.";
            return window.push_notification(Notification::info(hint), cx);
        }
        let keys = cx.new(|cx| InputState::new(window, cx).placeholder("Issue keys or links, e.g. PROJ-123, PROJ-456"));
        keys.update(cx, |state, cx| state.focus(window, cx));
        self.new_ticket = None;
        self.jira_form = Some(JiraForm { keys, issues: None, selected: BTreeSet::new(), busy: false });
        cx.notify();
    }

    fn jira_config(&self) -> Option<JiraConfig> {
        self.board.config.jira.clone()
    }

    fn import_typed_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.jira_form else { return };
        let input = form.keys.read(cx).value().to_string();
        let keys = match jira::parse_keys(&input) {
            Ok(keys) if keys.is_empty() => return window.push_notification(Notification::warning("Type at least one issue key"), cx),
            Ok(keys) => keys,
            Err(err) => return self.notify_error(&err, window, cx),
        };
        self.run_jira(window, cx, move |client| keys.iter().map(|key| client.issue(key)).collect(), Self::finish_import);
    }

    fn load_open_issues(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.jira_config() else { return };
        self.run_jira(window, cx, move |client| client.my_open_issues(&config.open_issues_jql), |app, issues, _window, cx| {
            if let Some(form) = app.jira_form.as_mut() {
                form.issues = Some(issues);
            }
            cx.notify();
        });
    }

    fn import_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.jira_form else { return };
        let chosen: Vec<JiraIssue> = form.issues.iter().flatten().filter(|issue| form.selected.contains(&issue.key)).cloned().collect();
        if chosen.is_empty() {
            return window.push_notification(Notification::warning("Pick at least one issue"), cx);
        }
        self.finish_import(chosen, window, cx);
    }

    fn toggle_issue(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(form) = self.jira_form.as_mut() {
            if !form.selected.remove(&key) {
                form.selected.insert(key);
            }
        }
        cx.notify();
    }

    /// Runs a Jira request off the UI thread, then hands the result to `done` (errors become toasts).
    fn run_jira<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        request: impl FnOnce(&JiraClient) -> Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(config) = self.jira_config() else { return };
        if let Some(form) = self.jira_form.as_mut() {
            form.busy = true;
        }
        let background = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = background.spawn(async move { JiraClient::from_config(&config).and_then(|client| request(&client)) }).await;
            let _ = this.update_in(cx, |app, window, cx| {
                if let Some(form) = app.jira_form.as_mut() {
                    form.busy = false;
                }
                match result {
                    Ok(value) => done(app, value, window, cx),
                    Err(err) => app.notify_error(&err, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_import(&mut self, issues: Vec<JiraIssue>, window: &mut Window, cx: &mut Context<Self>) {
        let column = self.board.config.first_column().id.clone();
        match jira::import(&self.board.tickets, &column, &issues).context("importing from Jira") {
            Ok(report) => {
                window.push_notification(Notification::success(describe_report(&report)), cx);
                self.jira_form = None;
                self.reload_tickets(window, cx);
                if let [only] = report.created.as_slice() {
                    self.select(only.clone(), window, cx);
                }
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
        cx.notify();
    }

    pub(super) fn render_jira_form(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let form = self.jira_form.as_ref()?;
        let theme = cx.theme();
        let picker: Vec<AnyElement> = form
            .issues
            .iter()
            .flatten()
            .map(|issue| {
                let key = issue.key.clone();
                let checked = form.selected.contains(&issue.key);
                h_flex()
                    .id(SharedString::from(format!("pick-{}", issue.key)))
                    .gap_2()
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(checked, |this| this.bg(theme.accent.opacity(0.25)))
                    .text_xs()
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_issue(key.clone(), cx)))
                    .child(div().flex_none().child(if checked { "☑" } else { "☐" }))
                    .child(div().flex_none().text_color(theme.muted_foreground).child(issue.key.clone()))
                    .child(div().flex_1().min_w_0().truncate().child(issue.summary.clone()))
                    .into_any_element()
            })
            .collect();
        let loaded = form.issues.as_ref().map(Vec::len);
        let selected = form.selected.len();
        Some(
            v_flex()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme.accent)
                .bg(theme.background)
                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child("Import from Jira"))
                .child(Input::new(&form.keys).small())
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .child(Button::new("jira-import-keys").label("Import keys").primary().xsmall().loading(form.busy).on_click(cx.listener(|this, _, window, cx| {
                            this.import_typed_keys(window, cx)
                        })))
                        .child(Button::new("jira-load-mine").label("My open issues").outline().xsmall().loading(form.busy).on_click(cx.listener(|this, _, window, cx| {
                            this.load_open_issues(window, cx)
                        })))
                        .child(Button::new("jira-cancel").label("Cancel").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                            this.jira_form = None;
                            cx.notify();
                        }))),
                )
                .when(loaded == Some(0), |this| this.child(div().text_xs().text_color(theme.muted_foreground).child("No open issues assigned to you.")))
                .when(!picker.is_empty(), |this| {
                    this.child(v_flex().id("jira-picker").max_h(px(260.)).overflow_y_scroll().gap_0p5().children(picker)).child(
                        Button::new("jira-import-selected").label(format!("Import selected ({selected})")).primary().xsmall().on_click(cx.listener(
                            |this, _, window, cx| this.import_selected(window, cx),
                        )),
                    )
                }),
        )
    }
}

fn describe_report(report: &ImportReport) -> String {
    let mut parts = Vec::new();
    if !report.created.is_empty() {
        parts.push(format!("Imported {}", report.created.join(", ")));
    }
    if !report.skipped.is_empty() {
        parts.push(format!("already on the board or archived: {}", report.skipped.join(", ")));
    }
    if parts.is_empty() { "Nothing to import".into() } else { parts.join("; ") }
}
