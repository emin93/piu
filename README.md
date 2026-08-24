# Più

Più is a native macOS workspace for agentic software development. It runs the Pi agent runtime against either bundled local MLX inference or subscription-backed models while preserving one set of chats, skills, extensions, tools, worktrees, terminals, diffs, and pull-request workflows.

Più is in early development. The product contract lives in [CONTEXT.md](CONTEXT.md), the architectural decisions in [docs/adr](docs/adr), and the implementation standards in [docs/implementation-principles.md](docs/implementation-principles.md).

## Supported system

Più targets Apple Silicon Macs running macOS 15 or newer. Intel Macs and other operating systems are not supported.

## Development prerequisites

- Xcode 16 or newer, including its command-line tools
- Node.js 24.19.0 with npm
- Rust 1.98.0 for `aarch64-apple-darwin` (selected automatically by `rust-toolchain.toml`)

Install JavaScript dependencies with `npm ci`. The standard commands are:

- `npm run dev` — launch the live Tauri development application
- `npm run check` — run formatting, linting, type checks, frontend and Rust tests, Clippy, the frontend production build, and bundle-boundary verification
- `npm run build` — build the Apple Silicon production application at `src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Più.app`
- `npm run smoke:packaged` — launch the packaged executable with isolated temporary data and wait for the production host round trip

## Dependency policy

Più starts each implementation or upgrade from the latest stable compatible release of every direct dependency. Exact versions are pinned in `package.json`, `Cargo.toml`, `rust-toolchain.toml`, and their lockfiles. Prereleases, floating ranges, stale template pins, and compatibility-only packages require an explicit architectural decision.

## Principles

- One Pi runtime across every model route.
- One isolated worktree and branch per chat.
- Opinionated macOS-native behavior with minimal settings.
- Latest stable dependencies selected deliberately and pinned exactly.
- No telemetry, automatic uploads, or silent cloud fallback.

## License

[MIT](LICENSE) © 2026 Emin
