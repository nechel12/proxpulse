# Security Policy

## Supported versions

Security fixes are provided for the latest release and the `main` branch.
Older versions may not receive fixes.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting:

https://github.com/nechel12/proxpulse/security/advisories/new

Do not open a public issue for security-sensitive reports.

Please include:

- the ProxPulse version and operating system;
- steps to reproduce the issue;
- expected and actual behavior;
- impact and any relevant logs.

Do not include real proxy credentials, passwords, or other secrets. Redact them
from logs and screenshots before submitting a report.

## Scope

This policy covers the ProxPulse desktop application, its updater, and the
local proxy dispatcher. Vulnerabilities in third-party dependencies should be
reported upstream; if they affect ProxPulse, a private report is still welcome.

## Webhooks

Webhook URLs may contain secrets (tokens). Proxy credentials are stripped from
webhook payloads by default; the "send credentials" option requires HTTPS.
Treat webhook URLs as secrets and do not include them in reports.

Reports are reviewed on a best-effort basis. There is no bug bounty program.
