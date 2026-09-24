#!/usr/bin/env bash
#
# Gates subagent spawns for this repository.
#
#   * Every Agent/Task spawn is escalated to a permission prompt, carrying a
#     reminder to check /usage first. Nothing spawns without an explicit yes.
#   * A spawn is denied outright once MAX_SUBAGENTS are already running.
#
# One script serves three hook events, branching on hook_event_name:
# PreToolUse decides, SubagentStart/SubagentStop maintain the counter, and
# SessionStart clears any counter left behind by a crashed session.
#
# Background: on 2026-08-20 three research subagents ran in parallel, hit the
# session usage limit together, and all three died before writing their
# findings down. The work was lost and their tickets were left claimed but
# empty.

set -uo pipefail

MAX_SUBAGENTS=2

input=$(cat)
event=$(jq -r '.hook_event_name // ""' <<<"$input")
session=$(jq -r '.session_id // "unknown"' <<<"$input")

counter="${TMPDIR:-/tmp}/claude-subagents-${session}.count"

read_count() {
  local n
  n=$(cat "$counter" 2>/dev/null) || n=0
  [[ "$n" =~ ^[0-9]+$ ]] || n=0
  printf '%s' "$n"
}

write_count() {
  printf '%s' "$1" >"$counter" 2>/dev/null || true
}

case "$event" in
  SubagentStart)
    write_count "$(( $(read_count) + 1 ))"
    ;;

  SubagentStop)
    n=$(read_count)
    (( n > 0 )) && n=$(( n - 1 ))
    write_count "$n"
    ;;

  SessionStart)
    write_count 0
    ;;

  PreToolUse)
    running=$(read_count)
    if (( running >= MAX_SUBAGENTS )); then
      jq -n --argjson running "$running" --argjson max "$MAX_SUBAGENTS" '{
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "deny",
          permissionDecisionReason: (
            "Blocked: \($running) subagents are already running and the limit for this repository is \($max). "
            + "Wait for one to finish, or resume a stopped agent with SendMessage instead of spawning a new one."
          )
        }
      }'
    else
      jq -n --argjson running "$running" --argjson max "$MAX_SUBAGENTS" '{
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "ask",
          permissionDecisionReason: (
            "Check /usage before approving this subagent. Do not start one when less than 25% of the session remains — "
            + "an agent killed mid-task loses its findings. Currently running: \($running) of \($max)."
          )
        }
      }'
    fi
    ;;
esac

exit 0
