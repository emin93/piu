# Issue 2 foundation baseline

Recorded on 2026-08-24 on an Apple M5 Pro MacBook Pro with 64 GB memory and macOS 26.6.2. The packaged window was held at the configured 1180 × 760 points.

## Production bundle

Vite emitted a 197,833-byte initial JavaScript entry (62.70 kB gzip) and a 6.08 kB stylesheet (2.01 kB gzip). Conversation, diff, files, terminal, and settings each remained a separate dynamic entry. `npm run bundle:check` verifies that split against the production manifest.

## Packaged launch

`npm run smoke:packaged` launches the `.app` executable with isolated temporary application data and waits for the production command/event round trip. The first post-build run reached the ready marker in 679 ms. Three subsequent runs were 251 ms, 248 ms, and 246 ms (248 ms median), below the initial 1.5-second usable-shell budget.

## Visual inspection

The packaged empty inbox was inspected at the same window size in both system appearances. Keyboard focus is visible on Open Repository, and activating it opens the native macOS directory picker. The 10–12 px secondary text exceeds 4.5:1 contrast and the focus ring exceeds 3:1 against its adjacent stage in both appearances. Screenshots are recorded beside this file as `issue-2-empty-inbox-light.png` and `issue-2-empty-inbox-dark.png`.

Issue #3 will consume and persist the selected path immediately after integration; this foundation deliberately stops at the picker result and does not add a temporary post-selection workflow. The nested workspace, grid, and empty-state card should be reassessed once populated inbox and chat content provide representative density rather than redesigned from the foundation's empty state.
