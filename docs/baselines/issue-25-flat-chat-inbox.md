# Issue 25 flat chat inbox baseline

Recorded on 2026-08-26 on a MacBook Pro with Apple M5 Pro, 64 GB memory, and macOS 26.6.2. The packaged review window was held at 1180 x 761 points.

## Checked-in references

- Active chat: `issue-25-active-light-compact.png` and `issue-25-active-dark-wide.png`
- Empty inbox: `issue-25-empty-light-compact.png` and `issue-25-empty-dark-compact.png`
- Retained draft: `issue-25-draft-light-compact.png` and `issue-25-draft-dark-wide.png`
- Search result: `issue-25-search-light-compact.png` and `issue-25-search-dark-wide.png`
- Chat menu: `issue-25-menu-light-compact.png` and `issue-25-menu-dark-wide.png`
- Delete confirmation: `issue-25-confirmation-light-compact.png` and `issue-25-confirmation-dark-wide.png`
- Successful deletion: `issue-25-deletion-success-light-compact.png` and `issue-25-deletion-success-dark-wide.png`
- Safe deletion failure: `issue-25-deletion-failure-light-compact.png` and `issue-25-deletion-failure-dark-wide.png`

Every reference is a native macOS PNG window capture at 2x resolution, 2360 x 1522 pixels. The Light references use the compact 256-point sidebar. The Dark references use the widened 328-point sidebar except for the empty inbox, where the splitter is correctly disabled because there is nothing to navigate. `issue-25-flat-chat-light.png` and `issue-25-flat-chat-dark.png` remain the canonical active-chat references. The review used isolated application data and a development-only active-chat fixture. Normal builds never seed product data.

The conversation follows Più's locally adapted AI Elements message grammar: restrained right-aligned user turns, full-width assistant output, subordinate reasoning and tool activity, and a docked composer. The sidebar keeps one newest-created-first chat list below its fixed search, New Chat, project scope, and Open Repository controls. It does not restore the former Projects, Drafts, and Chats shelves.

## Packaged interaction review

Computer Use exercised the packaged Apple Silicon application through production Tauri commands:

- Dragged blank and titled points in the 36-pixel native overlay header. The window moved through AppKit's native drag path, no title text selected, and the native traffic lights, wordmark, and centered conversation title stayed vertically aligned.
- Dragged the sidebar splitter from 256 to 328 pixels and back. The accessible splitter value tracked both moves, the one-pixel divider stayed quiet, and no sidebar text selected.
- Opened the same Rename and Delete menu from secondary click and the row overflow button. Rename selected the current title and kept branch and worktree identity unchanged.
- Opened the Delete confirmation and confirmed that Cancel owned initial focus. The disclosure described permanent local conversation, managed worktree, and local branch removal, plus active-process shutdown.
- Exercised a deliberately invalid managed-worktree identity. Deletion failed inline and retained the chat record for retry.
- Exercised successful deletion against an isolated real managed worktree. The row disappeared, focus moved to its stable neighbor, the worktree and local branch were gone, and both the chat and deletion-journal rows were absent. The remote branch was untouched.
- Filtered the inbox by a real title query and cleared it without changing row order.
- Verified fresh-profile model discovery failure in both new-chat and active-chat composers. The draft stayed editable, Sign in to Codex opened the provider-owned login dialog, Cancel made no account changes, and retry remained available. Component coverage proves that a successful sign-in revision reloads the route and effort controls, preserves the exact draft, and enables Send.
- Selected transcript text while sidebar, titlebar, splitter, toolbar, and status text remained nonselectable. Nonselection is owned by those local chrome containers rather than the document root, so composer errors, tool output, future diff output, and terminal output remain copyable.
- Changed macOS from Light to Dark while the app was open and observed an immediate update without a Più theme setting. macOS was restored to Light after review.

An independent design critique found no merge-blocking visual defect. It specifically confirmed the titlebar alignment, understated divider, flat inbox hierarchy, chat presentation, composer regions, and consistent system appearances. Recovery actions use the foreground token while the associated error copy alone uses the destructive token.

## Production performance

The packaged WKWebView performance runner used React's production profiling renderer with 24 chats, 180 transcript entries per conversation, progressive streaming, and three 2 MiB image attachments in the active draft.

- Locally available chat switches: 68 ms p95 and max, below the 100 ms budget. React commits were 2 ms p95 and 4 ms max.
- Project/composer navigation: 34 ms p95 and max, below the 50 ms budget.
- Composer input: 18 ms p95 and max, below the 50 ms budget.
- Scrolling: 60.01 fps with no frame over 20 ms.
- Streaming: 60.04 fps with no frame over 20 ms.
- Streaming rerender isolation: zero unrelated chat-row, project-scope, or inference-control renders. One intended target-row render recorded its activity update.

The recent-chat session cache retains at most three transcript/controller sessions. A cached conversation renders immediately while its production Pi transport reconnects; sending, stopping, attachments, and inference controls stay disabled until reconnection. Cache reads remain pure during render, and eviction/disposal occurs only after commit.

Run the deterministic measurement with:

```sh
node scripts/measure-chat-performance.mjs
```

The runner writes `work/chat-performance-result.json` and the animation-hitch trace, then restores the normal production package.

## Verification

```sh
npm run check
npm run build
npm run smoke:packaged
node scripts/measure-chat-performance.mjs
```

The visual review package was rebuilt normally without `VITE_PIU_VISUAL_REVIEW_STATE` before the final packaged smoke test.
