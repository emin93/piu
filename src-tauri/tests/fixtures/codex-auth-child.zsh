#!/bin/zsh
set -eu

record_dir="$PIU_AUTH_FIXTURE_RECORD_DIR"
mode="$PIU_AUTH_FIXTURE_MODE"

pwd -P > "$record_dir/cwd"
print -r -- "$HOME" > "$record_dir/home"
print -r -- "$*" > "$record_dir/arguments"

case "$mode" in
  browser-success)
    print -r -- '{"type":"auth_event","event":{"type":"info","message":"Choose a sign-in method","links":[]}}'
    print -r -- '{"type":"auth_prompt","id":"auth-1","prompt":{"type":"select","message":"Sign in using","options":[{"id":"browser","label":"Browser","description":"Recommended"}]}}'
    IFS= read -r command
    print -r -- "$command" > "$record_dir/command"
    print -r -- '{"type":"auth_complete"}'
    ;;
  all-variants)
    print -r -- '{"type":"auth_event","event":{"type":"info","message":"Read the provider help","links":[{"url":"https://example.test/help","label":"Help"}]}}'
    print -r -- '{"type":"auth_event","event":{"type":"auth_url","url":"https://example.test/auth","instructions":"Continue in the browser"}}'
    print -r -- '{"type":"auth_event","event":{"type":"device_code","userCode":"ABCD-EFGH","verificationUri":"https://example.test/device","intervalSeconds":5,"expiresInSeconds":900}}'
    print -r -- '{"type":"auth_event","event":{"type":"progress","message":"Waiting for authorization"}}'
    print -r -- '{"type":"auth_prompt","id":"auth-text","prompt":{"type":"text","message":"Organization","placeholder":"Example, Inc."}}'
    IFS= read -r first
    print -r -- '{"type":"auth_prompt","id":"auth-secret","prompt":{"type":"secret","message":"One-time secret"}}'
    IFS= read -r second
    print -r -- '{"type":"auth_prompt","id":"auth-manual","prompt":{"type":"manual_code","message":"Paste the callback code","placeholder":"code"}}'
    IFS= read -r third
    print -r -- '{"type":"auth_complete"}'
    ;;
  provider-cancelled-prompt)
    print -r -- '{"type":"auth_prompt","id":"auth-race","prompt":{"type":"manual_code","message":"Paste the callback code"}}'
    print -r -- '{"type":"auth_prompt_cancelled","id":"auth-race"}'
    print -r -- '{"type":"auth_complete"}'
    ;;
  user-cancel)
    print -r -- '{"type":"auth_prompt","id":"auth-cancel","prompt":{"type":"text","message":"Continue"}}'
    IFS= read -r command
    print -r -- "$command" > "$record_dir/command"
    print -r -- '{"type":"auth_prompt_cancelled","id":"auth-cancel"}'
    print -r -- '{"type":"auth_cancelled"}'
    exit 1
    ;;
  provider-failure)
    print -u2 -r -- 'provider failed with sensitive refresh-token'
    print -r -- '{"type":"auth_failed","code":"provider_secret","message":"sensitive refresh-token"}'
    exit 1
    ;;
  malformed)
    print -r -- '{not-json}'
    sleep 10
    ;;
  unknown)
    print -r -- '{"type":"future_auth_record"}'
    sleep 10
    ;;
  extra-field)
    print -r -- '{"type":"auth_complete","credential":"sensitive"}'
    sleep 10
    ;;
  oversized)
    /usr/bin/yes x | /usr/bin/head -c 70000
    sleep 10
    ;;
  malformed-descendant)
    print -r -- "$$" > "$record_dir/parent.pid"
    sleep 10 &
    descendant=$!
    print -r -- "$descendant" > "$record_dir/descendant.pid"
    print -r -- '{not-json}'
    wait
    ;;
  framing)
    /usr/bin/printf '%s' '{"type":"auth_event","event":{"type":"progress","message":"Waiting in Z'
    /bin/sleep 0.01
    /usr/bin/printf '%s\r\n' 'ürich"}}'
    /usr/bin/printf '%s' '{"type":"auth_complete"}'
    ;;
  ignore-cancel-descendant)
    print -r -- "$$" > "$record_dir/parent.pid"
    sleep 10 &
    descendant=$!
    print -r -- "$descendant" > "$record_dir/descendant.pid"
    IFS= read -r ignored
    wait
    ;;
  hang)
    sleep 10
    ;;
  record-after-complete)
    print -r -- '{"type":"auth_complete"}'
    print -r -- '{"type":"progress","message":"must not follow terminal"}'
    sleep 10
    ;;
  complete-with-failed-exit)
    print -r -- '{"type":"auth_complete"}'
    exit 1
    ;;
  complete-without-exit)
    print -r -- '{"type":"auth_complete"}'
    sleep 10
    ;;
  *)
    print -u2 -r -- 'unknown fixture mode'
    exit 64
    ;;
esac
