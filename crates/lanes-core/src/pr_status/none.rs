//! The `pr_provider = "none"` choice: no link counts as a pull request, so nothing is fetched or shown.

use anyhow::{Result, bail};

use super::{PrProvider, PrStatus};

pub struct NoPullRequests;

impl PrProvider for NoPullRequests {
    fn normalize(&self, _url: &str) -> Option<String> {
        None
    }

    fn short_name(&self, _url: &str) -> Option<String> {
        None
    }

    fn fetch(&self, url: &str) -> Result<PrStatus> {
        bail!("pull request checks are off (pr_provider = \"none\"): {url}")
    }
}
