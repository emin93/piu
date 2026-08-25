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
  local candidate
  for candidate in prompt steer follow_up abort new_session get_state set_model get_available_models set_thinking_level set_auto_retry get_messages; do
    if [[ "$line" == *"\"type\":\"$candidate\""* ]]; then
      print -r -- "$candidate"
      return 0
    fi
  done
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
  local model_input='["text","image"]'
  if [[ "$mode" == "text-only" ]]; then
    model_input='["text"]'
  fi
  printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"%s","sessionFile":"%s","model":{"input":%s},"isStreaming":false,"thinkingLevel":"xhigh","messageCount":0,"pendingMessageCount":0}}\n' "$id" "$session_id" "$session_path" "$model_input"
}

IFS= read -r readiness
respond_state "$(request_id "$readiness")"
prompt_count=0

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
        if [[ "$mode" == "persisted-unresolved-tool" && -s "$session_path" ]]; then
          printf '{"id":"%s","type":"response","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":"Inspect the runtime","timestamp":100},{"role":"assistant","content":[{"type":"toolCall","id":"call-crashed","name":"shell","arguments":{"command":"npm test"}}],"usage":{"input":8,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":11,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":200}]}}\n' "$id"
        elif [[ "$mode" == "streaming" ]]; then
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
      (( prompt_count += 1 ))
      if [[ "$mode" == "crash-before-prompt" && ! -f "$record_dir/crashed-before-prompt" ]]; then
        touch "$record_dir/crashed-before-prompt"
        exit 47
      fi
      prompt_text="${line#*\"message\":\"}"
      prompt_text="${prompt_text%%\",\"streamingBehavior\"*}"
      print -r -- "$prompt_text" > "$prompt_record"
      if [[ "$mode" == "persisted-unresolved-tool" ]]; then
        print -r -- '{"role":"assistant","content":[{"type":"toolCall","id":"call-crashed","name":"shell","arguments":{"command":"npm test"}}],"stopReason":"toolUse"}' > "$session_path"
      fi
      if [[ "$mode" == "crash-after-prompt" || "$mode" == "persisted-unresolved-tool" ]]; then
        printf '%s\n' \
          '{"type":"agent_start"}' \
          '{"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"pending","timestamp":200}}' \
          '{"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":0,"id":"call-crashed","toolName":"shell"}}' \
          '{"type":"tool_execution_start","toolCallId":"call-crashed","toolName":"shell","args":{"command":"npm test"}}'
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
      elif [[ "$mode" == "steering-queue" ]]; then
        if (( prompt_count == 1 )); then
          printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
          printf '%s\n' \
            '{"type":"agent_start"}' \
            '{"type":"message_start","message":{"role":"user","content":"Inspect the runtime","timestamp":100}}' \
            '{"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"pending","timestamp":200}}' \
            '{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}' \
            '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Working on the current turn."}}'
        else
          printf '%s\n' \
            '{"type":"queue_update","steering":["Inspect the queued result"],"followUp":[]}'
          sleep 1
          printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
          printf '%s\n' \
            '{"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":1,"id":"call-queued","toolName":"shell"}}' \
            '{"type":"tool_execution_start","toolCallId":"call-queued","toolName":"shell","args":{"command":"npm test"}}'
        fi
      elif [[ "$mode" == "retry-attempt" ]]; then
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
        printf '%s\n' \
          '{"type":"agent_start"}' \
          '{"type":"message_start","message":{"role":"assistant","content":[],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"pending","timestamp":200}}' \
          '{"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":0,"id":"call-retry","toolName":"shell"}}' \
          '{"type":"tool_execution_start","toolCallId":"call-retry","toolName":"shell","args":{"command":"npm test"}}' \
          '{"type":"agent_end","messages":[{"role":"assistant","content":[],"stopReason":"error","errorMessage":"provider unavailable","timestamp":201}],"willRetry":true}' \
          '{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":1000,"errorMessage":"provider unavailable"}'
      elif [[ "$mode" == "needs-input" ]]; then
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
        printf '%s\n' \
          '{"type":"agent_start"}' \
          '{"type":"extension_ui_request","id":"extension-choice-1","method":"select","title":"Choose a strategy","options":["Keep both","Replace"]}'
      elif [[ "$mode" == "reject-prompt" || ("$mode" == "reject-steer" && $prompt_count -gt 1) ]]; then
        sleep 1
        printf '{"id":"%s","type":"response","command":"prompt","success":false,"error":"fixture rejection"}\n' "$id"
      else
        printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
      fi
      ;;
    extension_ui_response)
      print -r -- "$line" >> "$record_dir/extension-ui-responses"
      if [[ "$line" == *'"id":"extension-choice-1"'* ]]; then
        printf '%s\n' \
          '{"type":"extension_ui_request","id":"extension-editor-2","method":"editor","title":"Explain the choice","prefill":"Keep the public API"}'
      fi
      ;;
    set_auto_retry)
      printf '{"id":"%s","type":"response","command":"set_auto_retry","success":true}\n' "$id"
      ;;
    abort|steer)
      printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type"
      ;;
    *)
      printf '{"id":"%s","type":"response","command":"%s","success":false,"error":"unsupported fixture command"}\n' "$id" "$type"
      ;;
  esac
done
