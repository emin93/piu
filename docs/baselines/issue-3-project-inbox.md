# Issue 3 project inbox baseline

Captured from the packaged Apple Silicon `Più.app` at a consistent 1180 × 761 window capture. The checked-in light and dark PNGs use the deterministic development-only review fixture from `src-tauri/examples/seed_inbox_review.rs`; normal builds never seed product data.

## Checked-in references

- `issue-3-project-inbox-light.png` and `issue-3-project-inbox-dark.png`: All Projects in both appearances with two projects, one moved-repository status, one retained draft, three newest-created-first chats, deliberately long names, PR metadata, and collapsed merged history.
- `issue-3-empty-light.png` and `issue-3-empty-dark.png`: fresh application data in both appearances with the sole central Open Repository action.
- `issue-3-selected-draft-dark.png`: the long-name project selected with its saved draft editor above two project-filtered chats.
- `issue-3-filtered-zero-dark.png`: a selected project with a real non-matching search query and the resulting zero state; the draft is intentionally hidden while searching.
- `issue-3-unavailable-dark.png`: the moved repository selected, including its actionable warning and disabled draft editor.
- `issue-3-removal-dialog-dark.png`: an eligible project added through the native picker, with an unsent draft and the deletion disclosure; safe Cancel owns initial focus.
- `issue-3-narrow-dark.png`: the same eligible project at the configured minimum window dimensions, with the project list and rail footer both visible.

## Interactive packaged inspection

The following states were exercised in the packaged app through its real Tauri commands and native folder picker on 2026-08-24:

- Empty: launched against fresh temporary application data; the only actionable control was the central Open Repository button.
- Populated: loaded the deterministic two-project/four-chat SQLite fixture through the production snapshot command.
- Project filtered: selected the long Atlas project; its one retained textarea draft appeared above its two active chats.
- Search filtered: entered `#62`; exactly the matching chat remained and the unsent draft was not searched or displayed.
- Long names: verified truncation in the rail, full accessible names, stable chat-row geometry, and a full editable draft heading.
- Repository unavailable: moved Beacon after admission; the rail rendered “Repository unavailable” without exposing a path.
- Invalid folder: selected a real non-Git directory through the native picker; the populated rail rendered the actionable inline error “Choose a folder that contains a Git repository.” and persisted no project.
- Filtered zero result: entered `no-matching-chat` in a selected project and observed the rendered “No matching chats” state with metadata-oriented recovery guidance.
- Draft durability: changed the Atlas prompt, navigated to All Projects and back, closed the packaged app, relaunched against the same application data, and observed the retained replacement draft.
- Removal rule: controls for projects with unmerged chats exposed their blocked reason to the accessibility tree. A third real repository with no chats was opened through the native picker, given an unsent draft, and used to render the confirmation. Focus entered on Cancel, Tab wrapped between the two dialog controls, Escape cancelled, and focus returned to the triggering remove control. The destructive confirmation was not accepted during visual QA.
- Narrow layout: resized the packaged window to the configured 880 × 600 minimum. The rail kept an independently constrained project region, the footer remained visible, long names stayed contained, and the draft and empty-chat states remained usable.
- Appearance: switched macOS from the original dark appearance to light while Più was running, inspected and recaptured the populated and empty states in both, then restored dark.

## Contrast checks

CSS token contrast was recalculated against both adjacent surfaces. The 10px secondary token is 4.65:1 or better in light appearance and 5.19:1 or better in dark appearance. The focus-ring token is 5.36:1 or better in light appearance and 5.80:1 or better in dark appearance, above the 3:1 non-text threshold.

## Reproduction

```sh
review_app_data="$(mktemp -d /tmp/piu-issue3-review.XXXXXX)"
cargo run --manifest-path src-tauri/Cargo.toml --example seed_inbox_review -- "$review_app_data"
npm run build
env PIU_TEST_APP_DATA_DIR="$review_app_data" \
  src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app/Contents/MacOS/piu
```

The review app-data directory is isolated and disposable. The extra removal-dialog repository was admitted through the production native picker rather than seeded. The final packaged app was rebuilt and smoked without fixture data.
