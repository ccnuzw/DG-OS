#!/bin/sh
# SYNTHETIC PROTOCOL ONLY. No CLI, model, HTTP request or approved command executes.
if [ "${1-}" = --version ]; then printf 'codex-cli 0.167.0\n'; exit 0; fi
printf '%s\n' "$$" > phase6-child.pid
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> phase6-protocol.jsonl
  request=${line#*\"id\":}; request=${request%%,*}
  case "$line" in
    *'"method":"initialize"'*) printf '{"id":%s,"result":{}}\n' "$request";;
    *'"method":"config/read"'*) printf '{"id":%s,"result":{"config":{"model_provider":"dgos_project","model_providers":{"dgos_project":{"base_url":"http://127.0.0.1:9/v1","wire_api":"responses","env_key":"DGOS_PROVIDER_KEY","requires_openai_auth":false}}}}}\n' "$request";;
    *'"method":"model/list"'*) printf '{"id":%s,"result":{"data":[{"model":"fixture-model","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"medium"},{"reasoningEffort":"high"}]}],"nextCursor":null}}\n' "$request";;
    *'"method":"collaborationMode/list"'*) printf '{"id":%s,"result":{"data":[{"name":"Default","mode":"default","model":"fixture-model"},{"name":"Plan","mode":"plan","model":"fixture-model"}]}}\n' "$request";;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*) printf '{"id":%s,"result":{"cwd":"%s","model":"fixture-model","modelProvider":"dgos_project","approvalPolicy":"on-request","sandbox":{"type":"workspaceWrite"},"reasoningEffort":"medium","thread":{"id":"codex-lifecycle-SYNTHETIC-thread","turns":[]}}}\n' "$request" "$PWD";;
    *'"method":"turn/start"'*)
      turn=$((turn+1))
      printf '{"id":%s,"result":{"turn":{"id":"lifecycle-turn-%s"}}}\n' "$request" "$turn"
      printf '{"method":"turn/started","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turn":{"id":"lifecycle-turn-%s"}}}\n' "$turn"
      if [ "$turn" -eq 1 ]; then
        printf '%s\n' '{"id":"lifecycle-SYNTHETIC-approval","method":"item/commandExecution/requestApproval","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turnId":"lifecycle-turn-1","itemId":"synthetic-command","command":"SYNTHETIC PROTOCOL ONLY: deny this request to preserve the selected read-only sandbox; no command executes","availableDecisions":["accept","decline"]}}'
      else
        printf '{"method":"item/completed","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turnId":"lifecycle-turn-%s","item":{"id":"synthetic-result-%s","type":"agentMessage","text":"SYNTHETIC PROTOCOL ONLY: explicit second send completed; no model invocation."}}}\n' "$turn" "$turn"
        printf '{"method":"turn/completed","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turn":{"id":"lifecycle-turn-%s","status":"completed"}}}\n' "$turn"
      fi;;
    *'"decision":"decline"'*)
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turnId":"lifecycle-turn-1","item":{"id":"synthetic-denial-receipt","type":"agentMessage","text":"SYNTHETIC PROTOCOL ONLY: denial received; original turn remains running for steering or interrupt. No command executed."}}}'
      ;;
    *'"method":"turn/steer"'*) printf '{"id":%s,"result":{"turnId":"lifecycle-turn-%s"}}\n' "$request" "$turn";;
    *'"method":"turn/interrupt"'*)
      printf '{"id":%s,"result":{}}\n' "$request"
      printf '{"method":"turn/completed","params":{"threadId":"codex-lifecycle-SYNTHETIC-thread","turn":{"id":"lifecycle-turn-%s","status":"interrupted"}}}\n' "$turn";;
    *'"id":'*'"method":'*) printf '{"id":%s,"error":{"code":-32601,"message":"Synthetic fixture does not implement this method"}}\n' "$request";;
  esac
done
