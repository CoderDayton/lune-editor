# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✓         |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Please report security issues by emailing **coderdayton14@gmail.com** with:

- A description of the vulnerability and its impact
- Steps to reproduce or a proof-of-concept
- Any suggested mitigations (optional)

You will receive a response within 72 hours. If the issue is confirmed, a patch
will be prioritised and a CVE requested where appropriate.

## Scope

Areas of particular concern:

- **File system access** — path traversal in file operations or the file tree
- **Process execution** — command injection via AI client spawning or PTY
- **Config parsing** — TOML deserialisation of untrusted config/theme files
- **Git operations** — repository path handling via libgit2

## Out of Scope

- Denial-of-service through large files (best-effort only)
- Cosmetic UI bugs with no security impact
