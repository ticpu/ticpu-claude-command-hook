# Design rationale

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
