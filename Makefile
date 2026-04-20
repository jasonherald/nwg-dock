# Makefile for nwg-dock — binary-repo subset per epic §3.6.
#
# Default install target is /usr/local (LSB convention for locally-built
# software). Contributors iterating from a clone should use the no-sudo
# override:
#   make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin
# Go-predecessor parity is an opt-in:
#   sudo make install PREFIX=/usr
#
# The binary renames from nwg-dock-hyprland → nwg-dock (epic §3.1), but
# DATA + CONFIG paths still reference `nwg-dock-hyprland/` to preserve
# existing users' `~/.config/nwg-dock-hyprland/style.css` customizations
# across upgrade. Future release may unify paths with a migration step.

CARGO   ?= cargo
PREFIX  ?= /usr/local
BINDIR  ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
DESTDIR ?=

# Go-predecessor data-dir name — see header comment.
DATA_APP_NAME := nwg-dock-hyprland
# New binary name + legacy symlink target for backwards-compat.
BIN_NAME        := nwg-dock
LEGACY_BIN_NAME := nwg-dock-hyprland

SONAR_SCANNER ?= /opt/sonar-scanner/bin/sonar-scanner
SONAR_HOST_URL ?= https://sonar.aaru.network
SONAR_TRUSTSTORE ?= /tmp/sonar-truststore.jks
SONAR_TRUSTSTORE_PASSWORD ?= changeit

.PHONY: all build build-release test test-integration lint check-tools \
        lint-fmt lint-clippy lint-test lint-deny lint-audit \
        install install-bin install-data uninstall \
        setup-hyprland setup-sway upgrade \
        sonar clean help

all: build

define HELP_TEXT
Targets:
  make build           Debug build
  make build-release   Release build (used by install + upgrade)
  make test            cargo test + cargo clippy --all-targets
  make test-integration tests/integration/test_runner.sh (requires sway + foot)
  make lint            Full local check: fmt + clippy + test + deny + audit
  make install         Build release + install binary + install data
  make install-bin     Install binary to $(DESTDIR)$(BINDIR) + nwg-dock-hyprland symlink alias
  make install-data    Install data assets to $(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)/
  make uninstall       Remove installed binary, symlink, and data
  make setup-hyprland  Print Hyprland autostart snippet
  make setup-sway      Print Sway autostart snippet
  make upgrade         Capture running args, stop, rebuild, install, restart
  make sonar           Run SonarQube scan (requires sonar-scanner + .env)
  make clean           cargo clean

Install-path invocations:
  sudo make install                                              # default /usr/local
  make install PREFIX=$$HOME/.local BINDIR=$$HOME/.cargo/bin     # no-sudo dev
  sudo make install PREFIX=/usr                                  # distro-parity
endef
export HELP_TEXT

help:
	@echo "$$HELP_TEXT"

build:
	$(CARGO) build

build-release:
	$(CARGO) build --release

test:
	$(CARGO) test
	$(CARGO) clippy --all-targets

test-integration: build-release
	@echo "Running headless Sway integration tests..."
	@bash tests/integration/test_runner.sh

check-tools:
	@if ! command -v cargo-deny >/dev/null 2>&1; then \
		echo "Installing cargo-deny..."; \
		$(CARGO) install cargo-deny; \
	fi
	@if ! command -v cargo-audit >/dev/null 2>&1; then \
		echo "Installing cargo-audit..."; \
		$(CARGO) install cargo-audit; \
	fi

# Individual lint subtargets — each runnable on its own.
lint-fmt:
	@echo "── Format ──"
	$(CARGO) fmt --all --check

lint-clippy:
	@echo "── Clippy ──"
	$(CARGO) clippy --all-targets -- -D warnings

# Plain test (no clippy) so `make lint` runs clippy exactly once via lint-clippy.
lint-test:
	@echo "── Tests ──"
	$(CARGO) test

lint-deny:
	@echo "── Cargo Deny (licenses, advisories, bans, sources) ──"
	$(CARGO) deny check

lint-audit:
	@echo "── Cargo Audit (dependency CVEs) ──"
	$(CARGO) audit

lint: check-tools lint-fmt lint-clippy lint-test lint-deny lint-audit
	@echo ""
	@echo "All local checks passed ✓"

# ─────────────────────────────────────────────────────────────────────
# Install / uninstall
# ─────────────────────────────────────────────────────────────────────

install: build-release install-bin install-data

install-bin:
	@echo "Installing binary to $(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	install -D -m 755 target/release/$(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	@echo "Creating legacy-name symlink: $(LEGACY_BIN_NAME) → $(BIN_NAME)"
	ln -sf $(BIN_NAME) "$(DESTDIR)$(BINDIR)/$(LEGACY_BIN_NAME)"

install-data:
	@echo "Installing data assets to $(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)/"
	install -d "$(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)/images"
	install -m 644 data/$(DATA_APP_NAME)/style.css "$(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)/"
	install -m 644 data/$(DATA_APP_NAME)/images/*.svg "$(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)/images/"

uninstall:
	@echo "Removing binary + symlink + data"
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	rm -f "$(DESTDIR)$(BINDIR)/$(LEGACY_BIN_NAME)"
	rm -rf "$(DESTDIR)$(DATADIR)/$(DATA_APP_NAME)"
	@echo "Uninstalled."

# ─────────────────────────────────────────────────────────────────────
# Compositor setup helpers
# ─────────────────────────────────────────────────────────────────────

setup-hyprland:
	@echo "# Add to ~/.config/hypr/autostart.conf:"
	@echo "exec-once = uwsm-app -- $(BIN_NAME) -d -i 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation -c \"nwg-drawer --pb-auto\""

setup-sway:
	@echo "# Add to ~/.config/sway/config:"
	@echo "exec_always $(BIN_NAME) -d -i 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation -c \"nwg-drawer --pb-auto\""

# ─────────────────────────────────────────────────────────────────────
# Upgrade — capture running args, stop, rebuild+install, restart
# ─────────────────────────────────────────────────────────────────────
#
# `pidof -c` matches against the binary's comm field, which stays the
# same across the symlink alias — so we pass both names to cover users
# still running via the `nwg-dock-hyprland` symlink.
upgrade: build-release
	@RUNNING_PIDS="$$(pidof -c $(BIN_NAME) $(LEGACY_BIN_NAME) 2>/dev/null || true)"; \
	if [ -n "$$RUNNING_PIDS" ]; then \
		ARGS_FILE="$$(mktemp)" || exit 1; \
		trap 'rm -f "$$ARGS_FILE"' EXIT; \
		for pid in $$RUNNING_PIDS; do \
			target/release/$(BIN_NAME) --dump-args "$$pid" >> "$$ARGS_FILE" || exit 1; \
		done; \
		echo "Stopping running instance(s): $$RUNNING_PIDS"; \
		kill $$RUNNING_PIDS 2>/dev/null || true; \
		sleep 1; \
		$(MAKE) install-bin install-data || exit 1; \
		if [ -s "$$ARGS_FILE" ]; then \
			while IFS= read -r args; do \
				echo "Restarting with captured args: $$args"; \
				setsid sh -c "$$args" </dev/null >/dev/null 2>&1 & \
			done < "$$ARGS_FILE"; \
		fi; \
	else \
		echo "No running instance — installing without restart"; \
		$(MAKE) install-bin install-data || exit 1; \
	fi
	@echo "Upgrade complete."

# ─────────────────────────────────────────────────────────────────────
# SonarQube scan — .env is PARSED (never sourced) to avoid shell injection.
# ─────────────────────────────────────────────────────────────────────

sonar:
	@echo "Running SonarQube scan..."
	@test -f ./.env || { echo "ERROR: .env not found in repo root"; exit 1; }
	@command -v "$(SONAR_SCANNER)" >/dev/null 2>&1 || [ -x "$(SONAR_SCANNER)" ] || { \
		echo "ERROR: sonar-scanner not found (looked at $(SONAR_SCANNER))"; exit 1; \
	}
	@test -r "$(SONAR_TRUSTSTORE)" || { \
		echo "ERROR: truststore not found or not readable at $(SONAR_TRUSTSTORE)"; \
		echo "  (sonar.aaru.network uses a self-signed cert — regenerate with:"; \
		echo "     openssl s_client -connect sonar.aaru.network:443 -showcerts </dev/null 2>/dev/null \\\\"; \
		echo "       | awk '/BEGIN CERT/,/END CERT/' > /tmp/sonar-cert.pem && \\\\"; \
		echo "     keytool -importcert -alias sonar-aaru -file /tmp/sonar-cert.pem \\\\"; \
		echo "       -keystore $(SONAR_TRUSTSTORE) -storepass $(SONAR_TRUSTSTORE_PASSWORD) -noprompt)"; \
		exit 1; \
	}
	@TOKEN="$$(awk '/^SONAR_TOKEN=/{sub(/^[^=]*=[ \t]*/, ""); sub(/[ \t]+$$/, ""); print; exit}' ./.env)"; \
	test -n "$$TOKEN" || { echo "ERROR: SONAR_TOKEN is empty in .env"; exit 1; }; \
	SONAR_TOKEN="$$TOKEN" \
	SONAR_SCANNER_OPTS="-Djavax.net.ssl.trustStore=$(SONAR_TRUSTSTORE) -Djavax.net.ssl.trustStorePassword=$(SONAR_TRUSTSTORE_PASSWORD)" \
	"$(SONAR_SCANNER)" -Dsonar.host.url="$(SONAR_HOST_URL)"

clean:
	$(CARGO) clean
