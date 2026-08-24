# Bridge Pi authentication to macOS Keychain

Più will bundle a small Node launcher that constructs Pi sessions through Pi's public runtime APIs and passes the resulting runtime to Pi's official native RPC runner. The launcher supplies Pi's public `CredentialStore` interface backed by macOS Keychain instead of allowing Pi to create `auth.json`. Credential updates are serialized per provider across processes so OAuth refresh-token rotation remains correct.

Because Pi's native chat RPC has no authentication commands, sign-in and sign-out use a separate short-lived helper built from the same pinned Pi packages. It calls the public `ModelRuntime.login()` or logout API and exposes only Pi's public authentication prompt and notification contract to the Tauri host. Browser URLs, device codes, manual-code prompts, progress, cancellation, and errors cross that boundary; access tokens, refresh tokens, and API keys do not. The helper exits after the operation. Active chat children continue to use only Pi's official native JSONL RPC protocol.

Pi retains ownership of provider login, refresh, model, tool, skill, extension, session, and RPC behavior; Più owns only Keychain persistence, cross-process serialization, process supervision, and the graphical interaction callbacks. The desktop process does not embed the Pi SDK.

This is preferred to copying Pi's CLI bootstrap, storing OAuth credentials in a private plaintext file, passing long-lived tokens through environment variables, or introducing the AI SDK Pi Harness. The launcher is an application boundary required by Più's credential policy, not a second agent runtime or protocol.
