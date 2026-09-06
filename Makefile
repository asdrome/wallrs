PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user
DOCDIR ?= $(PREFIX)/share/doc/wallrs
LICDIR ?= $(PREFIX)/share/licenses/wallrs

CARGO ?= cargo
TARGET ?= release

.PHONY: all build test clean install uninstall

all: build

build:
	$(CARGO) build --release

test:
	$(CARGO) test --workspace

clean:
	$(CARGO) clean

install: build
	install -d $(DESTDIR)$(BINDIR)
	install -m 755 target/release/wallrsd $(DESTDIR)$(BINDIR)/wallrsd
	install -m 755 target/release/wallctl $(DESTDIR)$(BINDIR)/wallctl
	install -d $(DESTDIR)$(SYSTEMD_USER_DIR)
	install -m 644 extra/systemd/wallrsd.service $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrsd.service
	install -d $(DESTDIR)$(DOCDIR)
	install -m 644 README.md $(DESTDIR)$(DOCDIR)/README.md
	install -d $(DESTDIR)$(LICDIR)
	install -m 644 LICENSE-MIT $(DESTDIR)$(LICDIR)/LICENSE-MIT
	install -m 644 LICENSE-APACHE $(DESTDIR)$(LICDIR)/LICENSE-APACHE

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/wallrsd
	rm -f $(DESTDIR)$(BINDIR)/wallctl
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/wallrsd.service
	rm -rf $(DESTDIR)$(DOCDIR)
	rm -rf $(DESTDIR)$(LICDIR)

