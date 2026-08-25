# Issue 5 Pi runtime baseline

Recorded on 2026-08-25 on a MacBook Pro with Apple M5 Pro, 64 GB memory, and macOS 26.6.2. The reference packaged window was 1180 × 761 points unless noted otherwise.

## Checked-in references

- `issue-5-welcome-light.png` and `issue-5-welcome-dark.png`: the project composer before a chat is selected.
- `issue-5-chat-recovery-light.png` and `issue-5-chat-recovery-dark.png`: a chat whose runtime cannot resume, with its retry action and docked composer retained but sending unavailable.
- `issue-5-rejected-send-light.png` and `issue-5-rejected-send-dark.png`: a stopped chat after a rejected send, with the exact draft retained and graphical sign-in kept beside the recovery state.
- `issue-5-conversation-stream-light.png` and `issue-5-conversation-stream-dark.png`: the running conversation with text, collapsed reasoning, usage, and succeeded, running, and failed tool states.
- `issue-5-close-confirmation-light.png` and `issue-5-close-confirmation-dark.png`: the native quit guard shown over an inbox with three active chats.
- `issue-5-codex-sign-in-light.png` and `issue-5-codex-sign-in-dark.png`: the graphical Codex provider handoff.
- `issue-5-codex-sign-in-min-light.png`: the same handoff at the 880 × 601 minimum captured window.

## Packaged launch

`npm run smoke:packaged` verifies bundled Git 2.55.0, Node 24.19.0, and Pi 0.84.3, then starts the normal production `Più.app` with fresh isolated application data. The launch timer starts immediately before spawning Più and stops when the loaded frontend completes the production host round trip and storage-readiness check that emits `piu_shell_ready`; runtime version probes are deliberately outside the launch measurement.

Seven consecutive samples were 298, 278, 265, 268, 272, 276, and 271 ms: 272 ms median and 298 ms p95/max. All were below the 1.5-second usable-shell budget.

## Frontend responsiveness

The checked-in `scripts/performance/chat` harness builds the actual production `InboxWorkspace`, `ChatConversationPanel`, `ConversationSurface`, composer, styles, and lazy conversation boundary into a temporary packaged Tauri application. It swaps only React DOM's standard production renderer for React's production profiling renderer. The harness does not ship in Più's normal bundle, does not add a dependency, and `scripts/measure-chat-performance.mjs` restores the normal package after a run.

The foreground WKWebView ran at 1180 × 761 with one project, 24 locally available chats, and 181 representative messages per opened transcript. After one unrecorded warm chat switch, it measured 30 alternating chat switches, 30 chat-to-project composer navigations, 60 controlled composer input events, 120 scrolling animation frames, and 120 simulated Pi text deltas delivered one per animation frame. Interaction latency runs from DOM activation/input dispatch until the expected production UI is present and the next animation frame is available. Values are milliseconds.

| Scenario | Samples | Min | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Chat switch to visible restored transcript | 30 | 31 | 33 | 35 | 35 |
| Navigation to visible project composer | 30 | 31 | 33 | 36 | 57 |
| Composer input to next frame | 60 | 14 | 16 | 19 | 19 |

React Profiler recorded the following production component commit durations:

| Scenario | Commits | Median | p95 | Max |
| --- | ---: | ---: | ---: | ---: |
| Chat switching | 60 | 1 | 1 | 2 |
| Project/composer navigation | 30 | 1 | 1 | 1 |
| Composer input | 60 | 0 | 1 | 1 |
| Transcript scrolling | 2 | 0 | 1 | 1 |
| Simulated streaming | 120 | 0 | 0 | 1 |

The 119 measured scrolling intervals delivered 60.04 animation frames per second, with 17 ms median, 19 ms p95/max, and no interval over 20 ms. The 119 simulated-streaming intervals delivered 60.01 animation frames per second, with 17 ms median, 19 ms p95/max, and no interval over 20 ms. These results support the 60-fps scrolling and streaming baseline and the sub-100-ms local chat-switch budget. The repeated release trace cleared the earlier composer outlier, reducing its maximum from 59 to 19 ms. Project navigation had one 57-ms next-frame outlier despite a 1-ms maximum React commit; the preceding repeat had one 66-ms navigation outlier. The strict “no operation over 50 ms” navigation gate therefore remains open for the dedicated performance issue rather than being hidden by selecting only the clean samples.

## Method and limitations

Run the deterministic packaged measurement with:

```sh
node scripts/measure-chat-performance.mjs
```

The script writes its non-sensitive JSON result and an attempted local Animation Hitches trace under ignored `work/`, restores the previous clipboard contents, removes isolated application data, and rebuilds the normal production app in `finally`.

This is production WebKit and production Più UI code, but the inbox/transcript data and Pi deltas are deterministic local fixtures; the measurement excludes provider, network, model, and disk latency. Animation-frame cadence measures foreground WKWebView callbacks, not independent display-server frame presentation. Safari WebDriver tracing was unavailable because Safari's “Allow remote automation” setting is disabled on the reference Mac. The attempted Instruments recording was not used to claim a pass because it could not be reliably exported and correlated to individual scripted interactions in this run. A foreground interactive browser/Instruments trace remains the release gate for the isolated navigation outlier and for visual frame-presentation confirmation.

## Verification

The close-confirmation, connection-recovery, rejected-send, and conversation references use explicit, deterministic packaged visual-review inputs over a seeded temporary inbox. The adapters supply only their review snapshot or typed failure; the rendered shell, inbox, conversation store, transcript, tool rows, usage, recovery controls, and composer are the production components. These references verify presentation and interaction hierarchy, while the runtime contracts verify event mapping; they are not evidence of a live provider exchange. The package was rebuilt normally without the review inputs before release validation. The normal production package was also restored after each performance measurement. `npm run smoke:packaged` passed seven consecutive times. The performance harness passes its dedicated TypeScript check, ESLint, Prettier, and production Vite build; the repository's normal frontend bundle remains unchanged by the harness entry.
