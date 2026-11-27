use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Platform, find_program, is_bare_name, starts_with_home};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Windows;

impl Platform for Windows {
    fn shell_command(&self, command: &str) -> Command {
        let mut process = Command::new("cmd");
        process.arg("/C").raw_arg(command).creation_flags(CREATE_NO_WINDOW);
        process
    }

    fn quiet_command(&self, program: &str) -> Command {
        let mut process = Command::new(program);
        process.creation_flags(CREATE_NO_WINDOW);
        process
    }

    fn open(&self, path: &Path) {
        let _ = self.quiet_command("cmd").args(["/C", "start", ""]).arg(path).spawn();
    }

    /// Prefers the .exe, then an npm .cmd shim, since terminals can't run the extensionless shim npm also installs.
    fn resolve_program(&self, name: &str) -> PathBuf {
        if !is_bare_name(name) {
            return PathBuf::from(name);
        }
        find_program(&[format!("{name}.exe"), format!("{name}.cmd")]).unwrap_or_else(|| PathBuf::from(name))
    }

    fn external_terminal(&self, title: &str, cwd: &Path, program: &Path, args: &[String]) -> Option<Command> {
        let mut command = Command::new("wt.exe");
        // Windows Terminal treats ';' as a command separator, even inside the title.
        command.args(["-w", "0", "new-tab", "--title", &title.replace(';', ","), "-d"]).arg(cwd).arg(program).args(args);
        Some(command)
    }

    fn paths_equal(&self, a: &Path, b: &Path) -> bool {
        let normalize = |p: &Path| self.portable(p).trim_end_matches('/').to_ascii_lowercase();
        normalize(a) == normalize(b)
    }

    /// Forward slashes work in Git Bash, cmd and PowerShell alike.
    fn portable(&self, path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn is_local_path_text(&self, text: &str) -> bool {
        let bytes = text.as_bytes();
        let drive = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
        let unc = text.starts_with("\\\\") && text.len() > 2;
        drive || unc || starts_with_home(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_compare_case_insensitively_and_print_with_forward_slashes() {
        assert!(Windows.paths_equal(Path::new(r"C:\Board\Tickets\T-1\"), Path::new("c:/board/tickets/t-1")));
        assert_eq!(Windows.portable(Path::new(r"C:\a\b")), "C:/a/b");
        assert!(Windows.is_local_path_text(r"C:\repos\app") && Windows.is_local_path_text(r"\\server\share"));
        assert!(!Windows.is_local_path_text("/usr/bin"));
    }

    #[test]
    fn external_terminal_is_a_windows_terminal_tab() {
        let command = Windows.external_terminal("T-1; Dev", Path::new("C:/work"), Path::new("lanes.exe"), &["launch".into()]).unwrap();
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(command.get_program(), "wt.exe");
        assert_eq!(args, ["-w", "0", "new-tab", "--title", "T-1, Dev", "-d", "C:/work", "lanes.exe", "launch"]);
    }
}
