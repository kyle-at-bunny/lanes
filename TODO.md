# TODO

## Later

- **Editable ticket description:** show it in full and scrollable in the ticket panel (it's clipped to ~4 lines today), with Edit / Save / Cancel.
- **Config schema:** add `lanes config schema` (JSON Schema generated from the config structs via `schemars`, with doc comments as descriptions) and `lanes config docs` (embedded configuration reference). Point the assistant's built-in reference at them, and allow editor autocomplete for `board.toml`.
- **Panel divider extras:** a wide/narrow toggle on the panel header, and double-click the divider to reset it to 50%.
- **CI:** add `cargo fmt --check` and `cargo clippy -D warnings`, and a release workflow that attaches `lanes.exe` to GitHub releases.
