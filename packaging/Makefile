NAME := ticpu-claude-command-hook
BINARY := ticpu-claude-command-hook
CARGO_VERSION := $(shell grep '^version' ../Cargo.toml | head -1 | cut -d'"' -f2)
VERSION := v$(CARGO_VERSION)
GIT_DIRTY := $(shell git diff-index --quiet HEAD -- .. 2>/dev/null || echo dirty)
GIT_TAG := $(shell git describe --exact-match --tags 2>/dev/null | grep -E '^v')
GIT_VERSION := $(shell git log --oneline | wc -l)-$(shell git rev-parse --short HEAD)
BASE_VERSION := $(if $(GIT_DIRTY),$(VERSION)+$(GIT_VERSION),$(if $(GIT_TAG),$(VERSION),$(VERSION)+$(GIT_VERSION)))
DEB_VERSION := $(patsubst v%,%,$(BASE_VERSION))
DEB_AMD64 := $(NAME)_$(DEB_VERSION)_amd64.deb
DEB_ARM64 := $(NAME)_$(DEB_VERSION)_arm64.deb
BINARIES := dist/$(BINARY).amd64 dist/gf.amd64 dist/$(BINARY).arm64 dist/gf.arm64
DEPENDS := dist/deb-depends
PKG := package.tmp

.PHONY: all clean binaries deb deb-amd64 deb-arm64

all: binaries

binaries: $(BINARIES)

deb: deb-amd64 deb-arm64

deb-amd64: $(DEB_AMD64)

deb-arm64: $(DEB_ARM64)

# A version bump otherwise leaves the previous version's .deb here for a deploy
# script to pick up alongside the current one.
define purge-stale-debs
	@for f in $(NAME)_*.deb; do \
		[ -e "$$f" ] || continue; \
		case "$$f" in \
			$(DEB_AMD64)|$(DEB_ARM64)) ;; \
			*) rm -f "$$f"; echo "removed stale $$f"; ;; \
		esac; \
	done
endef

# gf stays out of PATH: two letters is a name to leave to whoever claims it in a
# distribution first. The hook looks for it here when it is not its own sibling.
define build-deb
	$(call purge-stale-debs)
	rm -rf "$(PKG)"
	install -D -m 755 -T "dist/$(BINARY).$(1)" "$(PKG)/usr/bin/$(BINARY)"
	install -D -m 755 -T "dist/gf.$(1)" "$(PKG)/usr/libexec/$(NAME)/gf"
	install -D -m 644 -T ../docs/allowed-commands.md "$(PKG)/usr/share/doc/$(NAME)/allowed-commands.md"
	install -D -m 644 -T ../LICENSE "$(PKG)/usr/share/doc/$(NAME)/copyright"
	install -D -m 644 -T control "$(PKG)/DEBIAN/control"
	@test -s $(DEPENDS) || { echo "$(DEPENDS) is empty: Depends would ship blank" >&2; exit 1; }
	sed -i -e "s/^Version:.*/Version: $(DEB_VERSION)/" \
		-e "s/^Architecture:.*/Architecture: $(1)/" \
		-e "s/^Depends:.*/Depends: $$(cat $(DEPENDS))/" \
		-e "s/^Installed-Size:.*/Installed-Size: $$(du -ks --exclude=DEBIAN $(PKG) | cut -f1)/" \
		"$(PKG)/DEBIAN/control"
	cd "$(PKG)" && find . -path ./DEBIAN -prune -o -type f -print0 \
		| xargs -0 md5sum | sed 's| \./| |' > DEBIAN/md5sums
	dpkg-deb --build --root-owner-group "$(PKG)" "$@"
	rm -rf "$(PKG)"
endef

$(DEB_AMD64): dist/$(BINARY).amd64 dist/gf.amd64 $(DEPENDS) control ../LICENSE
	$(call build-deb,amd64)

$(DEB_ARM64): dist/$(BINARY).arm64 dist/gf.arm64 $(DEPENDS) control ../LICENSE
	$(call build-deb,arm64)

# One container build cross-compiles every target, so they share a single rule
$(BINARIES) $(DEPENDS) &: build.sh Containerfile ../Cargo.toml $(wildcard ../Cargo.lock) $(wildcard ../src/*.rs ../src/*/*.rs)
	./build.sh

clean:
	rm -rf "$(PKG)" dist *.deb
