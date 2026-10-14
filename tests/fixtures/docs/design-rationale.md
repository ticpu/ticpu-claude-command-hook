# Design rationale

## A client returns the first failure

The client hands every failed request back to its caller instead of retrying it. Only the caller knows whether a second attempt is safe, and a retry hidden inside the client turns one clear failure into an ambiguous partial one.
