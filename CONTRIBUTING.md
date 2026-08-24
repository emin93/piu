# Contributing

Più accepts focused changes that implement an approved GitHub issue and preserve the product vocabulary and architectural decisions.

Before coding:

1. Read [CONTEXT.md](CONTEXT.md).
2. Read the relevant [architectural decisions](docs/adr).
3. Read [implementation principles](docs/implementation-principles.md).
4. Confirm the issue is unblocked and marked `ready-for-agent`.

Every change must include proportionate behavioral tests, pass formatting, linting, type checking, Rust checks, and the full test suite, and leave the application runnable. UI changes also require packaged-app inspection, light and dark appearance verification, keyboard and focus checks, and screenshots for independent design critique.

Commit one coherent issue-sized change at a time. The commit message should reference its issue.
