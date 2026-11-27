//! Everything that differs between operating systems, behind one interface chosen at compile time.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

pub trait Platform: Sync {
    /// A command line typed by the user (actions, cleanup), run through the system shell.
    fn shell_command(&self, command: &str) -> Command;
    /// A background program that must not flash a console window.
    fn quiet_command(&self, program: &str) -> Command;
    /// Opens a file in its default app, or a folder in the file manager.
    fn open(&self, path: &Path);
    /// The runnable file for a bare program name on PATH (or `~/.local/bin`), never a shell-script shim.
    fn find_on_path(&self, name: &str) -> Option<PathBuf>;
    /// A configured program: bare names are looked up with `find_on_path`, anything else is used as given.
    fn resolve_program(&self, name: &str) -> PathBuf {
        if !is_bare_name(name) {
            return PathBuf::from(name);
        }
        self.find_on_path(name).unwrap_or_else(|| PathBuf::from(name))
    }
    /// A command that opens a new terminal window or tab in `cwd` running `program args`, or None when unsupported.
    fn external_terminal(&self, title: &str, cwd: &Path, program: &Path, args: &[String]) -> Option<Command>;
    /// Whether two paths name the same file, with the platform's case rules.
    fn paths_equal(&self, a: &Path, b: &Path) -> bool;
    /// The path as text that the platform's shells and Claude's tools all accept.
    fn portable(&self, path: &Path) -> String;
    /// Whether text looks like an absolute local path (or `~/…`) worth turning into Open buttons.
    fn is_local_path_text(&self, text: &str) -> bool;
}

#[cfg(windows)]
pub fn current() -> &'static dyn Platform {
    &windows::Windows
}

#[cfg(not(windows))]
pub fn current() -> &'static dyn Platform {
    &unix::Unix
}

fn starts_with_home(text: &str) -> bool {
    (text.starts_with("~/") || text.starts_with("~\\")) && text.len() > 2
}

/// The first `candidates` file found on PATH, then in `~/.local/bin` (where native installers such as Claude Code's put programs).
fn find_program(candidates: &[String]) -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>()).unwrap_or_default();
    let local_bin = dirs::home_dir().map(|home| home.join(".local").join("bin"));
    path_dirs.into_iter().chain(local_bin).flat_map(|dir| candidates.iter().map(move |name| dir.join(name))).find(|candidate| candidate.is_file())
}

fn is_bare_name(name: &str) -> bool {
    let path = Path::new(name);
    !path.is_absolute() && path.extension().is_none() && path.components().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_are_local_on_every_platform() {
        assert!(current().is_local_path_text("~/projects/app"));
        assert!(!current().is_local_path_text("https://example.com"));
        assert!(!current().is_local_path_text("feat/login"));
    }

    #[test]
    fn unknown_programs_resolve_to_their_own_name() {
        assert_eq!(current().resolve_program("lanes-no-such-program"), PathBuf::from("lanes-no-such-program"));
        assert!(is_bare_name("claude") && !is_bare_name("claude.exe") && !is_bare_name("bin/claude"));
    }
}
