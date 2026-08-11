# Design rationale

## A session client shares its approval with nothing

One approval covers a whole Bash call, so a client opening a session outside the working tree
— a remote shell, a database — carries whatever is chained to it on the strength of its own
name. Such a call is denied rather than split: it has to be readable as the one thing it does.

The pipeline is asymmetric on purpose. A stage after the client only reads what it printed and
is approved on its own terms; a stage before it produces what the client then acts on, and
rides along. Operators inside the quoted remote command or SQL body belong to the far end and
must not count — the rule is about what this shell runs.

## A location-dependent deny states the location

A deny that turns on where the command runs names the working directory and its repo root; the
rest say nothing about either. Neither reaches the caller in a tool result, so a bare refusal
buys a `pwd` round trip before it can even be acted on.

A `git add` pathspec resolving under the repo root but not the working directory is denied on
the same grounds — wrong root, and the deny can name the right spelling. Only then: a pathspec
that resolves nowhere is a deletion, which keeps the normal prompt.

## Path shape gates the fold before the filesystem is consulted

`gf` decides that a line's leading text is a path by asking the filesystem whether it exists.
Existence alone is too weak: source lines routinely open with text that happens to name a file
in the tree, and accepting one costs the reader that text — the fold is there to shorten long
paths, so silently eating line content is the one failure it must not have. A syntactic shape
test now runs first, and only a path-shaped candidate reaches the stat. Ordering it that way
also keeps the hit/miss caches meaningful and spares content lines a stat apiece.

Requiring a slash was the tempting rule and is wrong: a search naming files in the working
directory prints matches with no directory component at all, and those must keep folding. So a
slash-free candidate stays eligible, judged instead on the punctuation that separates a
filename from code. The test is deliberately conservative — a rejected candidate only forgoes
folding and prints in full, while a wrong acceptance corrupts output.

The same gate guards the fast path that folds a line repeating the previous path. It trusted
the previous match without rechecking, which made a content line beginning with that path plus
a separator lose the text.

## A prefixed search line is refused, not parsed around

`gf` anchors a path at the start of a line, so a search that filters another search's output and
adds a position or filename prefix of its own makes every line unfoldable. Teaching gf to skip
such a prefix is the wrong repair: those positions count the piped stream, so they name no line
in any file, and folding around them would dress up output that is already wrong. Deny instead.

## Countable rules are checked in code, judged rules in the model

Every rule a program can decide stays in the program; the model is asked only what it can point
at and quote. A small model flags textbook knowledge and narration reliably, and misses what it
has to count just as reliably — a forbidden heading form and a section at twice the length bound
survive the same review that catches the prose faults beside them.

## The judge is trusted only where it was measured

Its rule list is closed and deliberately shorter than the authoring rules: given every rule, the
same model on the same edits rule-shops until something matches, and denies nearly everything. A
rule joins that list only after it earns its place against labelled sections, and only if it
holds up asked on its own: the list is read as a set, so a rule matching almost any passage stays
quiet while a better match exists, and one that needs its neighbours to outbid it is deciding
nothing. A finding is taken only where the quoted passage can carry it. Qualifying the rule's own text does not hold
the model to it — under pressure to answer it reaches for whichever rule is nearest — so where a
rule has a precondition a program can see, the finding is checked against its quote and dropped
when it fails: a line naming no rule, a quote absent from the added text, or a quote carrying
nothing the rule it names needs — no reference to the past under the rule against narrating a
previous state, no value of any kind under the one against enumerating them. A line naming no
rule is what a model does when it reasons in the open: it answers with the verdict, then argues
itself to the other one, and every line of the argument quotes the passage it is weighing.

A judge that did not run must never look like one that passed. The context length is stated on
every request rather than left to server configuration, where a short default truncates the
prompt and the model answers confidently from the fragment that survived; an unreachable or
unparseable reply allows the write and says on screen that no judgement was made.

## A judged objection is overruled in one prompt

An objection stops the edit, since an Edit's permission prompt renders the diff and nothing
else — carried there, it is read by nobody before deciding. Stopping puts the finding in front
of the model instead, which can revise or argue it. The overrule is the bypass marker's own
prompt: the model states the finding and creates the marker, and approving that creation is the
decision. Putting the question to the reader first and creating it afterwards asks the same
thing twice, and the model is small enough to read domain behaviour a project depends on as
textbook knowledge — so the second ask is the common case, not the rare one. A countable rule
refuses outright, having nothing to weigh.

Approving is the review, and the write says so afterwards, because otherwise the writer stops
and asks for a second one nobody owes. It is said after rather than on the prompt: a prompt's
reason is addressed to whoever answers the prompt, so a refusal reaches the writer only because
refusing is itself the answer, and the same words beside a question the reader is answering
reach nobody. An edit too small to judge is prompted too, rather than landing unseen.

A file that does not exist yet is judged with nothing around it, and the rules asking what a
reader holding this repo would already know have nothing to check against; the objection then
asks for the frame the file is missing rather than for a narrower passage.

What is measured is what the edit introduces, with the whole lines it copies out of the document
stripped from both ends — never part of one, since an insert lands before a heading and shares
its marker, and a heading handed over with the marker gone is judged as the flat assertion it
then reads as. An edit that removes a paragraph has to re-emit the section around it,
and judged whole it draws findings against prose already in the file, which no revision can
answer. A re-wrap introduces nothing at all. A section named as already owning the decision may
not be one the edit is rewriting or deleting, for the same reason.

## A bug here must not stop the tools

Every failure path exits 0 with no decision, so a check that panics, mis-parses or cannot reach
what it needs leaves the tool call to its normal permission rules. This binary sits in front of
every Bash, Edit and Write call in every session, so a refusal it emits by accident is not one
bad answer — it is the whole toolset down until someone edits settings.json. A wrong allow costs
one prompt that should have been shown.
