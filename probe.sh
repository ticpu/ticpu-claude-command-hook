#!/bin/bash
# Asks the hook what it would do with each command read from stdin, one per line.
# The asserted version of this lives in tests/verdicts.rs; this is for one-offs:
#
#   echo 'grep -rn foo src | head' | ./probe.sh
set -uo pipefail

hook=${HOOK:-$(dirname "$0")/target/release/ticpu-claude-command-hook}
# The hook resolves its own path, so match the canonical form to shorten it here.
gf=$(realpath "$(dirname "$hook")/gf")

if [[ ! -x $hook ]]; then
	printf 'probe: %s is not built (make release)\n' "$hook" >&2
	exit 1
fi

while IFS= read -r command; do
	[[ -z $command ]] && continue
	payload=$(jq -nc --arg c "$command" \
		'{hook_event_name:"PreToolUse",tool_name:"Bash",cwd:".",tool_input:{command:$c}}')
	out=$(printf '%s' "$payload" | "$hook")

	# No output is the hook's "allowed, untouched" answer.
	if [[ -z $out ]]; then
		printf 'PASS  %s\n' "$command"
		continue
	fi

	rewritten=$(jq -r '.hookSpecificOutput.updatedInput.command // empty' <<<"$out")
	if [[ -n $rewritten ]]; then
		printf 'FOLD  %s\n' "${rewritten//$gf/gf}"
		continue
	fi

	printf '%s  %s\n' "$(jq -r '.hookSpecificOutput.permissionDecision | ascii_upcase' <<<"$out")" "$command"
	jq -r '"      → " + .hookSpecificOutput.permissionDecisionReason' <<<"$out"
done
