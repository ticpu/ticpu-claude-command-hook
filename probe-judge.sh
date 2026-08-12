#!/bin/bash
# Asks the design-rationale check what it would do with each passage, as the added text
# of an Edit to the named document:
#
#   ./probe-judge.sh ~/GIT/eido/docs/design-rationale.md probes/design-rationale/*/*.md
#
# The document is judged too, so a passage's verdict only means something held against a
# fixed one — and never against this repo's own, which is about the judge. RUNS repeats
# each passage: the two judge calls race, so one verdict is a lead and not a result.
# ANCHOR is the text an Edit replaces, empty for an append.
set -uo pipefail

hook=${HOOK:-$(dirname "$0")/target/release/ticpu-claude-command-hook}
runs=${RUNS:-1}
anchor=${ANCHOR:-}

if [[ $# -lt 2 ]]; then
	printf 'usage: %s <design-rationale.md> <passage.md>...\n' "$0" >&2
	exit 2
fi
document=$1
shift

for passage in "$@"; do
	for ((i = 0; i < runs; i++)); do
		out=$(jq -nc --arg p "$document" --arg o "$anchor" --rawfile n "$passage" \
			'{hook_event_name:"PreToolUse",tool_name:"Edit",cwd:".",
			  tool_input:{file_path:$p,old_string:$o,new_string:$n}}' | "$hook")

		printf '%-48s %s\n' "$(basename "$(dirname "$passage")")/$(basename "$passage")" \
			"$(jq -r '.hookSpecificOutput.permissionDecision' <<<"$out")"
		# The findings are the last paragraph of the objection; the rest is standing text.
		jq -r '.hookSpecificOutput.permissionDecisionReason
		       | select(startswith("The design-rationale judge objects"))
		       | "     " + (split("\n\n") | last | gsub("\n"; "\n     "))' <<<"$out"
	done
done
