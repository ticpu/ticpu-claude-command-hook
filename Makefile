CARGO ?= cargo

.PHONY: all check clippy test release probe install uninstall

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

install: release
	./target/release/ticpu-claude-command-hook install

# Takes the settings.json entries out; the built binary stays. It matches by binary
# name, so it works from a checkout that has moved since the install. Built first
# because the installed binary predates the verb.
uninstall: release
	./target/release/ticpu-claude-command-hook uninstall

# Ad-hoc verdicts for commands on stdin; the asserted table is tests/verdicts.rs.
probe: release
	@./probe.sh
