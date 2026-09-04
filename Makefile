CARGO ?= cargo

.PHONY: all check clippy test release probe install uninstall archpkg archpkg-install

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

# The .deb is cross-compiled in a container for two architectures; this one builds
# the checkout natively for the machine it runs on. pacman -U prompts for the
# install, so it is not the --noconfirm shape.
archpkg:
	cd packaging && makepkg -f

archpkg-install:
	cd packaging && makepkg -fsi

# Ad-hoc verdicts for commands on stdin; the asserted table is tests/verdicts.rs.
probe: release
	@./probe.sh
