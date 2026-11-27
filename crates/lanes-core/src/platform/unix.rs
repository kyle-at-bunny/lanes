//! macOS and Linux. Best effort: these paths compile in CI but aren't exercised by tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Platform, find_program, starts_with_home};

pub struct Unix;

impl Platform for Unix {
    fn shell_command(&self, command: &str) -> Command {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    }

    fn quiet_command(&self, program: &str) -> Command {
        Command::new(program)
    }

    fn open(&self, path: &Path) {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = Command::new(opener).arg(path).spawn();
    }

    fn find_on_path(&self, name: &str) -> Option<PathBuf> {
        find_program(&[name.to_string()])
    }

    fn external_terminal(&self, title: &str, cwd: &Path, program: &Path, args: &[String]) -> Option<Command> {
        let words: Vec<String> = std::iter::once(program.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|word| shell_quote(&word)).collect();
        let script = format!("cd {} && {}", shell_quote(&cwd.to_string_lossy()), words.join(" "));
        if cfg!(target_os = "macos") {
            let mut command = Command::new("osascript");
            let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
            command.args(["-e", &format!("tell application \"Terminal\" to do script \"{escaped}\""), "-e", "tell application \"Terminal\" to activate"]);
            return Some(command);
        }
        let terminal = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "x-terminal-emulator".into());
        let mut command = Command::new(terminal);
        command.args(["-T", title, "-e", "sh", "-c", &script]);
        Some(command)
    }

    /// macOS volumes are case-insensitive by default; Linux file systems are not.
    fn paths_equal(&self, a: &Path, b: &Path) -> bool {
        let normalize = |p: &Path| {
            let text = p.to_string_lossy().trim_end_matches('/').to_string();
            if cfg!(target_os = "macos") { text.to_lowercase() } else { text }
        };
        normalize(a) == normalize(b)
    }

    fn portable(&self, path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn is_local_path_text(&self, text: &str) -> bool {
        (text.starts_with('/') && text.len() > 1) || starts_with_home(text)
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
