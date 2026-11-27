use std::collections::HashMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, Theme, h_flex};
use gpui_kit::*;
use lanes_core::paths::now_unix;
use lanes_core::pr_status::{self, Checks, PrCache, PrState, PrStatus, Review, Tone};

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

    fn links(&self, ticket_id: &str) -> &[String] {
        self.links.get(ticket_id).map(Vec::as_slice).unwrap_or_default()
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

    /// The card's PR row: chips like "2 PRs · 1 failing", "changes requested", "3 unresolved" that wrap on a narrow card.
    pub(super) fn pr_badge(&self, ticket_id: &str, cx: &App) -> Option<AnyElement> {
        let summary = pr_status::summarize(&self.prs.statuses(ticket_id))?;
        let theme = cx.theme();
        let chips: Vec<_> = summary
            .parts
            .into_iter()
            .map(|part| match summary.tone {
                Tone::Failing => tinted_tag(part, theme.danger),
                Tone::Pending => tinted_tag(part, theme.warning),
                Tone::Neutral => tinted_tag(part, theme.info),
                Tone::Done => Tag::secondary().xsmall().child(part).into_any_element(),
            })
            .collect();
        Some(h_flex().flex_wrap().gap_1().children(chips).into_any_element())
    }

    /// The ticket's PR links under its header, each as a short link with its status chips.
    pub(super) fn render_pr_links(&self, ticket_id: &str, cx: &App) -> Option<AnyElement> {
        let links = self.prs.links(ticket_id);
        if links.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let rows: Vec<_> = links
            .iter()
            .map(|url| {
                let target = url.clone();
                let link = Button::new(SharedString::from(format!("header-pr-{url}")))
                    .label(pr_status::short_name(url).unwrap_or_else(|| url.clone()))
                    .link()
                    .xsmall()
                    .tooltip(url.clone())
                    .on_click(move |_, _, cx| cx.open_url(&target));
                h_flex().items_center().gap_1().child(link).children(pr_chip(url, &self.prs.cache, theme)).into_any_element()
            })
            .collect();
        Some(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_x_3()
                .gap_y_1()
                .text_xs()
                .child(div().text_color(theme.muted_foreground).child(if links.len() == 1 { "PR" } else { "PRs" }))
                .children(rows)
                .into_any_element(),
        )
    }

    /// Lines for the Finish & clean up confirmation about PRs that are still open.
    pub(super) fn open_pr_warnings(&self, ticket_id: &str) -> Vec<String> {
        self.prs.statuses(ticket_id).into_iter().filter(|s| s.is_open()).map(|s| format!("⚠ PR #{} is still open ({}): {}", s.number, s.full_label(), s.url)).collect()
    }
}

/// Status chips shown next to a PR link (state and checks, review, unresolved comments); empty when the link isn't a PR or hasn't been fetched yet.
pub(super) fn pr_chip(url: &str, cache: &PrCache, theme: &Theme) -> Vec<AnyElement> {
    let Some(url) = pr_status::normalize_pr_url(url) else { return Vec::new() };
    if cache.error(&url).is_some() {
        return vec![div().text_color(theme.muted_foreground).child("status unavailable").into_any_element()];
    }
    let Some(status) = cache.status(&url) else { return Vec::new() };
    let color = match (status.state, status.checks) {
        (PrState::Merged, _) => theme.primary,
        (PrState::Closed, _) => theme.muted_foreground,
        (_, Checks::Failing) => theme.danger,
        (_, Checks::Pending) => theme.warning,
        _ => theme.success,
    };
    let review = status.review_label().map(|label| {
        let color = match status.review {
            Review::Approved => theme.success,
            Review::ChangesRequested => theme.danger,
            _ => theme.info,
        };
        tinted_tag(label, color)
    });
    let unresolved = status.unresolved_label().map(|label| tinted_tag(label, theme.warning));
    std::iter::once(tinted_tag(status.label(), color)).chain(review).chain(unresolved).collect()
}

/// Coloured text on a faint fill of the same colour, readable in both themes.
pub(super) fn tinted_tag(label: impl Into<SharedString>, color: Hsla) -> AnyElement {
    Tag::custom(color.opacity(0.15), color, color.opacity(0.6)).xsmall().child(label.into()).into_any_element()
}
