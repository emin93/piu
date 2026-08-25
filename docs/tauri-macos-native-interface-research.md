# Native macOS interface behavior in Tauri

Research date: 2026-08-25. This note targets Più's pinned Tauri 2.11.5 shell, WKWebView on macOS 15 or newer, and T3 Code at commit [`e67074f80933a27bd3cdc4e24f486358407690fb`](https://github.com/pingdotgg/t3code/tree/e67074f80933a27bd3cdc4e24f486358407690fb).

## Recommendation

1. Keep a decorated, opaque Tauri window with `titleBarStyle: "Overlay"` and `hiddenTitle: true`. Use AppKit's default traffic-light placement and align a 36-pixel HTML header to those native controls. Use `data-tauri-drag-region="deep"` on the header. Do not recreate the traffic lights, move the window from pointer handlers, or patch AppKit views.
2. Make interface chrome nonselectable at its own container boundaries. Leave chat messages, tool output, errors, diffs, terminal output, and form values selectable. Do not disable selection on the document root.
3. Use Tauri's built-in native `Menu.popup()` for chat and project context menus. Keep one typed product action list and render it from both the row menu and the visible overflow action. Do not use a DOM menu for secondary click.
4. Keep the Tauri window theme unset and drive tokens with `color-scheme: light dark` and `prefers-color-scheme`. This follows the Mac live without a setting or a second appearance store.
5. Do not enable whole-window transparency in the default build yet. Tauri has a direct sidebar-material path, but it requires a private WKWebView API and makes the whole window composited. Prototype it on the packaged app and keep it only if it passes Più's idle energy, resize, scrolling, and reduced-transparency checks.

## Window chrome

Tauri's documented macOS route is a decorated native window with a transparent or overlay titlebar. Tauri warns that removing the native titlebar loses system window behavior and recommends a transparent titlebar when those behaviors matter. This is the right trade for Più because HTML occupies the native-height header while AppKit still owns close, minimize, zoom, window movement, and fullscreen behavior. See [Tauri window customization](https://v2.tauri.app/learn/window-customization/) and the [`WindowConfig` reference](https://v2.tauri.app/reference/config/#windowconfig).

Tauri 2.11.5 implements `data-tauri-drag-region="deep"` directly. It walks the composed event path, lets buttons and other interactive descendants block dragging, prevents text selection when a drag starts, and invokes the native window command. That removes the need for React `mousedown` listeners or a manual `startDragging()` call. The window capability must allow `core:window:allow-start-dragging`. See Tauri's [2.11.5 drag-region source](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/window/scripts/drag.js#L42-L105).

Tauri also exposes `trafficLightPosition` when Overlay and decorations are enabled, but the value is a native inset rather than a CSS circle center. Wry changes the titlebar container height and reapplies the inset during drawing; it does not directly set the buttons' vertical origins. Packaged macOS 26 captures kept the controls at y=10–23 (center 16.5) with both y=26 and y=35, so Più must not claim that a configuration assertion centers them. The stable result is to omit the custom inset and align the HTML header to AppKit's native default. See Wry's [0.55.1 traffic-light implementation](https://github.com/tauri-apps/wry/blob/wry-v0.55.1/src/wkwebview/class/wry_web_view_parent.rs#L60-L111).

The HTML header should use a fixed 36-pixel grid, reserve the traffic-light area on the leading side, and give the trailing side an equal balancing track when the title needs geometric centering. Every blank header area can drag. Buttons, links, inputs, and menus stay interactive because Tauri excludes them from deep dragging.

## Sidebar material and transparency

There is a documented Tauri configuration for real macOS material:

```json
{
  "app": {
    "macOSPrivateApi": true,
    "windows": [
      {
        "transparent": true,
        "windowEffects": {
          "effects": ["sidebar"],
          "state": "followsWindowActiveState"
        }
      }
    ]
  }
}
```

Tauri maps `sidebar` to AppKit's semantic `NSVisualEffectMaterial.sidebar`; Apple says to choose this material by its role rather than by a desired tint. Tauri's built-in effect places one behind-window `NSVisualEffectView` over the native content view and follows window activation by default. Making the web root transparent and the main conversation pane opaque would expose the effect only through the sidebar and any intended titlebar area. This last sentence is an inference from Tauri's implementation, not a separate sidebar-region API. See [Apple's `NSVisualEffectView` guidance](https://developer.apple.com/documentation/AppKit/NSVisualEffectView), Tauri's [effect mapping](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri/src/vibrancy/macos.rs#L12-L75), and the underlying [full-view insertion](https://github.com/tauri-apps/window-vibrancy/blob/window-vibrancy-v0.6.0/src/macos/internal.rs#L16-L59).

This path has real costs:

- Tauri requires `macOSPrivateApi` because Wry disables WKWebView's background through the private `drawsBackground` key. Tauri's config reference says this prevents Mac App Store acceptance. See Wry's [private-API call](https://github.com/tauri-apps/wry/blob/wry-v0.55.1/src/wkwebview/mod.rs#L367-L384) and Tauri's [`transparent` warning](https://v2.tauri.app/reference/config/#windowconfig).
- The native effect covers the full content view. CSS can hide it behind opaque panes, but the window remains transparent. An open upstream Tauri report measured continuous full-window compositing and materially higher GPU power for an otherwise static transparent macOS window. That report needs reproduction on Più's supported Mac before it becomes a product claim, but it is enough to block an unmeasured default. See [tauri-apps/tauri#15471](https://github.com/tauri-apps/tauri/issues/15471).
- Apple says an app must use opaque windows when Reduce Transparency is enabled and exposes a live `accessibilityDisplayShouldReduceTransparency` value plus a change notification. A packaged vibrancy experiment must cover this before shipping. See [`NSWorkspace.accessibilityDisplayShouldReduceTransparency`](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducetransparency).
- On macOS 26, Apple's new native AppKit sidebar glass comes from the split-view hierarchy; Apple specifically warns that a legacy `NSVisualEffectView` can block it. Più's WKWebView layout does not gain that treatment automatically. Do not add a direct Liquid Glass dependency or an OS-specific AppKit hierarchy until Tauri exposes a stable documented path and Più raises its minimum OS. See [WWDC25, Build an AppKit app with the new design](https://developer.apple.com/videos/play/wwdc2025/310/?time=1290).

The smallest stable default is therefore the opaque semantic sidebar already described by Più's token system. A real material experiment should use Tauri's `windowEffects`, not a second vibrancy crate or custom Objective-C bridge, and should be removed if it misses the performance or accessibility gates.

## Text selection

`user-select` is the standard control. The CSS specification recommends applying `user-select: none` only where accidental selection interferes with the intended interaction and explicitly warns against disabling selection at the root. See [CSS Basic User Interface Level 4](https://www.w3.org/TR/css-ui-4/#content-selection).

For Più, apply `user-select: none` to the titlebar, sidebar rows and headings, resize handle, toolbars, tabs, status labels, menu triggers, settings navigation, and other button-like chrome. Keep `user-select: text` or the default behavior on user and assistant message bodies, markdown, code blocks, tool output, errors, diffs, terminal output, and editable controls. This prevents the sidebar-drag selection shown in QA without making useful diagnostic text impossible to copy. The resize interaction should set the nonselection rule only for the duration of pointer capture.

T3 Code uses the same local policy: sidebar rows are `select-none`, resize handles are `select-none`, and copyable previews and logs are `select-text`. It does not use a document-wide ban. See [`Sidebar.tsx`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L1109-L1126) and [`MessagesTimeline.tsx`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/chat/MessagesTimeline.tsx#L785-L905).

## System appearance

Tauri's window `theme` defaults to the system theme on macOS. WebKit has supported `prefers-color-scheme` since Safari 12.1 and updates the matching styles when macOS appearance changes. The direct implementation is to omit a Tauri theme override, declare `color-scheme: light dark`, and keep both token maps in CSS. Use a Tauri `onThemeChanged` listener only if non-CSS code needs the resolved value, such as a native image renderer. See Tauri's [`theme` configuration](https://v2.tauri.app/reference/config/#windowconfig) and [WebKit's system appearance support](https://webkit.org/blog/8718/new-webkit-features-in-safari-12-1/).

Semantic AppKit materials also adapt to appearance. If the vibrancy experiment survives its gates, use `sidebar` with `followsWindowActiveState`. Do not select deprecated `light` or `dark` materials.

## Native context menus

Tauri 2 includes native context menus in its core API. `Menu.popup()` opens a menu at the current pointer or at a logical position relative to the window; no plugin or custom AppKit bridge is required. Più's existing `core:default` capability already includes `core:menu:default`. See Tauri's [menu API](https://v2.tauri.app/reference/javascript/api/namespacemenu/#popup) and [Rust `ContextMenu` contract](https://docs.rs/tauri/2.11.5/tauri/menu/trait.ContextMenu.html).

Use the browser `contextmenu` event only to identify the target, prevent the web menu on that row, and open a Tauri menu. Let normal selection and the system edit menu continue to work inside transcript text. Tauri's predefined Copy and Select All items are preferable to clipboard reimplementations when Più supplies an edit menu.

Apple recommends short, relevant context menus, consistent availability, visible access to the same commands elsewhere, and placing destructive actions last. Tauri does not expose a macOS destructive-text style, so Più should not fake red native menu text. Put Delete Chat last, separate it, and use the product's explicit confirmation dialog. See Apple's [context-menu guidance](https://developer.apple.com/design/human-interface-guidelines/context-menus).

One product action builder should supply both the sidebar row and chat-header menus. T3 Code uses this exact separation: [`threadActionMenu.logic.ts`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/threadActionMenu.logic.ts#L1-L145) owns labels, ordering, destructive placement, and capability gates; [`Sidebar.tsx`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L3040-L3250) maps the selected native-menu action to product behavior.

## T3 Code sidebar precedent

T3 Code's current sidebar is a global thread inbox, not a permanently nested project tree:

- A fixed header combines Search and New Thread, followed by one project-scope picker and New Project. With one project, New Thread creates immediately; with several, it opens the project picker. See [`Sidebar.tsx` lines 3348-3585](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L3348-L3585).
- Selecting a project filters the global list. Each full chat row still shows its project favicon and name, chat title, branch, PR, diff summary, provider, and activity. See the [scope state](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L1941-L2008) and [card composition](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L1400-L1595).
- Unsent drafts and pinned items sit first. Active items follow. Snoozed and Settled are collapsible history shelves. Archived items are omitted. See the [partition](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L2015-L2110) and [section rendering](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L3750-L3928).
- Activity never reorders active rows. They stay newest-created-first. Approval, input, working, failed, unread completion, and wake state change the row's label and emphasis. In-flight work recedes so work that needs the user wins attention. See [`Sidebar.logic.ts`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.logic.ts#L482-L576) and the [row-emphasis rules](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/components/Sidebar.tsx#L820-L945).
- Project copies from different execution environments can be grouped into one logical picker entry. That is T3-specific multi-environment state, not a visual requirement. See [`sidebarProjectGrouping.ts`](https://github.com/pingdotgg/t3code/blob/e67074f80933a27bd3cdc4e24f486358407690fb/apps/web/src/sidebarProjectGrouping.ts#L1-L119).

Più should adopt the compact information architecture, stable ordering, local status emphasis, and shared action-builder pattern. It should not copy pinning, snoozing, settlement, multi-environment grouping, remote-server capabilities, manual order, or T3's title-only search. Più's direct version is simpler: one retained draft per project, one project filter over a global newest-created-first list of unmerged chats, transient activity and unread cues on each row, and a separate collapsible Merged history. Search follows Più's own contract across chat title, project, branch, and PR number. Chat actions are Rename Chat and Delete Chat, with the PR action remaining visible in the chat toolbar; project actions follow Più's New Chat, project details, and guarded Remove Project rules.

## Acceptance checks

- Drag every blank point in the 36-pixel header, including nested title text, in focused and unfocused windows. Confirm controls remain clickable and no text selection appears.
- Measure native traffic-light centers and the HTML title at the supported packaged window size in both appearances. Keep the geometry in configuration and CSS, not pointer code.
- Drag the sidebar splitter across text. Confirm the pointer stays `col-resize`, only the one-pixel divider is visible, and selection is restored when pointer capture ends.
- Secondary-click and Control-click chat and project rows. Confirm the native menu targets the clicked row, keyboard focus returns sensibly, Delete is last, and transcript selection keeps its edit behavior.
- Switch system appearance and window activation while the app is open. Verify tokens, form controls, context menus, and any native material update without relaunch.
- If transparency is prototyped, compare packaged opaque and transparent builds at idle, during resize, during transcript scroll, and during streaming. Test Reduce Transparency live. Reject the feature if it breaks Più's performance budgets or cannot become effectively opaque for accessibility.
