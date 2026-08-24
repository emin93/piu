# Più

Più is a native macOS workspace for agentic software development. It runs the Pi agent runtime against either bundled local MLX inference or subscription-backed models while preserving one set of chats, skills, extensions, tools, worktrees, terminals, diffs, and pull-request workflows.

Più is in early development. The product contract lives in [CONTEXT.md](CONTEXT.md), the architectural decisions in [docs/adr](docs/adr), and the implementation standards in [docs/implementation-principles.md](docs/implementation-principles.md).

## Principles

- One Pi runtime across every model route.
- One isolated worktree and branch per chat.
- Opinionated macOS-native behavior with minimal settings.
- Latest stable dependencies selected deliberately and pinned exactly.
- No telemetry, automatic uploads, or silent cloud fallback.

## License

[MIT](LICENSE) © 2026 Emin
