#!/bin/zsh

set -euo pipefail

git_runtime_dir=${1:-${0:a:h}/runtime}
git_executable="$git_runtime_dir/bin/git"
git_exec_path="$git_runtime_dir/libexec/git-core"
git_template_dir="$git_runtime_dir/share/git-core/templates"

if [[ ! -x $git_executable ]]; then
  print -u2 "Bundled Git executable is missing or not executable: $git_executable"
  exit 1
fi

for executable in "$git_executable" "$git_exec_path/git-remote-http"; do
  if [[ $(file -b "$executable") != *"arm64"* ]]; then
    print -u2 "Bundled executable is not arm64: $executable"
    exit 1
  fi
  if [[ $(otool -l "$executable" | awk '/minos/{ print $2; exit }') != 15.0 ]]; then
    print -u2 "Bundled executable does not target macOS 15.0: $executable"
    exit 1
  fi
  if otool -L "$executable" | tail -n +2 | awk '{ print $1 }' | grep -Evq '^(/System/Library/|/usr/lib/)'; then
    print -u2 "Bundled executable has a non-system dynamic-library dependency: $executable"
    exit 1
  fi
done

while IFS= read -r -d '' runtime_file; do
  file_kind=$(file -b "$runtime_file")
  case $file_kind in
    *Mach-O*|*executable*|*script*)
      if [[ ! -x $runtime_file ]]; then
        print -u2 "Bundled Git runtime program is not executable: $runtime_file"
        exit 1
      fi
      ;;
  esac
done < <(find "$git_runtime_dir/bin" "$git_runtime_dir/libexec/git-core" \
  -type f ! -name git-sh-i18n ! -name git-sh-setup ! -name git-mergetool--lib -print0)

git_env=(
  env -i
  HOME="${TMPDIR:-/tmp}/piu-git-verification-home"
  PATH=/usr/bin:/bin
  LC_ALL=C
  GIT_CONFIG_NOSYSTEM=1
  GIT_TERMINAL_PROMPT=0
  GIT_EXEC_PATH="$git_exec_path"
  GIT_TEMPLATE_DIR="$git_template_dir"
)

if [[ $("${git_env[@]}" "$git_executable" version) != "git version 2.55.0" ]]; then
  print -u2 "Bundled Git reported an unexpected version."
  exit 1
fi

git_smoke_dir=$(mktemp -d "${TMPDIR:-/tmp}/piu-git-smoke.XXXXXX")
trap 'rm -rf "$git_smoke_dir"' EXIT

"${git_env[@]}" "$git_executable" init --quiet --bare "$git_smoke_dir/remote.git"
"${git_env[@]}" "$git_executable" init --quiet "$git_smoke_dir/source"
"${git_env[@]}" "$git_executable" -C "$git_smoke_dir/source" \
  -c user.name=Più -c user.email=piu@example.invalid \
  commit --quiet --allow-empty -m initial
"${git_env[@]}" "$git_executable" -C "$git_smoke_dir/source" \
  push --quiet "$git_smoke_dir/remote.git" HEAD:main
"${git_env[@]}" "$git_executable" init --quiet "$git_smoke_dir/client"
"${git_env[@]}" "$git_executable" -C "$git_smoke_dir/client" \
  fetch --quiet "$git_smoke_dir/remote.git" main
"${git_env[@]}" "$git_executable" -C "$git_smoke_dir/client" \
  worktree add --quiet --detach "$git_smoke_dir/worktree" FETCH_HEAD
test -f "$git_smoke_dir/worktree/.git"
