# Bundled Node and Pi runtime

Più packages the exact Node and Pi release recorded in `runtime-lock.json` and
the complete npm graph recorded in `package-lock.json`. `npm run
runtime:provision` downloads the official Node archive, verifies its SHA-256,
installs the Pi graph with that downloaded Node and its bundled npm, verifies
the result, and atomically publishes the generated `runtime/` directory.

`runtime/` and `.cache/` are build products and are intentionally excluded from
Git: the uncompressed official Node executable exceeds GitHub's per-file size
limit. A clean checkout provisions them before Tauri reads its resource map.
The application bundle receives this fixed layout:

```text
Resources/agent-runtime/
├── node/bin/node
└── pi/
    ├── launcher/ (once launcher source is present)
    ├── node_modules/
    ├── package.json
    └── package-lock.json
```

Launcher source belongs in `launcher/`. Provisioning copies it without
transformation to `pi/launcher/`, where host code resolves it beside
the locked public Pi packages. Runtime launch code must use the absolute Node
path above and must never search `PATH` for Node or Pi.
