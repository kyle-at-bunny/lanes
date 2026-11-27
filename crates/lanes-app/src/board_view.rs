use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, IconName, Sizable, Theme, ThemeMode, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::actions::{self, ActionOutcome};
use lanes_core::agent::{self, AgentKind, TranscriptSource};
use lanes_core::artifact_view::display_path;
use lanes_core::board::Board;
use lanes_core::cleanup::CleanupPlan;
use lanes_core::cli::format_duration;
use lanes_core::config::{ColumnDef, TerminalMode, ThemeSetting};
use lanes_core::paths::now_unix;
use lanes_core::session::LaunchSpec;
use lanes_core::state::{AgentStatus, Limit};
use lanes_core::ticket::Ticket;
use lanes_core::timeline::{self, EventKind};
use lanes_core::transcript::{Speaker, TranscriptEntry};
use uuid::Uuid;

mod archive;
mod artifacts;
mod assistant;
mod description;
mod detail_split;
mod folders;
mod lifecycle;
mod pr_status;
mod sessions_view;
mod split;

use crate::palette::TerminalThemeChoice;
use crate::pty::{GridSize, PtySession};
use crate::terminal_view::TerminalView;
use crate::tray::Tray;

#[derive(Clone)]
struct DraggedTicket {
    id: String,
    title: String,
}

struct DragPreview {
    title: String,
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().px_3().py_2().rounded_md().bg(cx.theme().popover).border_1().border_color(cx.theme().accent).text_sm().child(self.title.clone())
    }
}

struct PendingMove {
    ticket_id: String,
    to: String,
    missing: Vec<String>,
}

struct NewTicketForm {
    id: Entity<InputState>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    folder: Entity<InputState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Session,
    Timeline,
    Transcript,
    Artifacts,
}

struct LoadedTranscript {
    ticket_id: String,
    column_id: String,
    session_id: Uuid,
    agent: AgentKind,
    entries: Vec<TranscriptEntry>,
}

fn event_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Created => "created",
        EventKind::Moved => "moved",
        EventKind::SessionStarted => "session started",
        EventKind::SessionResumed => "session resumed",
        EventKind::ArtifactWritten => "artifact written",
        EventKind::ActionRun => "action",
        EventKind::CleanupRun => "cleanup",
        EventKind::Archived => "archived",
        EventKind::Restored => "restored",
    }
}

const COLUMN_MIN_WIDTH: f32 = 200.;
const BOARD_MIN_WIDTH: f32 = 320.;
const DETAIL_MIN_WIDTH: f32 = 420.;

/// What a card shows about its current column's session.
enum CardSession {
    None,
    NotStarted,
    Parked,
    Live(Option<AgentStatus>),
    /// Running, but its agent doesn't report working / needs input / your turn.
    LiveUntracked,
}

pub struct LanesApp {
    board: Board,
    tickets: Vec<Ticket>,
    terminals: HashMap<Uuid, Entity<TerminalView>>,
    selected: Option<String>,
    pending_move: Option<PendingMove>,
    new_ticket: Option<NewTicketForm>,
    known_status: HashMap<Uuid, AgentStatus>,
    detail_tab: DetailTab,
    transcript: Option<LoadedTranscript>,
    tray: Option<Tray>,
    quit_confirm: Option<usize>,
    quitting: bool,
    cleanup_confirm: Option<CleanupPlan>,
    show_archive: bool,
    archived: Vec<Ticket>,
    /// Ticket files that failed to load on the last reload, so each problem is shown once rather than every tick.
    unreadable_tickets: Vec<String>,
    assistant: Option<Entity<TerminalView>>,
    show_assistant: bool,
    config_mtime: Option<std::time::SystemTime>,
    folder_edit: Option<folders::FolderEdit>,
    show_sessions: bool,
    sessions: Vec<lanes_core::sessions::SessionInfo>,
    delete_confirm: Option<String>,
    assistant_reset_confirm: bool,
    split: Entity<gpui_kit::component::resizable::ResizableState>,
    ui_prefs: lanes_core::ui_prefs::UiPrefs,
    artifact_edit: Option<artifacts::ArtifactEdit>,
    description_edit: Option<description::DescriptionEdit>,
    show_other_artifacts: bool,
    prs: pr_status::PrTracker,
    _split_resized: Subscription,
    _appearance: Subscription,
}

impl LanesApp {
    pub fn new(board: Board, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (split, split_resized) = Self::create_split(cx);
        let ui_prefs = lanes_core::ui_prefs::UiPrefs::load(&board.home);
        let appearance = cx.observe_window_appearance(window, |app: &mut Self, window, cx| {
            if app.board.config.defaults.theme == ThemeSetting::System {
                app.apply_theme(window, cx);
            }
        });
        let mut app = Self {
            board,
            tickets: Vec::new(),
            terminals: HashMap::new(),
            selected: None,
            pending_move: None,
            new_ticket: None,
            known_status: HashMap::new(),
            detail_tab: DetailTab::Session,
            transcript: None,
            tray: None,
            quit_confirm: None,
            quitting: false,
            cleanup_confirm: None,
            show_archive: false,
            archived: Vec::new(),
            unreadable_tickets: Vec::new(),
            assistant: None,
            show_assistant: false,
            config_mtime: None,
            folder_edit: None,
            show_sessions: false,
            sessions: Vec::new(),
            delete_confirm: None,
            assistant_reset_confirm: false,
            split,
            ui_prefs,
            artifact_edit: None,
            description_edit: None,
            show_other_artifacts: false,
            prs: Default::default(),
            _split_resized: split_resized,
            _appearance: appearance,
        };
        app.config_mtime = app.board_file_mtime();
        app.install_tray(window, cx);
        app.apply_theme(window, cx);
        app.reload_tickets(window, cx);
        app.start_refresh_loop(cx);
        app
    }

    fn apply_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let defaults = &self.board.config.defaults;
        cx.set_global(TerminalThemeChoice(defaults.terminal_theme));
        match defaults.theme {
            ThemeSetting::System => Theme::sync_system_appearance(Some(window), cx),
            ThemeSetting::Light => Theme::change(ThemeMode::Light, Some(window), cx),
            ThemeSetting::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
        }
        window.refresh();
    }

    fn show_tab(&mut self, ticket_id: &str, tab: DetailTab, cx: &mut Context<Self>) {
        self.detail_tab = tab;
        let stale = self.transcript.as_ref().is_none_or(|t| t.ticket_id != ticket_id);
        if tab == DetailTab::Transcript && stale {
            let column = self.ticket(ticket_id).map(|t| t.column.clone()).unwrap_or_default();
            self.load_transcript(ticket_id, &column, cx);
        }
        cx.notify();
    }

    fn load_transcript(&mut self, ticket_id: &str, column_id: &str, cx: &mut Context<Self>) {
        self.transcript = self.ticket(ticket_id).and_then(|ticket| ticket.session(column_id)).map(|record| {
            let reported = self.board.states.hook(&record.id).and_then(|h| h.transcript_path);
            let agent_session_id = record.agent_session_id();
            let program = self.board.config.agent_program(record.agent);
            let source = TranscriptSource { program: &program, agent_session_id: &agent_session_id, cwd: &record.cwd, reported_path: reported.as_deref() };
            let entries = agent::get(record.agent).read_transcript(&source);
            LoadedTranscript { ticket_id: ticket_id.to_string(), column_id: column_id.to_string(), session_id: record.id, agent: record.agent, entries }
        });
        cx.notify();
    }

    /// Runs a column action off the UI thread and reports its result as a toast.
    fn run_action(&mut self, ticket_id: &str, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        let Some(action) = self.board.config.actions_for(&ticket.column).get(index).copied() else { return };
        let prepared = match self.board.prepare_action(ticket, action) {
            Ok(prepared) => prepared,
            Err(err) => return self.notify_error(&err, window, cx),
        };
        let ticket_id = ticket_id.to_string();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let label = prepared.label.clone();
            let outcome = background.spawn(async move { actions::run(&prepared) }).await;
            let _ = this.update_in(cx, |app, window, cx| app.report_action(&ticket_id, &label, outcome, window, cx));
        })
        .detach();
    }

    fn report_action(&mut self, ticket_id: &str, label: &str, outcome: Result<ActionOutcome>, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(err) => return self.notify_error(&err, window, cx),
        };
        let _ = self.board.record_action(ticket_id, label, outcome.success);
        let message = if outcome.output.is_empty() { label.to_string() } else { format!("{label}\n{}", outcome.output) };
        let notification = if outcome.success { Notification::success(message) } else { Notification::error(message) };
        window.push_notification(notification, cx);
        cx.notify();
    }

    fn cycle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.board.config.defaults.theme.next();
        let home = self.board.home.clone();
        if let Err(err) = self.board.config.save_theme(&home, next) {
            return self.notify_error(&err, window, cx);
        }
        self.config_mtime = self.board_file_mtime();
        self.apply_theme(window, cx);
        cx.notify();
    }

    fn start_refresh_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update_in(cx, |app, window, cx| {
                    app.reload_tickets(window, cx);
                    app.refresh_pr_statuses(cx);
                    app.announce_attention(window, cx);
                    app.update_tray_badge(cx);
                    app.reload_config_if_changed(window, cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn reload_tickets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.tickets.scan() {
            Ok(scan) => {
                self.tickets = scan.tickets;
                self.report_unreadable_tickets(scan.unreadable, window, cx);
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn report_unreadable_tickets(&mut self, unreadable: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        for problem in unreadable.iter().filter(|p| !self.unreadable_tickets.contains(p)) {
            self.notify_error(&anyhow::anyhow!("Skipped a ticket that couldn't be loaded: {problem}"), window, cx);
        }
        self.unreadable_tickets = unreadable;
    }

    /// Toasts when a session stops working and is waiting on the user.
    fn announce_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut announcements = Vec::new();
        for ticket in &self.tickets {
            for (column_id, record) in &ticket.sessions {
                let Some(state) = self.board.states.hook(&record.id) else { continue };
                let previous = self.known_status.insert(record.id, state.status);
                let needs_user = matches!(state.status, AgentStatus::NeedsInput | AgentStatus::Idle);
                if needs_user && previous.is_some_and(|p| p != state.status) {
                    let column = self.board.config.column(column_id).map(ColumnDef::title).unwrap_or(column_id);
                    announcements.push(format!("{} · {}: {}", ticket.id, column, state.status.label()));
                }
            }
        }
        for message in announcements {
            window.push_notification(Notification::info(message), cx);
        }
    }

    fn notify_error(&self, err: &anyhow::Error, window: &mut Window, cx: &mut App) {
        let message = format!("{err:#}");
        // Deferred because errors raised while the window is still opening come before its notification layer exists.
        window.defer(cx, move |window, cx| window.push_notification(Notification::error(message), cx));
    }

    fn ticket(&self, id: &str) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == id)
    }

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.ticket(&id).and_then(|t| self.live_terminal(t)) {
            let handle = terminal.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        }
        if self.detail_tab == DetailTab::Transcript {
            let column = self.ticket(&id).map(|t| t.column.clone()).unwrap_or_default();
            self.load_transcript(&id, &column, cx);
        }
        self.show_assistant = false;
        self.selected = Some(id);
        cx.notify();
    }

    fn request_move(&mut self, ticket_id: String, to: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(&ticket_id) else { return };
        if ticket.column == to {
            return;
        }
        match self.board.check_move(ticket, &to) {
            Ok(check) if !check.missing_outputs.is_empty() => {
                self.pending_move = Some(PendingMove { ticket_id, to, missing: check.missing_outputs });
                cx.notify();
            }
            Ok(_) => self.move_ticket(&ticket_id, &to, window, cx),
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn move_ticket(&mut self, ticket_id: &str, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_move = None;
        if let Err(err) = self.board.move_ticket(ticket_id, to) {
            return self.notify_error(&err, window, cx);
        }
        self.reload_tickets(window, cx);
        self.selected = Some(ticket_id.to_string());
        if self.is_last_column(to) && self.board.config.defaults.cleanup_on_done {
            self.request_cleanup(ticket_id, window, cx);
        }
        cx.notify();
    }

    fn step_ticket(&mut self, ticket_id: &str, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        let Some(index) = self.board.config.column_index(&ticket.column) else { return };
        let target = index as isize + delta;
        if let Some(column) = usize::try_from(target).ok().and_then(|i| self.board.config.columns.get(i)) {
            self.request_move(ticket_id.to_string(), column.id.clone(), window, cx);
        }
    }

    /// Starts or resumes the session for the ticket's current column; focuses it if it is already running.
    fn open_session(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.ticket(ticket_id).and_then(|t| self.live_terminal(t)) {
            let handle = terminal.read(cx).focus_handle(cx);
            handle.focus(window, cx);
            return;
        }
        let result = self.board.prepare_session(ticket_id).and_then(|spec| match self.board.config.defaults.terminal {
            TerminalMode::Embedded => self.spawn_embedded(spec, window, cx),
            TerminalMode::External => self.spawn_external(&spec),
        });
        if let Err(err) = result {
            self.notify_error(&err, window, cx);
        }
        self.reload_tickets(window, cx);
        cx.notify();
    }

    fn spawn_embedded(&mut self, spec: LaunchSpec, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        let session = PtySession::spawn(&spec, GridSize { columns: 120, rows: 40 })?;
        let terminal = cx.new(|cx| TerminalView::new(spec.session_id, session, cx));
        let handle = terminal.read(cx).focus_handle(cx);
        handle.focus(window, cx);
        if let Some(draft) = spec.draft {
            type_draft_when_ready(terminal.clone(), draft, cx);
        }
        self.terminals.insert(spec.session_id, terminal);
        Ok(())
    }

    fn spawn_external(&self, spec: &LaunchSpec) -> Result<()> {
        let command = spec.external_terminal_command(&self.board.home, &self.board.lanes_exe);
        let mut command = command.context("this platform has no external terminal; set terminal = \"embedded\" in board.toml")?;
        spec.save(&self.board.home)?;
        command.spawn().context("opening the external terminal")?;
        Ok(())
    }

    fn suspend_session(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.remove(&session_id) {
            terminal.read(cx).session.kill();
        }
        cx.notify();
    }

    fn request_handoff(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        let Some(terminal) = self.live_terminal(ticket) else { return };
        match self.board.handoff_message(ticket) {
            Ok(message) => terminal.read(cx).send_text(&message),
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn live_terminal(&self, ticket: &Ticket) -> Option<Entity<TerminalView>> {
        let record = ticket.session(&ticket.column)?;
        self.terminals.get(&record.id).cloned()
    }

    fn card_session(&self, ticket: &Ticket, cx: &App) -> CardSession {
        let has_session = self.board.config.column(&ticket.column).is_some_and(ColumnDef::has_session);
        let Some(record) = ticket.session(&ticket.column) else {
            return if has_session { CardSession::NotStarted } else { CardSession::None };
        };
        let hook = self.board.states.hook(&record.id).map(|h| h.status);
        let tracked = agent::get(record.agent).capabilities().live_state;
        match self.terminals.get(&record.id) {
            Some(terminal) if terminal.read(cx).session.has_exited() => CardSession::Parked,
            Some(_) if !tracked => CardSession::LiveUntracked,
            Some(_) => CardSession::Live(hook),
            None if self.board.config.defaults.terminal == TerminalMode::External && hook.is_some_and(|s| s != AgentStatus::Exited) => CardSession::Live(hook),
            None => CardSession::Parked,
        }
    }

    fn show_new_ticket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = NewTicketForm {
            id: cx.new(|cx| InputState::new(window, cx).placeholder("ID (optional, e.g. PROJ-123)")),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("Title")),
            description: cx.new(|cx| TextareaState::new(window, cx).placeholder("Description / context for the first stage").rows(5)),
            folder: cx.new(|cx| InputState::new(window, cx).placeholder("Working folder (optional, all stages)")),
        };
        form.title.update(cx, |state, cx| state.focus(window, cx));
        self.new_ticket = Some(form);
        cx.notify();
    }

    fn create_ticket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.new_ticket else { return };
        let id = form.id.read(cx).value().to_string();
        let title = form.title.read(cx).value().to_string();
        let description = form.description.read(cx).value().to_string();
        let folder = form.folder.read(cx).value().to_string();
        if title.trim().is_empty() {
            return window.push_notification(Notification::warning("A ticket needs a title"), cx);
        }
        let column = self.board.config.first_column().id.clone();
        let created = self.board.tickets.create(Some(&id), &title, &description, &column).and_then(|mut ticket| {
            ticket.set_cwd(&folder);
            self.board.tickets.save(&ticket).map(|()| ticket)
        });
        match created {
            Ok(ticket) => {
                self.new_ticket = None;
                self.reload_tickets(window, cx);
                self.select(ticket.id, window, cx);
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn reload_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.reload_config() {
            Ok(()) => {
                self.apply_theme(window, cx);
                window.push_notification(Notification::success("Board config reloaded"), cx)
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
        cx.notify();
    }
}

const DRAFT_SETTLE: Duration = Duration::from_millis(800);
const DRAFT_TYPE_ANYWAY: Duration = Duration::from_secs(20);

/// Types the draft into Claude's input box once its UI has drawn and gone quiet, so the keystrokes aren't lost during startup.
fn type_draft_when_ready(terminal: Entity<TerminalView>, draft: String, cx: &mut Context<LanesApp>) {
    cx.spawn(async move |_, cx| {
        let started = std::time::Instant::now();
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let (exited, quiet) = terminal.read_with(cx, |view, _| (view.session.has_exited(), view.session.quiet_for()));
            if exited {
                break;
            }
            let settled = started.elapsed() > Duration::from_secs(2) && quiet >= DRAFT_SETTLE;
            if settled || started.elapsed() > DRAFT_TYPE_ANYWAY {
                terminal.read_with(cx, |view, _| view.type_text(&draft));
                break;
            }
        }
    })
    .detach();
}

/// Opens a file in its default app, or a folder in the file manager.
fn open_path(path: &Path) {
    lanes_core::platform::current().open(path);
}

impl Render for LanesApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_top_bar(cx))
            .when_some(self.render_quit_confirm(cx), |this, banner| this.child(banner))
            .when_some(self.render_cleanup_confirm(cx), |this, banner| this.child(banner))
            .when_some(self.render_delete_confirm(cx), |this, banner| this.child(banner))
            .when_some(self.render_pending_move(cx), |this, banner| this.child(banner))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.show_sessions, |this| this.child(self.render_sessions(cx)))
                    .when(self.show_archive && !self.show_sessions, |this| this.child(self.render_archive(cx)))
                    .when(!self.show_archive && !self.show_sessions, |this| {
                        let board = self.render_board(window, cx).into_any_element();
                        let panel = self.render_right_panel(window, cx);
                        this.child(self.render_split(board, panel, window))
                    }),
            )
    }
}

impl LanesApp {
    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let limits = self.board.states.latest_limits();
        let attention = self.tickets.iter().filter(|t| matches!(self.card_session(t, cx), CardSession::Live(Some(AgentStatus::NeedsInput)))).count();
        h_flex()
            .flex_wrap()
            .px_3()
            .py_2()
            .gap_x_3()
            .gap_y_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::BOLD).child("Lanes"))
            .child(Button::new("new-ticket").label("New ticket").primary().small().on_click(cx.listener(|this, _, window, cx| this.show_new_ticket(window, cx))))
            .child({
                let button = Button::new("assistant").label("Assistant").small().tooltip("An agent session that can operate and reconfigure this board");
                let button = if self.show_assistant { button.primary() } else { button.outline() };
                button.on_click(cx.listener(|this, _, window, cx| this.toggle_assistant(window, cx)))
            })
            .when(attention > 0, |this| {
                let theme = cx.theme();
                this.child(Tag::custom(theme.warning, theme.warning_foreground, theme.warning).small().child(format!("{attention} waiting for input")))
            })
            .child(div().flex_1())
            .when_some(limits.as_ref().and_then(|l| l.five_hour), |this, limit| this.child(usage_meter("5h", limit, cx)))
            .when_some(limits.as_ref().and_then(|l| l.seven_day), |this, limit| this.child(usage_meter("7d", limit, cx)))
            .when(limits.is_none(), |this| this.child(div().text_xs().text_color(cx.theme().muted_foreground).child("plan usage appears after the first response")))
            .child({
                let label = if self.show_sessions { "Back to board" } else { "Sessions" };
                Button::new("toggle-sessions").label(label).ghost().small().on_click(cx.listener(|this, _, window, cx| this.toggle_sessions(window, cx)))
            })
            .child({
                let label = if self.show_archive { "Back to board" } else { "Archive" };
                Button::new("toggle-archive").label(label).ghost().small().on_click(cx.listener(|this, _, window, cx| this.toggle_archive(window, cx)))
            })
            .child(self.render_theme_toggle(cx))
            .child(Button::new("reload-config").label("Reload config").ghost().small().on_click(cx.listener(|this, _, window, cx| this.reload_config(window, cx))))
            .child({
                let board_file = self.board.home.board_file();
                Button::new("open-config").label("Edit board.toml").ghost().small().on_click(move |_, _, _| open_path(&board_file))
            })
            .child(
                Button::new("quit")
                    .label("Quit")
                    .ghost()
                    .small()
                    .tooltip("Closing the window keeps sessions running in the tray; this quits Lanes")
                    .on_click(cx.listener(|this, _, window, cx| this.request_quit(window, cx))),
            )
    }

    fn render_theme_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let setting = self.board.config.defaults.theme;
        let (icon, label) = match setting {
            ThemeSetting::System if cx.theme().is_dark() => (IconName::Moon, "Auto"),
            ThemeSetting::System => (IconName::Sun, "Auto"),
            ThemeSetting::Light => (IconName::Sun, "Light"),
            ThemeSetting::Dark => (IconName::Moon, "Dark"),
        };
        Button::new("theme-toggle")
            .icon(icon)
            .label(label)
            .ghost()
            .small()
            .tooltip(format!("Theme: {} (click for {})", setting.as_str(), setting.next().as_str()))
            .on_click(cx.listener(|this, _, window, cx| this.cycle_theme(window, cx)))
    }

    fn render_pending_move(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let pending = self.pending_move.as_ref()?;
        let (ticket_id, to) = (pending.ticket_id.clone(), pending.to.clone());
        Some(
            h_flex()
                .px_3()
                .py_2()
                .gap_3()
                .bg(cx.theme().warning.opacity(0.15))
                .child(div().flex_1().min_w_0().child(format!("{} has no {} yet. Move it anyway?", pending.ticket_id, pending.missing.join(", "))))
                .child(
                    Button::new("move-anyway")
                        .label("Move anyway")
                        .warning()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| this.move_ticket(&ticket_id, &to, window, cx))),
                )
                .child(Button::new("cancel-move").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                    this.pending_move = None;
                    cx.notify();
                }))),
        )
    }

    fn render_board(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns: Vec<_> = self.board.config.columns.iter().map(|c| self.render_column(c, window, cx).into_any_element()).collect();
        h_flex()
            .id("board")
            .flex_1()
            .min_w(px(BOARD_MIN_WIDTH))
            .h_full()
            .p_2()
            .gap_2()
            .items_start()
            .overflow_x_scroll()
            .on_click(cx.listener(|this, _, _, cx| this.close_right_panel(cx)))
            .children(columns)
    }

    /// Clicking the board itself, rather than a card or a form on it, puts away the ticket panel or the assistant.
    fn close_right_panel(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_some() || self.show_assistant {
            self.selected = None;
            self.show_assistant = false;
            cx.notify();
        }
    }

    fn render_column(&self, column: &ColumnDef, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column_id = column.id.clone();
        let cards: Vec<_> = self.tickets.iter().filter(|t| t.column == column.id).map(|t| self.render_card(t, cx).into_any_element()).collect();
        let theme = cx.theme();
        v_flex()
            .id(SharedString::from(format!("column-{}", column.id)))
            .flex_1()
            .min_w(px(COLUMN_MIN_WIDTH))
            .h_full()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme.muted.opacity(0.4))
            .drag_over::<DraggedTicket>(|style, _, _, cx| style.bg(cx.theme().accent.opacity(0.3)))
            .on_drop(cx.listener(move |this, dragged: &DraggedTicket, window, cx| this.request_move(dragged.id.clone(), column_id.clone(), window, cx)))
            .child(
                h_flex()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(column.title().to_string()))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(cards.len().to_string())),
            )
            .when_some(column.cwd.clone(), |this, cwd| this.child(div().text_xs().text_color(theme.muted_foreground).child(cwd)))
            .when(column.id == self.board.config.first_column().id, |this| {
                this.when_some(self.render_new_ticket_form(window, cx), |this, form| this.child(keep_clicks("new-ticket-form", form)))
            })
            .child(v_flex().id(SharedString::from(format!("cards-{}", column.id))).flex_1().gap_2().overflow_y_scroll().children(cards))
    }

    fn render_new_ticket_form(&self, _window: &mut Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let form = self.new_ticket.as_ref()?;
        Some(
            v_flex()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().accent)
                .bg(cx.theme().background)
                .child(Input::new(&form.id).small())
                .child(Input::new(&form.title).small())
                .child(Textarea::new(&form.description))
                .child(Input::new(&form.folder).small())
                .child(
                    h_flex()
                        .gap_2()
                        .child(Button::new("create-ticket").label("Create").primary().small().on_click(cx.listener(|this, _, window, cx| this.create_ticket(window, cx))))
                        .child(Button::new("cancel-ticket").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                            this.new_ticket = None;
                            cx.notify();
                        }))),
                ),
        )
    }

    fn render_card(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let id = ticket.id.clone();
        let selected = self.selected.as_deref() == Some(ticket.id.as_str());
        let session = self.card_session(ticket, cx);
        let context = ticket.session(&ticket.column).and_then(|r| self.board.states.status(&r.id)).and_then(|s| s.context_used_pct);
        let outputs = self.output_summary(ticket);
        let prs = self.pr_badge(&ticket.id, cx);
        let dragged = DraggedTicket { id: ticket.id.clone(), title: format!("{} {}", ticket.id, ticket.title) };
        let theme = cx.theme();
        let (left_id, right_id) = (id.clone(), id.clone());
        v_flex()
            .id(SharedString::from(format!("card-{}", ticket.id)))
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(if selected { theme.ring } else { theme.border })
            .bg(theme.background)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.select(id.clone(), window, cx)
            }))
            .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| DragPreview { title: dragged.title.clone() }))
            .child(h_flex().justify_between().child(card_id_label(ticket, cx)).child(session_tag(&session, cx)))
            .child(div().text_sm().child(ticket.title.clone()))
            .when_some(prs, |this, badge| this.child(badge))
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .when_some(context, |this, pct| this.child(context_label(pct, cx)))
                    .when_some(outputs, |this, outputs| this.child(outputs))
                    .child(div().flex_1())
                    .child(Button::new(SharedString::from(format!("left-{}", ticket.id))).label("‹").ghost().xsmall().on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.step_ticket(&left_id, -1, window, cx)
                        },
                    )))
                    .child(Button::new(SharedString::from(format!("right-{}", ticket.id))).label("›").ghost().xsmall().on_click(cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.step_ticket(&right_id, 1, window, cx)
                        },
                    ))),
            )
    }

    fn output_summary(&self, ticket: &Ticket) -> Option<String> {
        let column = self.board.config.column(&ticket.column)?;
        let produced: Vec<_> = column.produced().collect();
        if produced.is_empty() {
            return None;
        }
        let store = self.board.artifacts(&ticket.id);
        let done = produced.iter().filter(|a| store.read(a.name).ok().flatten().is_some()).count();
        Some(format!("out {done}/{}", produced.len()))
    }

    fn render_detail(&self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ticket) = self.ticket(ticket_id) else {
            return div().into_any_element();
        };
        let column = self.board.config.column(&ticket.column);
        let column_title = column.map(ColumnDef::title).unwrap_or(&ticket.column).to_string();
        let terminal = self.live_terminal(ticket);
        let theme = cx.theme().clone();
        let header = v_flex()
            .p_3()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_xs().text_color(theme.muted_foreground).child(ticket.id.clone()))
                    .child(Tag::secondary().small().child(column_title))
                    .child(div().flex_1())
                    .when(self.is_last_column(&ticket.column), |this| {
                        let ticket_id = ticket.id.clone();
                        this.child(
                            Button::new("finish-ticket")
                                .label("Finish & clean up")
                                .warning()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, window, cx| this.request_cleanup(&ticket_id, window, cx))),
                        )
                    })
                    .child({
                        let ticket_id = ticket.id.clone();
                        Button::new("delete-ticket")
                            .label("Delete")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| this.request_delete_ticket(&ticket_id, cx)))
                    })
                    .child(self.panel_width_button(cx))
                    .child(Button::new("close-detail").label("Close").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        cx.notify();
                    }))),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child(ticket.title.clone()))
            .when_some(ticket.link.clone(), |this, link| {
                this.child(
                    h_flex().child(
                        Button::new("open-link")
                            .label(format!("Open {}", link.key))
                            .link()
                            .xsmall()
                            .tooltip(link.url.clone())
                            .on_click(move |_, _, cx| cx.open_url(&link.url)),
                    ),
                )
            })
            .child(self.render_description(ticket, cx))
            .child(self.render_session_actions(ticket, terminal.as_ref(), cx))
            .when_some(self.render_folder_row(ticket, cx), |this, row| this.child(row))
            .when_some(self.render_column_actions(ticket, cx), |this, actions| this.child(actions))
            .when_some(self.render_artifact_summary(ticket, cx), |this, summary| this.child(summary))
            .when_some(self.render_pr_links(&ticket.id, cx), |this, prs| this.child(prs));
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(theme.border)
            .on_drag_move(cx.listener(|this, event, _, cx| this.on_header_drag(event, cx)))
            .child(self.render_detail_header(header, cx))
            .child(self.render_tab_strip(ticket, cx))
            .child(match (self.detail_tab, terminal) {
                (DetailTab::Session, Some(terminal)) => div().flex_1().min_h_0().p_1().bg(theme.background).child(terminal).into_any_element(),
                (DetailTab::Session, None) => self.render_no_session(ticket, column, window, cx).into_any_element(),
                (DetailTab::Timeline, _) => self.render_timeline(ticket, cx).into_any_element(),
                (DetailTab::Transcript, _) => self.render_transcript(ticket, cx).into_any_element(),
                (DetailTab::Artifacts, _) => self.render_artifacts_tab(ticket, cx).into_any_element(),
            })
            .into_any_element()
    }

    fn render_column_actions(&self, ticket: &Ticket, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let actions = self.board.config.actions_for(&ticket.column);
        if actions.is_empty() {
            return None;
        }
        let buttons: Vec<_> = actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let ticket_id = ticket.id.clone();
                Button::new(SharedString::from(format!("action-{index}")))
                    .label(action.label.clone())
                    .outline()
                    .xsmall()
                    .tooltip(action.command.clone())
                    .on_click(cx.listener(move |this, _, window, cx| this.run_action(&ticket_id, index, window, cx)))
                    .into_any_element()
            })
            .collect();
        Some(h_flex().gap_1().flex_wrap().pt_1().children(buttons))
    }

    fn render_tab_strip(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [(DetailTab::Session, "Session"), (DetailTab::Timeline, "Timeline"), (DetailTab::Transcript, "Transcript"), (DetailTab::Artifacts, "Artifacts")];
        let buttons: Vec<_> = tabs
            .into_iter()
            .map(|(tab, label)| {
                let ticket_id = ticket.id.clone();
                let button = Button::new(SharedString::from(format!("tab-{label}"))).label(label).small();
                let button = if self.detail_tab == tab { button.primary() } else { button.ghost() };
                button.on_click(cx.listener(move |this, _, _, cx| this.show_tab(&ticket_id, tab, cx))).into_any_element()
            })
            .collect();
        h_flex().gap_1().px_3().py_1().border_b_1().border_color(cx.theme().border).children(buttons)
    }

    fn render_timeline(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let now = now_unix();
        let theme = cx.theme();
        let rows: Vec<_> = timeline::read(&self.board.home, &ticket.id)
            .into_iter()
            .rev()
            .map(|event| {
                let detail = match event.by.as_deref() {
                    Some(timeline::USER) => format!("{} (by you)", event.detail),
                    Some(column) => format!("{} (by {})", event.detail, self.board.column_title(column)),
                    None => event.detail,
                };
                h_flex()
                    .gap_3()
                    .text_xs()
                    .child(div().w(px(70.)).text_color(theme.muted_foreground).child(format!("{} ago", format_duration(now.saturating_sub(event.at)))))
                    .child(div().w(px(110.)).font_weight(FontWeight::SEMIBOLD).child(event_label(event.kind)))
                    .child(div().flex_1().child(detail))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("timeline")
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_1()
            .overflow_y_scroll()
            .when(empty, |this| this.child(div().text_sm().text_color(theme.muted_foreground).child("No events recorded yet.")))
            .children(rows)
    }

    fn render_transcript(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.transcript.as_ref().filter(|t| t.ticket_id == ticket.id);
        let selected = loaded.map(|t| t.session_id);
        let pickers: Vec<_> = self
            .board
            .config
            .columns
            .iter()
            .filter_map(|column| ticket.session(&column.id).map(|record| (column, record.id)))
            .map(|(column, session_id)| {
                let ticket_id = ticket.id.clone();
                let column_id = column.id.clone();
                let button = Button::new(SharedString::from(format!("transcript-{}", column.id))).label(column.title().to_string()).xsmall();
                let button = if selected == Some(session_id) { button.primary() } else { button.ghost() };
                button.on_click(cx.listener(move |this, _, _, cx| this.load_transcript(&ticket_id, &column_id, cx))).into_any_element()
            })
            .collect();
        let theme = cx.theme();
        let body: AnyElement = match loaded {
            None if pickers.is_empty() => div().text_sm().text_color(theme.muted_foreground).child("This ticket has no sessions yet.").into_any_element(),
            None => div().text_sm().text_color(theme.muted_foreground).child("Pick a stage to read its transcript.").into_any_element(),
            Some(loaded) if !agent::get(loaded.agent).capabilities().transcript => div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("Transcripts aren't available for {} sessions.", agent::get(loaded.agent).display_name()))
                .into_any_element(),
            Some(loaded) if loaded.entries.is_empty() => {
                div().text_sm().text_color(theme.muted_foreground).child("No transcript found for this session yet.").into_any_element()
            }
            Some(loaded) => v_flex()
                .gap_2()
                .children(
                    loaded
                        .entries
                        .iter()
                        .map(|entry| {
                            let (label, color) = match entry.speaker {
                                Speaker::User => ("You", theme.primary),
                                Speaker::Assistant => (agent::get(loaded.agent).display_name(), theme.success),
                                Speaker::Tool => ("Tool", theme.muted_foreground),
                            };
                            v_flex()
                                .gap_0p5()
                                .text_xs()
                                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(color).child(label))
                                .child(div().when(entry.speaker == Speaker::Tool, |d| d.font_family(theme.mono_font_family.clone())).child(entry.text.clone()))
                        })
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .child(h_flex().gap_1().px_3().py_1().children(pickers).child(div().flex_1()).when_some(loaded.map(|t| t.column_id.clone()), |this, column_id| {
                let ticket_id = ticket.id.clone();
                this.child(
                    Button::new("refresh-transcript")
                        .label("Refresh")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| this.load_transcript(&ticket_id, &column_id, cx))),
                )
            }))
            .child(v_flex().id("transcript").flex_1().min_h_0().px_3().pb_3().overflow_y_scroll().child(body))
    }

    fn render_session_actions(&self, ticket: &Ticket, terminal: Option<&Entity<TerminalView>>, cx: &mut Context<Self>) -> impl IntoElement {
        let (open_id, handoff_id) = (ticket.id.clone(), ticket.id.clone());
        let session_id = terminal.map(|t| t.read(cx).session_id);
        let exited = terminal.is_some_and(|t| t.read(cx).session.has_exited());
        let has_session = self.board.config.column(&ticket.column).is_some_and(ColumnDef::has_session);
        let label = match (terminal.is_some() && !exited, ticket.session(&ticket.column).is_some()) {
            (true, _) => "Focus session",
            (false, true) => "Resume session",
            (false, false) => "Start session",
        };
        h_flex()
            .flex_wrap()
            .gap_2()
            .pt_1()
            .when(has_session, |this| {
                this.child(Button::new("open-session").label(label).primary().small().on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(id) = session_id.filter(|_| exited) {
                        this.suspend_session(id, cx);
                    }
                    this.open_session(&open_id, window, cx)
                })))
            })
            .when(terminal.is_some() && !exited, |this| {
                this.child(
                    Button::new("handoff")
                        .label("Request handoff")
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| this.request_handoff(&handoff_id, window, cx))),
                )
            })
            .when_some(session_id, |this, id| {
                this.child(Button::new("suspend").label("Suspend").ghost().small().on_click(cx.listener(move |this, _, _, cx| this.suspend_session(id, cx))))
            })
            .when_some(ticket.session(&ticket.column).map(|r| r.cwd.clone()), |this, cwd| {
                this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(display_path(&cwd.display().to_string(), artifacts::PATH_DISPLAY_CHARS)))
                    .child(Button::new("open-session-folder").label("Open folder").ghost().xsmall().on_click(move |_, _, _| open_path(&cwd)))
            })
    }

    fn render_no_session(&self, ticket: &Ticket, column: Option<&ColumnDef>, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let external_live = matches!(self.card_session(ticket, cx), CardSession::Live(_)) && self.board.config.defaults.terminal == TerminalMode::External;
        let message = match (column.is_some_and(ColumnDef::has_session), ticket.session(&ticket.column)) {
            (false, _) => "This column has no session. Drag the ticket to a stage column to work on it.".to_string(),
            (true, Some(_)) if external_live => "This session is running in a Windows Terminal tab (terminal = \"external\").".to_string(),
            (true, Some(record)) => format!("Session {} is parked. Resume it to continue where it left off.", record.id),
            (true, None) => "No session yet for this stage.".to_string(),
        };
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(div().w_full().min_w_0().text_center().child(message))
    }
}

/// The ticket id, plus its work item's key as a link that opens it without selecting the card.
fn card_id_label(ticket: &Ticket, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let id = div().text_color(theme.muted_foreground).child(ticket.id.clone());
    let Some(work_item) = ticket.link.clone() else {
        return id.text_xs().into_any_element();
    };
    let link = div()
        .id(SharedString::from(format!("work-item-link-{}", ticket.id)))
        .text_color(theme.info)
        .cursor_pointer()
        .hover(|style| style.underline())
        .child(work_item.key.clone())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            cx.open_url(&work_item.url);
        });
    let same_key = ticket.link.as_ref().is_some_and(|l| l.key.eq_ignore_ascii_case(&ticket.id));
    h_flex().gap_1().text_xs().when(!same_key, |this| this.child(id)).child(link).into_any_element()
}

fn session_tag(session: &CardSession, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match session {
        CardSession::None => div().into_any_element(),
        CardSession::NotStarted => Tag::secondary().xsmall().outline().child("not started").into_any_element(),
        CardSession::Parked => Tag::secondary().xsmall().child("parked").into_any_element(),
        CardSession::Live(None) => solid_tag(theme.info, theme.info_foreground, "starting"),
        CardSession::LiveUntracked => solid_tag(theme.info, theme.info_foreground, "running"),
        CardSession::Live(Some(status)) => match status {
            AgentStatus::Working => solid_tag(theme.primary, theme.primary_foreground, status.label()),
            AgentStatus::NeedsInput => solid_tag(theme.warning, theme.warning_foreground, status.label()),
            AgentStatus::Idle | AgentStatus::Ready => solid_tag(theme.success, theme.success_foreground, status.label()),
            AgentStatus::Exited => Tag::secondary().xsmall().child(status.label()).into_any_element(),
        },
    }
}

/// A filled tag with black or white text picked from the fill's lightness, so it stays readable in both themes.
fn solid_tag(fill: Hsla, _paired: Hsla, label: &'static str) -> AnyElement {
    let text = if fill.l > 0.55 { hsla(0., 0., 0.08, 1.) } else { hsla(0., 0., 0.98, 1.) };
    Tag::custom(fill, text, fill).xsmall().child(label).into_any_element()
}

fn context_label(pct: f64, cx: &App) -> impl IntoElement {
    let color = match pct {
        p if p >= 90.0 => cx.theme().danger,
        p if p >= 70.0 => cx.theme().warning,
        _ => cx.theme().muted_foreground,
    };
    div().text_color(color).child(format!("ctx {pct:.0}%"))
}

fn usage_meter(label: &'static str, limit: Limit, cx: &App) -> impl IntoElement {
    let reset = limit.resets_at.map(|at| format!(" · resets in {}", format_duration(at.saturating_sub(now_unix())))).unwrap_or_default();
    let color = match limit.used_pct {
        p if p >= 90.0 => cx.theme().danger,
        p if p >= 70.0 => cx.theme().warning,
        _ => cx.theme().primary,
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().w(px(90.)).child(Progress::new(label).value(limit.used_pct as f32).color(color)))
        .child(format!("{label} {:.0}%{reset}", limit.used_pct))
}

/// Wraps something on the board whose clicks must not reach the board, which would close the open panel.
fn keep_clicks(id: &'static str, content: impl IntoElement) -> impl IntoElement {
    div().id(id).on_click(|_, _, cx| cx.stop_propagation()).child(content)
}
