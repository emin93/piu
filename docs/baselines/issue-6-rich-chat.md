# Issue 6 rich chat baseline

Recorded on 2026-08-25 on a MacBook Pro with Apple M5 Pro, 64 GB memory, and macOS 26.6.2. The packaged review window was held at 1180 × 761 points.

## Packaged interaction and visual review

The final Apple Silicon `Più.app` was exercised against isolated application data through the production Tauri commands. The review covered a restored text attachment, native image selection, selection cancellation, preview and removal of text and image files, attachment-only send readiness, project/chat navigation, full process relaunch restoration, an unavailable repository, and a controlled fresh-main failure. Folder selection remained unavailable in the native file picker.

During the controlled slow send, the textarea was read-only and the attach, remove, and send controls were disabled. After the forced failure, every control recovered and the exact draft and both attachments remained available with the typed inline error. Light and dark appearances used the same grid, hierarchy, focus treatment, and semantic colors. macOS was restored to Light after review.

The checked-in references are native macOS PNG window captures of the final packaged application:

- `issue-6-rich-chat-light.png`
- `issue-6-rich-chat-dark.png`

## Runtime and recovery evidence

Fixture and host integration tests cover steer ordering, stop, tool ownership, reasoning and text deltas, typed Pi extension input, tool and turn failure, process interruption, attachment delivery, and immutable persisted history. A fresh host opening the exact stored session with an unresolved running tool restores the tool and turn as interrupted and keeps the prompt count at one, proving that recovery does not replay the prompt.

Current-schema integration tests cover attachment restoration, cancellation, unsupported model media, inaccessible and oversized files, UTF-8 validation, image-only chat titles, concise first-prompt titles, and rename invariants. Più is greenfield, so this work replaces the current schema directly and adds no migration machinery.

## Production performance

The packaged WKWebView performance harness used React's production profiling renderer with 24 chats, 180 transcript entries per conversation, progressive streaming, and three 2 MiB image attachments in the active draft. The 60 measured attachment-heavy composer inputs reached the next frame in 17 ms median, 18 ms p95, and 19 ms max, with React commits at 0 ms median, 1 ms p95, and 2 ms max.

The 119 measured scrolling intervals ran at 60.04 fps with 17 ms median, 19 ms p95, 20 ms max, and no interval over 20 ms. Simulated streaming ran at 60.01 fps with 17 ms median, 19 ms p95/max, and no interval over 20 ms. Streaming and scrolling React commits were 0 ms median and at most 2 ms.

After one explicit warm transition per locally available surface, chat switches measured 84 ms median, 85 ms p95, and 86 ms max while the underlying React commits remained 0 ms median, 2 ms p95, and 3 ms max. Project/composer navigation measured 16 ms median, 19 ms p95/max, with React commits at 2 ms median and 3 ms max. Every measured interaction stayed within the initial navigation budgets.

Run the deterministic packaged measurement with:

```sh
node scripts/measure-chat-performance.mjs
```

The harness restores the normal production package after recording `work/chat-performance-result.json` and the animation-hitch trace.
