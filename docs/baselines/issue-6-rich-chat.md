# Issue 6 rich chat baseline

Recorded on 2026-08-25 on a MacBook Pro with Apple M5 Pro, 64 GB memory, and macOS 26.6.2. The packaged review window was held at 1180 × 761 points.

## Packaged interaction and visual review

The final Apple Silicon `Più.app` was exercised against isolated application data through the production Tauri commands. The review covered first send, exact-session follow-up send, streaming reasoning and text, tool completion, collapsed usage, project/chat navigation, the native chat menu, sidebar resize, title-bar window drag, and full process relaunch restoration. The startup race was also exercised with the event subscriber active before the opening snapshot; the completed assistant text rendered once.

The transcript uses locally adapted AI Elements message, reasoning, and tool primitives pinned to upstream commit `6a9d5b1822ffb10bba4bd97175f01edd7d8651cd`, with its Apache-2.0 notice checked in. Its visual grammar follows the T3 Code reference at commit `5d7665396083d285132d67038813862a93337ca5`: centered 48 rem content, right-aligned user bubbles, flat assistant output, subordinate activity rows, and a floating composer. The controller and native Pi transport remain Più-owned. Light and dark appearances used the same grid, hierarchy, focus treatment, and semantic colors. macOS was restored to Light after review.

The checked-in references are native macOS PNG window captures of the final packaged application:

- `issue-6-rich-chat-light.png`
- `issue-6-rich-chat-dark.png`
- `issue-6-unread-completion-light.png`
- `issue-6-unread-completion-dark.png`

## Packaged background completion and unread evidence

The normal packaged product was restored after the performance run and verified from fresh isolated application data before this review: its accessibility root was `Più`, not the performance entry, and it opened the production empty inbox. A real local repository with a bare `origin/main` was then admitted through the native repository picker. Only the packaged Pi launcher resource was temporarily redirected through the existing `chat-runtime-child.zsh` streaming fixture; the Tauri host, event forwarding, React application, activity controller, inbox, and conversation surfaces remained the production package. No product source was changed, and the normal launcher was restored byte-for-byte after capture.

The fixture delayed its terminal `agent_end` by five seconds so the transition could be observed rather than preseeded. The selected chat first rendered as `running`. A second turn was sent and the project surface was selected immediately; while hidden, the same row remained in place as `running` with `Value: off`. After `agent_end`, the unchanged row exposed the exact accessible name `Inspect the runtime, finished, unread`, remained `Value: off`, and did not reorder. The title gained the unread weight treatment while the status indicator transitioned to the semantic finished color. The Light and Dark references capture that same background-completion state at 1180 × 761 points. Selecting the row afterward changed its accessible name to `Inspect the runtime, finished` with `Value: on`, confirming that selection clears unread without changing the finished state.

The unread treatment is visually restrained but clear at inbox density: heavier title weight carries unread ownership, the finished dot communicates the terminal state separately, and neither appearance introduces a competing badge or layout shift. Contrast, alignment, and truncation remained coherent in both appearances; no design blocker was observed.

## Runtime and recovery evidence

Fixture and host integration tests cover steer ordering, stop, tool ownership, reasoning and text deltas, typed Pi extension input, tool and turn failure, process interruption, attachment delivery, and immutable persisted history. A fresh host opening the exact stored session with an unresolved running tool restores the tool and turn as interrupted and keeps the prompt count at one, proving that recovery does not replay the prompt.

Current-schema integration tests cover attachment restoration, cancellation, unsupported model media, inaccessible and oversized files, UTF-8 validation, image-only chat titles, concise first-prompt titles, and rename invariants. Più is greenfield, so this work replaces the current schema directly and adds no migration machinery.

## Production performance

The packaged WKWebView performance harness used React's production profiling renderer with 24 chats, 180 transcript entries per conversation, progressive streaming, and three 2 MiB image attachments in the active draft. The 60 measured attachment-heavy composer inputs reached the next frame in 17 ms median and 19 ms p95/max, with React commits at 0 ms median and 1 ms p95/max.

The 119 measured scrolling intervals ran at 60.01 fps with 17 ms median and 19 ms p95/max, with no interval over 20 ms. Simulated streaming ran at 60.04 fps with 17 ms median and 19 ms p95/max, also with no interval over 20 ms. Streaming and scrolling React commits were 0 ms median and at most 3 ms.

After one explicit warm transition per locally available surface, chat switches measured 83 ms median, 85 ms p95, and 86 ms max while the underlying React commits remained 0 ms median, 2 ms p95, and 3 ms max. Project/composer navigation measured 33 ms median, 35 ms p95, and 36 ms max, with React commits at 2 ms median and 3 ms p95/max. Every measured interaction stayed within the initial navigation budgets.

Each navigation sample includes the first animation frame after the expected production UI is present. The corrected uncached transcript measurement initially reached 102 ms despite a 4 ms maximum React commit, identifying Virtuoso's repeated item measurement as the delay. Più now saves and restores Virtuoso's measured list state and manual scroll position for the 32 most recently visited chats. The runner persists its JSON report, then fails if a visible chat switch exceeds 100 ms, navigation or composer input exceeds 50 ms, or scrolling or streaming records a frame over 20 ms.

Run the deterministic packaged measurement with:

```sh
node scripts/measure-chat-performance.mjs
```

The harness restores the normal production package after recording `work/chat-performance-result.json` and the animation-hitch trace.
