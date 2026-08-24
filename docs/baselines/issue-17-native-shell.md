# Issue 17 native shell baseline

Captured from the packaged Apple Silicon `Più.app` at a consistent 1180 × 761 window capture. The review fixture is isolated from normal application data; normal builds never seed product data.

## Checked-in references

- `issue-17-populated-light.png` and `issue-17-populated-dark.png`: the composer-first shell in both system appearances, with the same repository, project context, and retained draft.
- `issue-17-empty-light.png` and `issue-17-empty-dark.png`: fresh application data with disabled inbox controls and Open Repository as the only primary action.
- `issue-17-unavailable-dark.png`: a remembered repository that is no longer available, with the composer replaced by recovery guidance.
- `issue-17-zero-results-dark.png`: a real non-matching search query and its compact empty result.
- `issue-17-removal-dialog-dark.png`: a repository with an unsent draft and the deletion disclosure; safe Cancel owns initial focus.
- `issue-17-startup-failure-light.png`: a packaged launch against an intentionally invalid isolated app-data path, with a keyboard-reachable Retry action.

## Interactive packaged inspection

The following states were exercised through the packaged app and real Tauri commands on 2026-08-24:

- Composer launch: after opening an eligible repository through the native picker, the centered textarea owned focus on first render. The stable `What should we build?` prompt stayed concise with a separate quiet repository context line.
- Empty library: launched against fresh temporary application data. Search, All Projects, and the splitter were disabled and absent from the tab order; the first Tab focused Open Repository.
- Native picker: cancelled once without changing state, then admitted a real local Git repository.
- Draft durability: typed a project draft, navigated between the project and All Projects, and observed the retained per-project text and saved state.
- Repository unavailable: moved an admitted repository and observed the actionable unavailable composition without an accepting textarea.
- Search: entered a real non-matching query and observed the compact `No matching chats` result.
- Removal dialog: focus entered on Cancel, Tab wrapped between Cancel and Remove, Escape closed the dialog, and focus returned to the triggering project action. The destructive confirmation was not accepted during visual QA.
- Sidebar keyboard behavior: focused the splitter, used Right Arrow to resize 256 → 272 px, then Left Arrow to restore 256 px. Focus visibility stayed confined to the short handle instead of painting the full window seam.
- Narrow layout: inspected the populated shell at the configured 880 × 600 minimum window size; the fixed hierarchy, composer, and rail remained usable.
- Appearance: changed macOS from Light to Dark while Più was running and observed the app update immediately without a product theme setting. Captured the relevant states in both appearances, then restored the original Light appearance.
- Startup failure: launched with `/dev/null` as an intentionally invalid, isolated test app-data path. The recovery composition rendered without stale inbox controls and the first Tab focused Retry.

The transient startup-loading composition is covered by the frontend accessibility test; packaged readiness is measured independently because the production initialization completed before Computer Use could acquire a window snapshot.

## Build and performance evidence

Measured from the final release build:

- Initial JavaScript: 440,060 bytes raw / 138,556 bytes gzip, below the 512 KiB raw / 160 KiB gzip limits.
- Initial CSS: 51,421 bytes raw / 10,026 bytes gzip, below the 64 KiB raw / 16 KiB gzip limits.
- Fonts: exactly two local Latin Geist variable WOFF2 assets; no remote font request or asset URL.
- Deferred surfaces: five feature-heavy surfaces remain outside the opening route.
- Packaged cold readiness: 317 ms on a MacBook Pro with Apple M5 Pro and 64 GB memory, measured by `npm run smoke:packaged` against isolated fresh app data.

## Verification

`npm run check`, `npm run build`, and `npm run smoke:packaged` pass. The frontend suite covers launch focus, keyboard navigation, system appearance, reduced motion, retained and failed drafts, stale snapshot reconciliation, retry, removal focus, local fonts, and the bundle ceilings.

## Reproduction

```sh
review_app_data="$(mktemp -d /tmp/piu-issue17-review.XXXXXX)"
cargo run --manifest-path src-tauri/Cargo.toml --example seed_inbox_review -- "$review_app_data"
npm run build
env PIU_TEST_APP_DATA_DIR="$review_app_data" \
  src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app/Contents/MacOS/piu
```

The review app-data directory and added repositories are local, isolated, and disposable. The final packaged smoke uses separate fresh app data and removes it after launch.
