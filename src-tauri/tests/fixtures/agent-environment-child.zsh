#!/bin/zsh
set -eu

record_dir="$PIU_ENVIRONMENT_FIXTURE_RECORD_DIR"
mkdir -p "$record_dir"
pwd -P > "$record_dir/cwd"
printf '%s\n' "$@" > "$record_dir/arguments"
printf '%s\n' "$HOME" > "$record_dir/home"
printf '%s\n' "$PATH" > "$record_dir/path"
printf '%s\n' "${GIT_EXEC_PATH:-}" > "$record_dir/git-exec-path"
printf '%s\n' "${GIT_TEMPLATE_DIR:-}" > "$record_dir/git-template-dir"

case "${PIU_ENVIRONMENT_FIXTURE_MODE:-snapshot}" in
  chat-runtime)
    project_skill="$PWD/.pi/skills/check/SKILL.md"
    project_extension="$PWD/.pi/extensions/review.mjs"
    skills='[]'
    extensions='[]'
    if [[ -f "$project_skill" ]]; then
      skills="[{\"id\":\"project://skills/check\",\"name\":\"Check\",\"path\":\"$project_skill\",\"enabled\":true,\"source\":\"local\",\"scope\":\"project\",\"origin\":\"top-level\"}]"
    fi
    if [[ -f "$project_extension" ]]; then
      extensions="[{\"id\":\"project://extensions/review\",\"name\":\"Review\",\"path\":\"$project_extension\",\"enabled\":true,\"source\":\"local\",\"scope\":\"project\",\"origin\":\"top-level\"}]"
    fi
    printf '{"modelRoutes":[{"provider":"openai-codex","id":"gpt-5.6-sol","name":"GPT-5.6 Sol","acceptsImages":true,"thinkingLevels":["off","minimal","low","medium","high","xhigh","max"]},{"provider":"local-mlx","id":"qwen3.8-27b","name":"Qwen 3.8 27B","acceptsImages":false,"thinkingLevels":["low","medium","xhigh"]}],"resources":{"extensions":%s,"skills":%s,"packages":[]},"diagnostics":[]}\n' "$extensions" "$skills"
    ;;
  snapshot)
    /bin/cat <<'JSON'
{"modelRoutes":[{"provider":"openai-codex","id":"gpt-5.6-sol","name":"GPT 5.6","acceptsImages":true,"thinkingLevels":["off","low","high","max"]},{"provider":"local","id":"qwen","name":"Qwen","acceptsImages":false,"thinkingLevels":["low","medium","xhigh"]}],"resources":{"extensions":[{"id":"piu://extensions/review","name":"Review tools","path":"/private/tmp/piu/extensions/review.mjs","enabled":true,"source":"local","scope":"user","origin":"top-level"},{"id":"package://extensions/review","name":"Package review","path":"/private/tmp/piu/packages/review/extension.mjs","enabled":true,"source":"npm:@piu/review@1.0.0","scope":"user","origin":"package"}],"skills":[{"id":"project://skills/check","name":"Check","path":"/private/tmp/project/.pi/skills/check/SKILL.md","enabled":true,"source":"local","scope":"project","origin":"top-level"},{"id":"package://skills/review","name":"Package review","path":"/private/tmp/piu/packages/review/SKILL.md","enabled":true,"source":"npm:@piu/review@1.0.0","scope":"user","origin":"package"}],"packages":[{"id":"npm:@piu/review@1.0.0","name":"npm:@piu/review@1.0.0","source":"npm:@piu/review@1.0.0","scope":"user","filtered":false}]},"diagnostics":[{"resourceType":"skill","type":"warning","message":"Fixture warning","path":"/private/tmp/project/.pi/skills/check/SKILL.md"}]}
JSON
    ;;
  fail)
    print -u2 -- 'fixture inspection failed'
    exit 12
    ;;
  oversize)
    /usr/bin/yes x | /usr/bin/head -c 8192
    ;;
  sleep)
    /bin/sleep 30
    ;;
  *)
    exit 64
    ;;
esac
