# Security Policy

<!--
    TEMPLATE — this file is copied into each per-tool repo as part of
    the Phase 1–4 extractions (#80). Before committing, replace:

      jasonherald   e.g. jasonherald
      nwg-dock    e.g. nwg-common / nwg-dock / nwg-drawer / nwg-notifications

    Also review the "Scope" section below — the default lists every
    user-visible behavior across the four tools; prune to just the
    behaviors this specific repo is responsible for:

      nwg-common         Library only — no runtime behavior of its own;
                         scope inherits from whichever binary consumes it.
                         Consider pointing the reader at the binaries'
                         SECURITY.md for behavioral scope.
      nwg-dock           Executes `.desktop` Exec commands via compositor;
                         reads/writes pin state; compositor IPC.
      nwg-drawer         Executes `.desktop` Exec commands via compositor;
                         reads/writes pin state; compositor IPC.
      nwg-notifications  Listens on D-Bus (org.freedesktop.Notifications);
                         compositor IPC for window-focus signals.
-->

## Supported Versions

Only the latest release on the `main` branch is supported with security updates. We do not backport fixes to older versions.

| Branch | Supported |
|--------|-----------|
| `main` | Yes |
| Other  | No  |

## Reporting a Vulnerability

**Please do not open a public issue for security vulnerabilities.**

Use GitHub's private vulnerability reporting to submit a report:

1. Go to the [Security tab](https://github.com/jasonherald/nwg-dock/security)
2. Click **"Report a vulnerability"**
3. Provide a description, steps to reproduce, and any relevant details

### What to expect

- **Acknowledgment** within 48 hours
- **Assessment** of severity and impact within 1 week
- **Fix or mitigation** as soon as practical, depending on severity
- **Disclosure** 90 days after the fix is released, or immediately if the vulnerability is already public
- Credit in the fix commit (unless you prefer to remain anonymous)

## Security Scanning

This project uses automated security scanning across multiple layers:

| Tool | Integration | Coverage |
|------|-------------|----------|
| [CodeQL](https://codeql.github.com/) | GitHub Actions (PR + weekly) | Source-level OWASP analysis (command injection, path traversal, tainted data flows) |
| [cargo-audit](https://rustsec.org/) | GitHub Actions (PR + weekly) | Known CVEs in Rust dependencies (RustSec advisory database) |
| [cargo-deny](https://embarkstudios.github.io/cargo-deny/) | GitHub Actions (PR + weekly) | License compliance, duplicate crates, source restrictions |
| [CodeRabbit](https://coderabbit.ai/) | GitHub App (PR review) | AI-assisted code review with OSV dependency scanning |
| [SonarQube](https://www.sonarqube.org/) | External (pre-PR) | Code quality, cognitive complexity, code smells |

## Scope

This project runs as a user-space application on Wayland compositors (Hyprland, Sway). It:

- Executes `.desktop` file `Exec=` commands via the compositor
- Reads/writes pin state to `~/.cache/mac-dock-pinned`
- Listens on D-Bus as a notification daemon (`org.freedesktop.Notifications`)
- Communicates with the compositor via IPC sockets

Vulnerabilities in any of these areas are in scope.

### Out of scope

- Bugs in the compositor itself (Hyprland, Sway) — report upstream
- Denial of service via resource exhaustion (e.g. sending thousands of notifications)
- Malicious `.desktop` files — these are user-installed and trusted by design
- Social engineering or phishing
- Issues requiring physical access to the machine
