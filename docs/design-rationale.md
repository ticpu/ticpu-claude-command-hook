# Design rationale

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
