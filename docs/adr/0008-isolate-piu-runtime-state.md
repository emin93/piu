# Isolate Più runtime state

Più will provide a self-contained agent environment by giving its bundled Pi runtime an application-owned configuration and session directory. It will not read, import, modify, or synchronize the `~/.pi/agent` state of a separately installed Pi CLI. Project-owned resources remain available from the chat worktree, but every global model route, credential, skill, extension, package, setting, and session shown in Più belongs to Più.
