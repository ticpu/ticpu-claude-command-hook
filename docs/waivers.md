# Waivers

Four checks refuse rather than prompt, and each names a one-shot waiver that overrules
it. One further marker, at the end, is a standing switch rather than a waiver. A waiver is a file you approve into existence; the binary deletes it as it reads it,
before the decision it overrules, so a failed delete cannot leave a standing pass behind.

They all live in one directory, made once per boot:

```sh
mkdir -p "$XDG_RUNTIME_DIR/claude-hooks"
```

Creating one is forced to a permission prompt whatever your allowlist says — an
allowlisted `touch` must not hand out a pass unseen — and the prompt describes what that
particular waiver grants. Only *creating* it is prompted: a `test -e`, an `rm` or a grep
of this repo's source names the file without granting anything.

The model is expected to run these itself after arguing why the objection is wrong, and
you decide at the prompt. Running one by hand is the same thing without the argument.

## `design-rationale-judge-bypass`

Refused by: the judged reviews of a `design-rationale.md` edit — a rules finding, a
duplicate section, or a question the reviewer could not resolve.

```sh
touch "$XDG_RUNTIME_DIR/claude-hooks/design-rationale-judge-bypass"
```

Spent by the next edit that reaches the judge, before it is judged. The mechanical rules
(heading form, section length, a CLAUDE.md reference) are outside it — there is nothing
to overrule in a count.

## `design-rationale-shell-write`

Refused by: a shell command that rewrites a `design-rationale.md`. The reviews hang off
`Edit` and `Write`, so a redirect, an `mv`, a `sed -i` or an interpreter heredoc writing
that file reaches no reviewer.

```sh
touch "$XDG_RUNTIME_DIR/claude-hooks/design-rationale-shell-write"
```

Spent by the next command this would refuse. Reading the document is never refused, so
the waiver is for a revert, a rename, or a file generated whole.

## `script-edit-waiver`

Refused by: an interpreter heredoc that reads a file whole, substitutes into it, and
writes it back — a substitution matching nothing rewrites nothing and says nothing.

```sh
touch "$XDG_RUNTIME_DIR/claude-hooks/script-edit-waiver"
```

Spent by the next command this would refuse. For generated output, a file too large to
read, or one substitution across many files.

## `transcript-read-waiver`

Refused by: a command or a `Read`/`Grep` naming a path that reads as a credential, where
the output would land in the session transcript.

```sh
touch "$XDG_RUNTIME_DIR/claude-hooks/transcript-read-waiver"
```

Spent by the next refusal. Approve only if that path holds nothing secret on this box —
the contents go into the transcript either way.

## `design-rationale-gate-off` — a switch, not a waiver

Turns the whole `Edit`/`Write` gate on `design-rationale.md` off: the countable rules and
both judged reviews. For rewriting the file section by section, where a round trip per
draft is the cost and not the point.

```sh
touch "$XDG_RUNTIME_DIR/claude-hooks/design-rationale-gate-off"
rm "$XDG_RUNTIME_DIR/claude-hooks/design-rationale-gate-off"
```

Not spent on use — it stands until you remove it, or until logout empties the directory.
Every edit made under it still prompts, saying the gate is off and how to restore it, and
the model is told afterwards that nothing read what it wrote. Creating it is prompted;
removing it is not.

A shell write of the document is still refused while this is on: `Edit` remains the route,
it is just unreviewed.

## Other files in that directory

Neither is a waiver, and neither is prompted:

- `glab-skill-<session-id>` — written by the binary after the first `glab` call of a
  session carries the guidance. Delete it to be handed the guidance again.
- `design-rationale-judge-context` — answers to a reviewer's questions, written by the
  model when the judge says what it was missing. Read into the next review of that file
  and deleted as it is read. Writing it yourself works the same way.
