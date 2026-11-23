use anyhow::Result;

use crate::paths::{LanesHome, write_atomic};

pub const STARTER_BOARD: &str = include_str!("../assets/board.toml");

const STARTER_PROMPTS: &[(&str, &str)] = &[
    ("prompts/plan.md", include_str!("../assets/prompts/plan.md")),
    ("prompts/dev.md", include_str!("../assets/prompts/dev.md")),
    ("prompts/review.md", include_str!("../assets/prompts/review.md")),
    ("prompts/qa.md", include_str!("../assets/prompts/qa.md")),
];

pub fn write_starter_board(home: &LanesHome) -> Result<()> {
    write_atomic(&home.board_file(), STARTER_BOARD.as_bytes())?;
    for (relative, contents) in STARTER_PROMPTS {
        let path = home.resolve(relative);
        if !path.exists() {
            write_atomic(&path, contents.as_bytes())?;
        }
    }
    Ok(())
}
