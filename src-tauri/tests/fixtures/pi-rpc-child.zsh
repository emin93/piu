#!/bin/zsh

set -eu

mode="${PIU_RPC_FIXTURE_MODE:-normal}"
record_dir="${PIU_RPC_FIXTURE_RECORD_DIR:-}"

if [[ -n "$record_dir" ]]; then
  print -r -- "$$" > "$record_dir/parent.pid"
  print -r -- "$PWD" > "$record_dir/cwd"
  print -r -- "${PIU_RPC_FIXTURE_EXPLICIT_ENV:-}" > "$record_dir/environment"
fi

request_id() {
  local line="$1"
  local suffix="${line#*\"id\":\"}"
  print -r -- "${suffix%%\"*}"
}

request_type() {
  local line="$1"
  local suffix="${line#*\"type\":\"}"
  print -r -- "${suffix%%\"*}"
}

hang() {
  sleep 30 &
  wait
}

if [[ "$mode" == "exit-before-readiness" ]]; then
  exit 17
fi

IFS= read -r readiness
readiness_id="$(request_id "$readiness")"

if [[ "$mode" == "never-ready" ]]; then
  hang
  exit 0
fi

if [[ "$mode" == "failed-readiness" ]]; then
  printf '{"id":"%s","type":"response","command":"get_state","success":false,"error":"fixture refused readiness"}\n' "$readiness_id"
  exit 9
fi

printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"fixture-session"}}\n' "$readiness_id"

if [[ "$mode" == "unsolicited-response" ]]; then
  printf '{"id":"never-issued","type":"response","command":"prompt","success":true}\n'
  hang
  exit 0
fi

if [[ "$mode" == "stderr-burst" ]]; then
  dd if=/dev/zero bs=1024 count=64 2>/dev/null | tr '\0' x >&2
fi

if [[ "$mode" == "forced-shutdown" || "$mode" == "graceful-descendant" || "$mode" == "malformed-descendant" ]]; then
  sleep 30 &
  descendant="$!"
  if [[ -n "$record_dir" ]]; then
    print -r -- "$descendant" > "$record_dir/descendant.pid"
  fi
  if [[ "$mode" == "forced-shutdown" ]]; then
    wait
  fi
fi

held_id=""
held_type=""
out_of_order_id=""
out_of_order_type=""

while IFS= read -r line; do
  id="$(request_id "$line")"
  type="$(request_type "$line")"
  case "$mode" in
    normal|stderr-burst|graceful-descendant)
      printf '{"type":"agent_start","fixture":"event-before-response"}\n'
      printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"accepted":true}}\n' "$id" "$type"
      ;;
    framing)
      printf '{"type":"future_event","text":"before'
      printf '\342'
      sleep 0.02
      printf '\200\250middle\342\200\251after"}\r\n'
      printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"framed":true}}' "$id" "$type"
      exit 0
      ;;
    framing-split)
      frame=$'{"type":"future_event","text":"Z\303\274rich \342\200\250 caf\303\251"}\r\n'
      split_at="${PIU_RPC_FIXTURE_SPLIT_AT:?split byte is required}"
      printf '%s' "$frame" | dd bs=1 count="$split_at" 2>/dev/null
      sleep 0.001
      printf '%s' "$frame" | dd bs=1 skip="$split_at" 2>/dev/null
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      ;;
    out-of-order)
      if [[ -z "$out_of_order_id" ]]; then
        out_of_order_id="$id"
        out_of_order_type="$type"
      else
        printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"slot":"second"}}\n' "$id" "$type"
        printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"slot":"first"}}\n' "$out_of_order_id" "$out_of_order_type"
      fi
      ;;
    hold-then-late)
      if [[ -z "$held_id" ]]; then
        held_id="$id"
        held_type="$type"
      else
        printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"late":true}}\n' "$held_id" "$held_type"
        printf '{"id":"%s","type":"response","command":"%s","success":true,"data":{"accepted":true}}\n' "$id" "$type"
      fi
      ;;
    remote-failure)
      printf '{"id":"%s","type":"response","command":"%s","success":false,"error":"fixture rejection"}\n' "$id" "$type"
      ;;
    malformed)
      printf 'not-json\n'
      hang
      ;;
    malformed-descendant)
      printf 'not-json\n'
      wait
      ;;
    invalid-utf8)
      printf '\377\n'
      hang
      ;;
    oversized)
      dd if=/dev/zero bs=1024 count=4 2>/dev/null | tr '\0' x
      printf '\n'
      hang
      ;;
    duplicate-response)
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      hang
      ;;
    mismatched-command)
      printf '{"id":"%s","type":"response","command":"not-%s","success":true}\n' "$id" "$type"
      hang
      ;;
    exit-pending)
      exit 23
      ;;
    event-backpressure)
      printf '{"type":"event_one"}\n'
      printf '{"type":"event_two"}\n'
      printf '{"type":"event_three"}\n'
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      ;;
    write-backpressure)
      sleep 2
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      ;;
    *)
      print -u2 -r -- "unknown fixture mode: $mode"
      exit 64
      ;;
  esac
done
