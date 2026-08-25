#!/bin/zsh

set -eu

value_for() {
  local wanted="$1"
  shift
  while (( $# > 0 )); do
    local flag="$1"
    local value="$2"
    shift 2
    if [[ "$flag" == "$wanted" ]]; then
      print -r -- "$value"
      return 0
    fi
  done
  return 1
}

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

agent_dir="$(value_for --agent-dir "$@")"
session_dir="$(value_for --session-dir "$@")"
session_path="$(value_for --session-path "$@" 2>/dev/null || true)"
if [[ -z "$session_path" ]]; then
  session_id="pi-${PWD:t}"
  session_path="$session_dir/$session_id.jsonl"
else
  session_id="${session_path:t:r}"
fi
record_dir="${agent_dir:h}/host-fixture"
mode="$(<"$record_dir/mode" 2>/dev/null || print -r -- quiet)"
prompt_record="$record_dir/prompt-$session_id"
mkdir -p "$record_dir" "$session_dir"
mkdir -p "$record_dir/live"
touch "$record_dir/live/$$"
cleanup() {
  rm -f "$record_dir/live/$$"
}
trap cleanup EXIT TERM HUP INT
touch "$session_path"
chmod 600 "$session_path"
print -r -- "$PWD" > "$record_dir/cwd"
print -r -- "$HOME" > "$record_dir/home"
print -r -- "$PATH" > "$record_dir/path"
print -r -- "$GIT_EXEC_PATH" > "$record_dir/git-exec-path"
print -r -- "$GIT_TEMPLATE_DIR" > "$record_dir/git-template-dir"
print -r -- "$0" > "$record_dir/launcher"
printf '%s\n' "$@" > "$record_dir/arguments"
print -r -- "launch" >> "$record_dir/launches"

respond_state() {
  local id="$1"
  printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"%s","sessionFile":"%s","isStreaming":false,"thinkingLevel":"xhigh","messageCount":0,"pendingMessageCount":0}}\n' "$id" "$session_id" "$session_path"
}

IFS= read -r readiness
respond_state "$(request_id "$readiness")"

while IFS= read -r line; do
  id="$(request_id "$line")"
  type="$(request_type "$line")"
  print -r -- "$line" >> "$record_dir/commands"
  print -r -- "$line" >> "$record_dir/commands-$session_id"
  case "$type" in
    get_state)
      respond_state "$id"
      ;;
    get_messages)
      if [[ -f "$prompt_record" ]]; then
        if [[ "$mode" == "streaming" ]]; then
          printf '{"id":"%s","type":"response","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":"Inspect the runtime","timestamp":100},{"role":"assistant","content":[{"type":"thinking","thinking":"I should inspect."},{"type":"text","text":"Done."},{"type":"toolCall","id":"call-1","name":"read","arguments":{"path":"README.md"}}],"usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":200},{"role":"toolResult","toolCallId":"call-1","toolName":"read","content":[{"type":"text","text":"fixture contents"}],"isError":false,"timestamp":201}]}}\n' "$id"
        else
          prompt_text="$(<"$prompt_record")"
          printf '{"id":"%s","type":"response","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":"%s","timestamp":100}]}}\n' "$id" "$prompt_text"
        fi
      else
        printf '{"id":"%s","type":"response","command":"get_messages","success":true,"data":{"messages":[]}}\n' "$id"
      fi
      ;;
    prompt)
      if [[ "$mode" == "crash-before-prompt" && ! -f "$record_dir/crashed-before-prompt" ]]; then
        touch "$record_dir/crashed-before-prompt"
        exit 47
      fi
      prompt_text="${line#*\"message\":\"}"
      prompt_text="${prompt_text%%\",\"streamingBehavior\"*}"
      print -r -- "$prompt_text" > "$prompt_record"
      if [[ "$mode" == "crash-after-prompt" ]]; then
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
        exit 48
      elif [[ "$mode" == "streaming" ]]; then
        printf '%s\n' \
          '{"type":"agent_start"}' \
          '{"type":"message_start","message":{"role":"user","content":"Inspect the runtime","timestamp":100}}' \
          '{"type":"message_end","message":{"role":"user","content":"Inspect the runtime","timestamp":100}}' \
          '{"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"pending","timestamp":200}}' \
          '{"type":"message_update","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}' \
          '{"type":"message_update","usage":{"input":1,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":3,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"I should inspect."}}' \
          '{"type":"message_update","usage":{"input":1,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":4,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"text_start","contentIndex":1}}' \
          '{"type":"message_update","usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Done."}}' \
          '{"type":"message_update","usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"toolcall_start","contentIndex":2,"id":"call-1","toolName":"read"}}' \
          '{"type":"message_update","usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"done","reason":"toolUse","message":{"role":"assistant","content":[],"usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":200}}}' \
          '{"type":"tool_execution_start","toolCallId":"call-1","toolName":"read","args":{"path":"README.md"}}' \
          '{"type":"tool_execution_update","toolCallId":"call-1","toolName":"read","args":{"path":"README.md"},"partialResult":{"content":[{"type":"text","text":"reading"}]}}' \
          '{"type":"tool_execution_end","toolCallId":"call-1","toolName":"read","result":{"content":[{"type":"text","text":"fixture contents"}]},"isError":false}' \
          '{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"I should inspect."},{"type":"text","text":"Done."},{"type":"toolCall","id":"call-1","name":"read","arguments":{"path":"README.md"}}],"usage":{"input":12,"output":7,"cacheRead":3,"cacheWrite":0,"totalTokens":22,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":200}}' \
          '{"type":"agent_end","messages":[],"willRetry":false}'
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
      elif [[ "$mode" == "repeated-events" ]]; then
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
        printf '%s\n' \
          '{"type":"message_start","message":{"role":"user","content":"Inspect the runtime","timestamp":100}}' \
          '{"type":"message_start","message":{"role":"user","content":"Repeat this","timestamp":101}}' \
          '{"type":"message_start","message":{"role":"user","content":"Repeat this","timestamp":102}}'
      else
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
      fi
      ;;
    abort|steer)
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      ;;
    *)
      printf '{"id":"%s","type":"response","command":"%s","success":false,"error":"unsupported fixture command"}\n' "$id" "$type"
      ;;
  esac
done
