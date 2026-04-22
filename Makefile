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
# still running via the `nwg-dock-hyprland` symlink. The output is
# pipelined through `sort -u` to dedupe — a process launched via the
# symlink gets reported under both names, which would otherwise cause
# --dump-args + kill + restart to run twice on the same pid.
#
# Install-target validation (issue #35): before killing anything,
# resolve /proc/$PID/exe for each running dock and compare against
# where this upgrade would install ($(BINDIR)/$(BIN_NAME)). If they
# don't match — usually because the user installed to ~/.cargo/bin
# but invoked upgrade without re-passing PREFIX/BINDIR, so we'd try
# to install to /usr/local and fail on permission — we abort with a
# helpful error BEFORE touching the dock. Previously the recipe
# killed the dock first and then failed the install, leaving the
# desktop with no visible dock and no binary update.
#
# Symlink handling: /proc/$pid/exe is a kernel-resolved canonical
# path, not a symlink. `readlink -f` on both sides (running exe +
# install target) ensures the comparison works even when the user
# launched via the `nwg-dock-hyprland` alias (the alias resolves to
# the nwg-dock binary's real path, which is what /proc reports).
#
# Atomicity: recipe order is validate → capture args → install →
# kill → restart. Install happens while the dock is still running
# (Linux's mmap semantics make this safe — `install` unlinks the
# destination and writes a new file; the running process's loaded
# pages survive intact until the process exits). If install fails,
# the dock is never killed.
#
# PID identity validation (CodeRabbit follow-up): capture
# `/proc/$PID/stat` field 22 (starttime — clock ticks since boot)
# alongside each pid at discovery; re-verify before SIGTERM and
# SIGKILL. Starttime is kernel-authoritative and unique per
# (pid, boot), so a reused pid with a different process attached
# gets dropped from the kill list rather than SIGKILLed blindly.
#
# SIGKILL escalation + refuse-restart-on-failure (CodeRabbit
# outside-diff finding): old code did `kill $PIDS || true; sleep 1;
# restart` — if SIGTERM silently failed or a process ignored it,
# we'd start a new dock alongside the old one (2 docks; singleton
# lockfile would prevent the second from staying alive but it's
# still wrong). Now: after SIGTERM+sleep, check each pid's
# starttime; still-alive pids get SIGKILL; if any survive SIGKILL
# the recipe fails BEFORE restart so you never end up with two
# dock instances fighting for the layer surface.
#
# --dump-args failure handling: a failure is only swallowed when
# the pid has actually disappeared (no `/proc/$PID/exe`). If
# --dump-args fails on a still-live dock that's a real bug —
# fail-fast with an explicit error rather than silently killing
# the dock without capturing its args.
upgrade: build-release
	@RUNNING_PIDS="$$(pidof -c $(BIN_NAME) $(LEGACY_BIN_NAME) 2>/dev/null | tr ' ' '\n' | sort -u | tr '\n' ' ' | sed 's/ $$//' || true)"; \
	if [ -n "$$RUNNING_PIDS" ]; then \
		INSTALL_TARGET="$(DESTDIR)$(BINDIR)/$(BIN_NAME)"; \
		INSTALL_TARGET_REAL="$$(readlink -f "$$INSTALL_TARGET" 2>/dev/null || echo "$$INSTALL_TARGET")"; \
		for pid in $$RUNNING_PIDS; do \
			RUNNING_EXE="$$(readlink -f "/proc/$$pid/exe" 2>/dev/null)"; \
			if [ -z "$$RUNNING_EXE" ]; then \
				if [ -d "/proc/$$pid" ]; then \
					echo "ERROR: unable to resolve /proc/$$pid/exe for live dock pid $$pid"; \
					echo "       (process is alive but its exe symlink is unreadable — refusing to proceed"; \
					echo "        without install-target validation)"; \
					exit 1; \
				fi; \
				continue; \
			fi; \
			if [ "$$RUNNING_EXE" != "$$INSTALL_TARGET_REAL" ]; then \
				RUNNING_BINDIR="$$(dirname "$$RUNNING_EXE")"; \
				RUNNING_PREFIX="$$(dirname "$$RUNNING_BINDIR")"; \
				echo "ERROR: running dock (pid $$pid) is installed at"; \
				echo "         $$RUNNING_EXE"; \
				echo "       but 'make upgrade' would install to"; \
				echo "         $$INSTALL_TARGET"; \
				echo ""; \
				echo "       Dock NOT killed — a prefix-mismatched upgrade would leave"; \
				echo "       you with no dock on your desktop and no new binary."; \
				echo ""; \
				echo "       Re-run with PREFIX/BINDIR matching the running binary:"; \
				echo "         make upgrade PREFIX=$$RUNNING_PREFIX BINDIR=$$RUNNING_BINDIR"; \
				echo "       (or stop the dock manually and re-run make install)."; \
				exit 1; \
			fi; \
		done; \
		ARGS_FILE="$$(mktemp)" || exit 1; \
		RUNNING_INFO="$$(mktemp)" || exit 1; \
		trap 'rm -f "$$ARGS_FILE" "$$RUNNING_INFO"' EXIT; \
		for pid in $$RUNNING_PIDS; do \
			START_TIME="$$(sed 's/.*) //' "/proc/$$pid/stat" 2>/dev/null | awk '{print $$20}' || true)"; \
			test -n "$$START_TIME" || continue; \
			if ! DUMP_OUT="$$(target/release/$(BIN_NAME) --dump-args "$$pid" 2>/dev/null)"; then \
				ACTUAL_START="$$(sed 's/.*) //' "/proc/$$pid/stat" 2>/dev/null | awk '{print $$20}' || true)"; \
				ACTUAL_EXE="$$(readlink -f "/proc/$$pid/exe" 2>/dev/null || true)"; \
				if [ -n "$$ACTUAL_START" ] && [ "$$ACTUAL_START" = "$$START_TIME" ] && \
				   [ "$$ACTUAL_EXE" = "$$INSTALL_TARGET_REAL" ]; then \
					echo "ERROR: --dump-args failed for live dock pid $$pid"; \
					exit 1; \
				fi; \
				continue; \
			fi; \
			printf "%s\t%s\n" "$$pid" "$$DUMP_OUT" >> "$$ARGS_FILE" || exit 1; \
			echo "$$pid $$START_TIME" >> "$$RUNNING_INFO" || exit 1; \
		done; \
		$(MAKE) install-bin install-data || exit 1; \
		VALIDATED_PIDS=""; \
		while IFS=' ' read -r pid start_time; do \
			ACTUAL_START="$$(sed 's/.*) //' "/proc/$$pid/stat" 2>/dev/null | awk '{print $$20}' || true)"; \
			if [ -n "$$ACTUAL_START" ] && [ "$$ACTUAL_START" = "$$start_time" ]; then \
				kill "$$pid" 2>/dev/null || true; \
				VALIDATED_PIDS="$$VALIDATED_PIDS $$pid"; \
			else \
				echo "Skipping pid $$pid — no longer our dock (starttime changed or process exited between capture and kill)"; \
			fi; \
		done < "$$RUNNING_INFO"; \
		if [ -n "$$VALIDATED_PIDS" ]; then \
			echo "Sent SIGTERM to:$$VALIDATED_PIDS"; \
			sleep 1; \
			STILL_RUNNING=""; \
			for pid in $$VALIDATED_PIDS; do \
				START_TIME="$$(grep "^$$pid " "$$RUNNING_INFO" | awk '{print $$2}')"; \
				ACTUAL_START="$$(sed 's/.*) //' "/proc/$$pid/stat" 2>/dev/null | awk '{print $$20}' || true)"; \
				if [ -n "$$ACTUAL_START" ] && [ "$$ACTUAL_START" = "$$START_TIME" ]; then \
					kill -9 "$$pid" 2>/dev/null || true; \
					STILL_RUNNING="$$STILL_RUNNING $$pid"; \
				fi; \
			done; \
			if [ -n "$$STILL_RUNNING" ]; then \
				echo "Escalated to SIGKILL:$$STILL_RUNNING"; \
				sleep 1; \
				FINAL_ALIVE=""; \
				for pid in $$STILL_RUNNING; do \
					START_TIME="$$(grep "^$$pid " "$$RUNNING_INFO" | awk '{print $$2}')"; \
					ACTUAL_START="$$(sed 's/.*) //' "/proc/$$pid/stat" 2>/dev/null | awk '{print $$20}' || true)"; \
					if [ -n "$$ACTUAL_START" ] && [ "$$ACTUAL_START" = "$$START_TIME" ]; then \
						FINAL_ALIVE="$$FINAL_ALIVE $$pid"; \
					fi; \
				done; \
				test -z "$$FINAL_ALIVE" || { \
					echo "ERROR: failed to stop$$FINAL_ALIVE after SIGKILL; refusing to restart while old dock still running"; \
					exit 1; \
				}; \
			fi; \
		fi; \
		if [ -n "$$VALIDATED_PIDS" ] && [ -s "$$ARGS_FILE" ]; then \
			for pid in $$VALIDATED_PIDS; do \
				args="$$(awk -v p="$$pid" 'BEGIN{FS="\t"} $$1==p{sub(/^[^\t]*\t/, ""); print; exit}' "$$ARGS_FILE")"; \
				test -n "$$args" || continue; \
				echo "Restarting with captured args: $$args"; \
				setsid sh -c "$$args" </dev/null >/dev/null 2>&1 & \
			done; \
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
