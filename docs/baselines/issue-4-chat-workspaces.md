# Issue 4 chat workspaces baseline

Captured from the packaged Apple Silicon `Più.app` on 2026-08-24 at 1180 × 760 points. The checked-in references exercise the production Tauri commands against a disposable local repository and bare `origin`, not a browser mock or a seeded chat.

## Checked-in references

- `issue-4-setup-progress-light.png` and `issue-4-setup-progress-dark.png`: the executable `.piu/setup.sh` streaming output while the isolated worktree setup is running.
- `issue-4-setup-failure-light.png` and `issue-4-setup-failure-dark.png`: exit code 17 persisted as an actionable failure with Retry setup and Open Terminal.
- `issue-4-setup-cancelled-light.png`: a retry cancelled from the packaged UI, retaining the worktree and the same recovery actions.

## Real repository fixture

The review repository used a local bare remote with `main` at `45f0376d0e7b6643cec9ad595a29ee7ecc6f78d4`. Its executable setup script printed 36 bounded progress lines at 250 ms intervals, wrote one stderr diagnostic, and exited with code 17. Only the repository's six-part stored identity was inserted into fresh isolated application data to avoid coupling issue #4 evidence to macOS picker automation; the first send, forced fetch, worktree, branch, durable message, setup process, retry, cancellation, events, and UI all crossed their production paths.

The created chat used branch `agent/31076574-add-resilient-parser-diagnostics-to-the-c`. Its worktree `HEAD` and the fresh bare `origin/main` both resolved to the commit above. SQLite contained exactly the first immutable user message, “Add resilient parser diagnostics to the command pipeline,” after the chat and worktree became durable. Relaunching the packaged app retained the failed chat and exposed Retry setup; retry incremented the persisted attempt from one to two while preserving the same worktree.

## Interactive inspection

- Watched setup transition from pending to running and stream stdout into the bounded log surface without blocking the window.
- Observed a visible running status in both the selected chat and inbox row, plus the Cancel setup action.
- Waited for the real script to exit 17 and verified the specific failure copy, retained log, Retry setup, and Open Terminal action without any user-facing path.
- Closed and relaunched the application against the same isolated app data, reselected the failed chat, and verified the failure and recovery actions survived restart.
- Retried after relaunch, observed a new streaming attempt in the same worktree, cancelled it, and verified the preserved-worktree recovery state.
- Switched macOS from its original dark appearance to light while Più was running, inspected progress and failure in both appearances, then restored dark.
- Confirmed the generated branch is visible in the inbox metadata while the managed worktree location remains hidden.

## Verification and performance

The final production check covered formatting, ESLint, TypeScript, 50 frontend tests, 68 Rust tests, strict Clippy, bundled Git 2.55.0 verification, and production bundle budgets. The packaged smoke probe reached the native host marker in 991 ms. Vite emitted a 447,487-byte initial JavaScript entry (140,657 bytes gzip) and a 55,596-byte stylesheet (10,644 bytes gzip), with conversation, diff, files, terminal, and settings retained as deferred entries.

Setup output retention is capped at 256 KiB and transported through a bounded channel. The real-Git suite additionally exercises a 1.5 MB setup stream, concurrent worktree creation, each durable crash checkpoint, exact worktree instance replacement, repository replacement, tracked and untracked recovery modifications, branch collision, signal and exit failures, cancellation, missing and non-executable scripts, an invalid shebang, retry after relaunch, and reader failure propagation.
