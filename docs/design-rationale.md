# Design rationale

## A session client shares its approval with nothing

One approval covers a whole Bash call, so a client opening a session outside the working tree
— a remote shell, a database — carries whatever is chained to it on the strength of its own
name. Such a call is denied rather than split: it has to be readable as the one thing it does.

The pipeline is asymmetric on purpose. A stage after the client only reads what it printed and
is approved on its own terms; a stage before it produces what the client then acts on, and
rides along. Operators inside the quoted remote command or SQL body belong to the far end and
must not count — the rule is about what this shell runs. Only that rule stops there: what a
command may hide from the transcript binds wherever it runs, so a search silencing stderr is
refused inside the body too, and an auto-allow reads the body as the command line it is.

## A location-dependent deny states the location

A deny that turns on where the command runs names the working directory and its repo root; the
rest say nothing about either. Neither reaches the caller in a tool result, so a bare refusal
buys a `pwd` round trip before it can even be acted on.

A `git add` pathspec resolving under the repo root but not the working directory is denied on
the same grounds — wrong root, and the deny can name the right spelling. Only then: a pathspec
that resolves nowhere is a deletion, which keeps the normal prompt.

A move into the repo the shell is already in is refused for the same reason. The prompt it would
otherwise take warns about hooks the move cannot reach, while the refusal can name the command
spelled from here and the bare move, which needs no approval. A move into a different repo keeps
that prompt: there the warning is true.

## One list of the segments an allow can carry

Every check that can allow a chain reads the same list of segments that grant nothing on their
own. Kept per check, those lists diverge along each check's own subject, and a chain mixing
subjects is then refused by both — each for the segment the other was written for.

## A literal named by a variable is refused, a computed one is not

Every permission rule, here and in the harness, matches command text, so an assignment in front
of the work makes the call match none of them and spends an approval on a string holding that
one value. A value the shell does not have to compute is therefore refused, the deny naming it:
the ground for refusing is that it can be written where it is used. One that must be computed
cannot be, and is the shape that keeps a credential out of the transcript, so it keeps its
normal prompt.

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

## A credential file is captured, never printed

A path whose name or directory says it holds a credential is refused wherever the shell would
print what it reads, and left to the normal prompt where the value is captured into a variable
or reaches another program through a substitution — a transcript outlives the session that
wrote it, so a credential printed into one is spent, while the shape that keeps it out of the
output still has to get the work done. What a program does with an argument it was handed is
outside this: the test is what the shell prints.

Matching on wording alone is exempt where the file is source or prose — a module about
credentials is not one — or where git already tracks the file, a committed value being spent
whatever this session does with it. Neither exemption extends to a location or an extension
that identifies a key. What is left is a name that reads like a credential and is not one, which no
rule here can settle: the refusal names a one-shot waiver, spent as it is read.

## A bug here must not stop the tools

Every failure path exits 0 with no decision, so a check that panics, mis-parses or cannot reach
what it needs leaves the tool call to its normal permission rules. This binary sits in front of
every command, read and write in every session, so a refusal it emits by accident is not one
bad answer — it is the whole toolset down until someone edits settings.json. A wrong allow costs
one prompt that should have been shown.

The credential check is the exception, having no allow to withhold: a command it cannot split is
judged whole rather than waved through.
