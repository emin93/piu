# Implementation principles

Più is a greenfield Apple Silicon macOS application. The implementation should be small, direct, and unsurprising to an experienced maintainer.

## Keep Più's code product-specific

- Use maintained, widely adopted libraries for solved problems. Check activity, release quality, documentation, license compatibility, and macOS support before adding one.
- Start implementation and each release update from the latest stable compatible version of every direct dependency and bundled runtime. Pin exact resolved versions in lockfiles and runtime manifests; do not use stale scaffolding defaults, floating `latest` references, or prereleases by accident.
- Do not wrap a library merely to hide its name. Add a seam only when Più has two real adapters or when the seam contains product rules that would otherwise leak into callers.
- Prefer deep modules with small interfaces. Process supervision, chat and worktree management, Pi sessions, inference lifecycle, persistence, and notifications should each hide their invariants from the rest of the application.
- Use the bundled `git`, `gh`, Pi, and oMLX interfaces instead of reimplementing their protocols or source-control behavior.
- Do not add compatibility code for Intel Macs, Windows, Linux, MCP, an embedded editor, arbitrary model layouts, shared Pi configuration, or automatic updates in the first release.
- Do not preserve an early implementation once the design proves wrong. Replace it with the clean design while the project is still greenfield.
- Keep operational complexity behind modules. Do not expose paths, ports, context limits, inference tuning, model layout, process policies, storage policies, or retry parameters as user-facing settings.

## Verify behavior continuously

- Test module behavior through the same interfaces used by production callers.
- Cover chat creation, process ownership, stop behavior, crash recovery, worktree cleanup, project removal, model and reasoning switching, PR merge detection, automatic archive, and credential boundaries with integration tests.
- Run the packaged application at every UI milestone. Exercise real navigation, chat switching, streaming, tool calls, diffs, terminal interaction, empty states, failures, interruption, and relaunch recovery.
- Capture screenshots of every material view and state at a consistent window size. Compare them across milestones so visual regressions are visible.
- Ask an independent design-review subagent to critique each milestone from the screenshots and running behavior. Resolve its concrete findings before treating the milestone as complete.
- Check keyboard operation, focus visibility, contrast, reduced motion, truncation, resizing, loading states, empty states, and error recovery before release.
- Enforce initial performance budgets on the supported reference Mac: usable shell within 1.5 seconds, locally available chat switches visible within 100 milliseconds, no input or navigation operation over 50 milliseconds, and 60 frames per second during composer input, scrolling, and streaming. Record hardware and measurements, investigate regressions above 10 percent, and tighten budgets after representative baselines exist.

## Keep React fast

- Every agent that writes or reviews React code must apply the `vercel-react-best-practices` skill. Apply its client, rendering, bundle, rerender, and JavaScript rules to Più's Tauri frontend; do not cargo-cult Next.js or server-rendering rules into a client-only desktop application.
- Start independent asynchronous work together and await it only where needed. Do not build sequential loading chains for project, chat, Git, and runtime state.
- Import components directly, defer heavy views until opened, and inspect the production bundle. The terminal, diff renderer, file previewers, and settings pages must not inflate the initial chat path unnecessarily.
- Keep high-frequency Pi streaming events out of broad React state. Use narrow subscriptions and derived selectors so a token delta or terminal chunk does not rerender the project list, toolbar, or unrelated messages.
- Virtualize or progressively render long transcripts, diffs, file lists, and terminal history using a maintained library whose behavior is verified in the macOS WebView.
- Measure production builds with React Profiler and browser performance traces. Track launch, chat switching, composer input, streaming, scrolling, and diff opening on the supported hardware rather than accepting subjective impressions.
- Treat unnecessary dependencies, duplicate state, broad context providers, effect-driven derived state, unstable callback chains, barrel imports, and avoidable client waterfalls as review failures.

## Keep the interface coherent

- Use one visual system and one interaction vocabulary across onboarding, inbox, chat, review, files, terminal, and settings.
- Let status determine emphasis. Work that needs the user should be prominent; work that is running should remain visible without demanding attention; completed work should recede without disappearing.
- Give each control one stable name that matches the resulting status and confirmation message.
- Favor opinionated defaults. Add a setting only when different users have a persistent, legitimate need for different behavior.
- Ship one complete light theme and one complete dark theme that follow macOS appearance changes live. Do not expose a theme override. Every state and embedded view must use the same design tokens and pass the same visual review gates.
