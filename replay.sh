#!/bin/bash
# Replays the design-rationale edits of a past session through the built hook, so a
# verdict someone hit in a real session can be reproduced here. The shapes that matter
# come from real sessions and not from probes written by hand: an edit inserting a
# section before an existing one re-emits that heading, which is how the anchor strip
# and the finding parser were both caught mangling their input.
#
#   ./replay.sh <session-id|transcript.jsonl>      verdict per edit
#   ./replay.sh <session-id|transcript.jsonl> 3    the old/new strings of edit 3
#
# ALL=1 replays every Edit/Write, not only the ones touching a design-rationale.
set -uo pipefail

hook=${HOOK:-$(dirname "$0")/target/release/ticpu-claude-command-hook}
projects=${CLAUDE_CONFIG_DIR:-$HOME/.claude}/projects

if [[ $# -lt 1 ]]; then
	sed -n '2,11p' "$0" >&2
	exit 2
fi

if [[ ! -x $hook ]]; then
	printf 'replay: %s is not built (make release)\n' "$hook" >&2
	exit 1
fi

transcript=$1
if [[ ! -f $transcript ]]; then
	# A bare session id: the project directory it belongs to is not known here, and
	# the same id never appears under two of them.
	transcript=$(find "$projects" -name "$1.jsonl" -print -quit 2>/dev/null)
fi
if [[ ! -f ${transcript:-} ]]; then
	printf 'replay: no transcript for %s under %s\n' "$1" "$projects" >&2
	exit 1
fi

filter='test("design-rationale")'
[[ ${ALL:-0} == 1 ]] && filter='.'

edits=$(jq -c --argjson keep true '
	select(.message.content) | .message.content[]?
	| select(.type == "tool_use" and (.name == "Edit" or .name == "Write"))
	| select((.input.file_path // "") | '"$filter"')
	| {name, path: .input.file_path,
	   old: (.input.old_string // ""),
	   new: (.input.new_string // .input.content // "")}' "$transcript" | jq -s '.')

count=$(jq -r 'length' <<<"$edits")
if [[ $count == 0 ]]; then
	printf 'replay: no matching edits in %s\n' "$(basename "$transcript")" >&2
	exit 1
fi

# One edit asked for by index: print what the hook would be handed, not a verdict.
if [[ $# -ge 2 ]]; then
	jq -r --argjson i "$2" '.[$i] | "=== \(.name) \(.path)\n=== REPLACED\n\(.old)\n=== ADDED\n\(.new)"' <<<"$edits"
	exit
fi

printf '%s — %s edits\n' "$(basename "$transcript")" "$count"
for ((i = 0; i < count; i++)); do
	printf '%3d: ' "$i"
	jq -c --argjson i "$i" '.[$i]' <<<"$edits" |
		jq -c '{hook_event_name: "PreToolUse", tool_name: .name, cwd: ".",
		        tool_input: {file_path: .path, old_string: .old, new_string: .new,
		                     content: .new}}' |
		"$hook" |
		jq -r '.hookSpecificOutput
		       | .permissionDecision + " "
		       + ((.permissionDecisionReason // "")
		          | split("\n")
		          | map(select(startswith("Rule ") or startswith("Already")))
		          | join(" | "))'
done
