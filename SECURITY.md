# Security Policy

## Supported versions

Security fixes are provided for the latest published release only. Upgrade to the latest release before reporting a problem that may already be fixed.

| Version | Supported |
|---|---|
| Latest `0.1.x` release | Yes |
| Older releases | No |

## Report a vulnerability

Do not open a public issue. Use GitHub private vulnerability reporting:

https://github.com/MaJ2cxyK/Lumen/security/advisories/new

Please include:

- Lumen version and Windows version.
- Install method: release installer, local build, or source checkout.
- Provider, agent, and sandbox mode, if relevant.
- Reproduction steps and expected versus actual behavior.
- Impact assessment, including what an attacker could read, change, or execute.
- Relevant logs or screenshots with secrets and personal data removed.

Never include API keys, provider keys, access tokens, updater private keys, certificate passwords, database dumps, session exports, or personal data in a report.

## Scope

Reports are especially relevant for:

- Command approval bypasses or unsafe command parsing.
- Sandbox escapes, privilege escalation, path traversal, or unexpected host access.
- Secret disclosure, provider key leakage, or unsafe logging.
- Budget enforcement bypasses.
- Unsafe Git worktree, snapshot, review, or cleanup behavior.
- SQL injection, database corruption, or unsafe migrations.
- Updater, release, or artifact integrity failures.
- Tauri IPC, WebView, permission, or local file access issues.

Out of scope:

- Vulnerabilities in third-party CLIs or agents. Report those to the upstream project.
- Issues that require administrator or owner access to the local machine.
- Social engineering, physical access, or denial-of-service without a security impact.
- Demo and mock data that is clearly fictional.
- SmartScreen warnings for the currently unsigned installer while the SignPath application is pending.

## Response

This is a maintainer-run open-source project with no bounty program. We aim to acknowledge reports within 3 business days and provide an initial assessment within 10 business days. Coordinated disclosure is normally targeted within 90 days, sooner for active exploitation or high-impact issues.

## Safe harbor

Good-faith security research is welcome. Test only against your own installation and projects, do not access other users' data, and do not disrupt services. If you are unsure whether an action is safe, ask privately before testing.

## Hardening guidance

- Install only from the official Releases page.
- Treat CLI agents as code that runs with your user privileges.
- Use an isolated virtual machine for untrusted projects or providers.
- Keep a provider configured and the model proxy enabled when you rely on AppContainer isolation.
- Git workspaces deliberately fall back to process isolation because Git for Windows requires `NUL`; do not rely on AppContainer for Git tasks.
- Keep budgets and approval rules enabled for unattended runs.
