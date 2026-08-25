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
- `npm run smoke:packaged` — verify the packaged Git, Node, and Pi runtimes in a scrubbed environment, then launch the packaged executable with isolated temporary data

Development, checks, and builds provision the pinned Node and Pi runtime
automatically. The generated runtime is verified and packaged from
`src-tauri/vendor/agent-runtime`; Più never uses it as a fallback to a system
Node or Pi installation.

Local packages are intentionally unsigned development artifacts. Their nested
executables retain linker ad-hoc signatures, but the application has no final
resource seal until the release-signing and notarization pipeline signs it.

## Dependency policy

Più starts each implementation or upgrade from the latest stable compatible release of every direct dependency. Exact versions are pinned in `package.json`, `Cargo.toml`, `rust-toolchain.toml`, and their lockfiles. Prereleases, floating ranges, stale template pins, and compatibility-only packages require an explicit architectural decision.

The application bundles the official arm64 macOS Git 2.55.0 runtime. Its pinned source, detached signature, build recipe, provenance, and GPL-2.0-only notice live under [`src-tauri/vendor/git`](src-tauri/vendor/git).

The application also bundles official Node.js 24.19.0 and Pi 0.84.3 runtime
artifacts. Their checksums, complete npm lock, provisioning and verification
commands, public-export check, and runtime layout live under
[`src-tauri/vendor/agent-runtime`](src-tauri/vendor/agent-runtime).

## Principles

- One Pi runtime across every model route.
- One isolated worktree and branch per chat.
- Opinionated macOS-native behavior with minimal settings.
- Latest stable dependencies selected deliberately and pinned exactly.
- No telemetry, automatic uploads, or silent cloud fallback.

## License

Più is [MIT](LICENSE) licensed © 2026 Emin. Bundled third-party executables retain their own licenses; Git is distributed separately under GPL-2.0-only as documented in [`src-tauri/vendor/git`](src-tauri/vendor/git).
