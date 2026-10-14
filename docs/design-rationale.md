# Design rationale

## A bug here must not stop the tools

Every failure path exits 0 with no decision, so a check that panics, mis-parses or cannot reach
what it needs leaves the tool call to its normal permission rules. This binary sits in front of
every command, read and write in every session, so a refusal it emits by accident is not one
bad answer — it is the whole toolset down until someone edits settings.json. A wrong allow costs
one prompt that should have been shown.

The credential check is the exception, having no allow to withhold: a command it cannot split is
judged whole rather than waved through.
