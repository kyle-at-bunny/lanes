use std::path::PathBuf;

/// Text typed into a terminal for files dropped on it, matching Windows Terminal: space-separated, quoted when needed.
pub fn dropped_paths_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|path| quote_if_needed(&path.to_string_lossy())).collect::<Vec<_>>().join(" ")
}

fn quote_if_needed(path: &str) -> String {
    if path.contains(char::is_whitespace) { format!("\"{path}\"") } else { path.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_are_joined_with_spaces() {
        let paths = [PathBuf::from(r"C:\shots\a.png"), PathBuf::from(r"C:\logs\b.txt")];
        assert_eq!(dropped_paths_text(&paths), r"C:\shots\a.png C:\logs\b.txt");
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        let paths = [PathBuf::from(r"C:\My Files\screen shot.png"), PathBuf::from(r"C:\x.png")];
        assert_eq!(dropped_paths_text(&paths), r#""C:\My Files\screen shot.png" C:\x.png"#);
    }

    #[test]
    fn no_paths_give_no_text() {
        assert_eq!(dropped_paths_text(&[]), "");
    }
}
