# Bundled Git runtime

Più packages Git 2.55.0 as an arm64 macOS 15 runtime and never resolves Git
from the user's `PATH`. The runtime is built from the official release artifacts
in `source/` by `build-macos-arm64.sh`. `verify-macos-arm64.sh` checks its
architecture, deployment target, dynamic-library boundary, executable modes,
reported version, local fetch, and worktree creation with a scrubbed environment.

Git is a separate executable distributed under GPL-2.0-only. Its license is
included at `runtime/share/licenses/git/COPYING`; Più's MIT license does not
change Git's license. The official source archive and detached signature are
kept in this repository and should be attached as source artifacts to releases.

The detached signature was made by signing subkey
`E1F036B1FEE7221FC778ECEFB0B5E88696AFE6CB`. Import and validate the maintainer's
key independently before using `gpg --verify` when auditing an upgrade.
