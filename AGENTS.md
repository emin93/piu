# Agent workflow

Read [CONTEXT.md](CONTEXT.md) before naming or modeling product concepts. Read the relevant records under [docs/adr](docs/adr) before changing an architectural boundary. Apply [docs/implementation-principles.md](docs/implementation-principles.md) to every change.

Work from one GitHub issue at a time using [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md). Deliver a vertical, independently verifiable slice, keep unrelated changes out, run the narrow checks during development and the complete required checks before committing.

For React work, read and apply the `vercel-react-best-practices` skill before editing. Treat broad rerenders, unnecessary dependencies, sequential async work, eager heavy imports, and unmeasured UI performance as defects. Run the packaged Tauri application and inspect the affected states in both system appearances before considering UI work complete.

Select the latest stable compatible dependency at the start of implementation or a deliberate upgrade, then pin its exact resolved version. Release candidates, floating versions, stale template pins, compatibility workarounds, and speculative abstractions require an explicit ADR.

Più is greenfield. Replace a wrong design directly while its surface is small. Prefer deep product modules over wrapper layers, and prefer maintained libraries for solved problems.
