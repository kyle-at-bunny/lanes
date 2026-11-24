use std::collections::HashMap;

use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, Theme};
use gpui_kit::*;
use lanes_core::paths::now_unix;
use lanes_core::pr_status::{self, Checks, PrCache, PrState, PrStatus, Tone};

use super::LanesApp;

/// PR links per ticket and their fetched statuses; links are rescanned on every refresh tick.
#[derive(Default)]
pub(super) struct PrTracker {
    links: HashMap<String, Vec<String>>,
    cache: PrCache,
}

impl PrTracker {
    pub(super) fn cache(&self) -> &PrCache {
        &self.cache
    }

    fn statuses(&self, ticket_id: &str) -> Vec<&PrStatus> {
        self.links.get(ticket_id).map(|urls| self.cache.statuses(urls)).unwrap_or_default()
    }
}

impl LanesApp {
    /// Rescans artifacts for PR links and fetches the statuses that are due, one `gh` call at a time off the UI thread.
    pub(super) fn refresh_pr_statuses(&mut self, cx: &mut Context<Self>) {
        self.prs.links = self.tickets.iter().map(|t| (t.id.clone(), pr_status::pr_urls(&self.board.home, &self.board.config, &t.id))).collect();
        let due = self.prs.cache.claim_stale(self.prs.links.values().flatten().cloned(), now_unix());
        if due.is_empty() {
            return;
        }
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            for url in due {
                let target = url.clone();
                let result = background.spawn(async move { pr_status::fetch(&target).map_err(|err| format!("{err:#}")) }).await;
                let applied = this.update(cx, |app, cx| {
                    app.prs.cache.finish(url, result, now_unix());
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// A card badge like "2 PRs · 1 failing".
    pub(super) fn pr_badge(&self, ticket_id: &str, cx: &App) -> Option<AnyElement> {
        let summary = pr_status::summarize(&self.prs.statuses(ticket_id))?;
        let theme = cx.theme();
        let tag = match summary.tone {
            Tone::Failing => tinted_tag(summary.text, theme.danger),
            Tone::Pending => tinted_tag(summary.text, theme.warning),
            Tone::Neutral => tinted_tag(summary.text, theme.info),
            Tone::Done => Tag::secondary().xsmall().child(summary.text).into_any_element(),
        };
        Some(tag)
    }

    /// Lines for the Finish & clean up confirmation about PRs that are still open.
    pub(super) fn open_pr_warnings(&self, ticket_id: &str) -> Vec<String> {
        self.prs.statuses(ticket_id).into_iter().filter(|s| s.is_open()).map(|s| format!("⚠ PR #{} is still open ({}): {}", s.number, s.label(), s.url)).collect()
    }
}

/// A status chip shown next to a PR link, or None when the link isn't a PR or hasn't been fetched yet.
pub(super) fn pr_chip(url: &str, cache: &PrCache, theme: &Theme) -> Option<AnyElement> {
    let url = pr_status::normalize_pr_url(url)?;
    if cache.error(&url).is_some() {
        return Some(div().text_color(theme.muted_foreground).child("status unavailable").into_any_element());
    }
    let status = cache.status(&url)?;
    let color = match (status.state, status.checks) {
        (PrState::Merged, _) => theme.primary,
        (PrState::Closed, _) => theme.muted_foreground,
        (_, Checks::Failing) => theme.danger,
        (_, Checks::Pending) => theme.warning,
        _ => theme.success,
    };
    Some(tinted_tag(status.label(), color))
}

/// Coloured text on a faint fill of the same colour, readable in both themes.
pub(super) fn tinted_tag(label: impl Into<SharedString>, color: Hsla) -> AnyElement {
    Tag::custom(color.opacity(0.15), color, color.opacity(0.6)).xsmall().child(label.into()).into_any_element()
}
