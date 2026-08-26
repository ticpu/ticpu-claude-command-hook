CARGO ?= cargo

.PHONY: all check clippy test release probe

all: release

# `make -j check` runs both concurrently; cargo serializes them on its own build
# lock, so the win is only on the non-overlapping work.
check: clippy test

clippy:
	$(CARGO) clippy --all-targets -- -D warnings

test:
	$(CARGO) test

# The generated list is regenerated here, not by hand: a CLAUDE.md importing it
# must not lag the binary that decides the allows.
release:
	$(CARGO) build --release
	./target/release/ticpu-claude-command-hook rules > docs/allowed-commands.md

# Ad-hoc verdicts for commands on stdin; the asserted table is tests/verdicts.rs.
probe: release
	@./probe.sh
