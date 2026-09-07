PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user
DOCDIR ?= $(PREFIX)/share/doc/wallrs
LICDIR ?= $(PREFIX)/share/licenses/wallrs

CARGO ?= cargo
TARGET ?= release

.PHONY: all build test clean install uninstall lint check deb rpm bump-patch bump-minor bump-major changelog

all: build

build:
	$(CARGO) build --release

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: lint test

clean:
	$(CARGO) clean

deb:
	cargo deb -p wallrs-daemon

rpm:
	@mkdir -p $(HOME)/rpmbuild/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
	@VERSION=$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1); \
	git archive --format=tar.gz --prefix="wallrs-$$VERSION/" -o $(HOME)/rpmbuild/SOURCES/"wallrs-$$VERSION.tar.gz" HEAD; \
	cp extra/rpm/wallrs.spec $(HOME)/rpmbuild/SPECS/; \
	rpmbuild -ba $(HOME)/rpmbuild/SPECS/wallrs.spec; \
	echo "RPM packages generated in $(HOME)/rpmbuild/RPMS/"

bump-patch:
	@python3 scripts/bump-version.py patch

bump-minor:
	@python3 scripts/bump-version.py minor

bump-major:
	@python3 scripts/bump-version.py major

changelog:
	@if command -v git-cliff >/dev/null 2>&1; then \
		git-cliff -o CHANGELOG.md; \
		echo "CHANGELOG.md updated via git-cliff"; \
	else \
		echo "# Changelog" > CHANGELOG.md; \
		echo "" >> CHANGELOG.md; \
		git log --pretty=format:"* %s (%h)" >> CHANGELOG.md; \
		echo "CHANGELOG.md generated from git log"; \
	fi

SHAREDIR ?= $(PREFIX)/share/wallrs

install: build
	install -d $(DESTDIR)$(BINDIR)
	install -m 755 target/release/wallrsd $(DESTDIR)$(BINDIR)/wallrsd
	install -m 755 target/release/wallctl $(DESTDIR)$(BINDIR)/wallctl
	install -d $(DESTDIR)$(SYSTEMD_USER_DIR)
	install -m 644 extra/systemd/wallrsd.service $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrsd.service
	install -m 644 extra/systemd/wallrs-theme-sync.path $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrs-theme-sync.path
	install -m 644 extra/systemd/wallrs-theme-sync.service $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrs-theme-sync.service
	install -d $(DESTDIR)$(SHAREDIR)/contrib
	install -m 755 contrib/*.sh $(DESTDIR)$(SHAREDIR)/contrib/
	install -d $(DESTDIR)$(DOCDIR)
	install -m 644 README.md $(DESTDIR)$(DOCDIR)/README.md
	install -d $(DESTDIR)$(LICDIR)
	install -m 644 LICENSE-MIT $(DESTDIR)$(LICDIR)/LICENSE-MIT
	install -m 644 LICENSE-APACHE $(DESTDIR)$(LICDIR)/LICENSE-APACHE

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/wallrsd
	rm -f $(DESTDIR)$(BINDIR)/wallctl
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrsd.service
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrs-theme-sync.path
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrs-theme-sync.service
	rm -rf $(DESTDIR)$(SHAREDIR)
	rm -rf $(DESTDIR)$(DOCDIR)
	rm -rf $(DESTDIR)$(LICDIR)

