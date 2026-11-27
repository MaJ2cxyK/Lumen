# Contributing to Lumen

Thanks for helping improve Lumen. This project is a local-first Windows workbench that launches CLI agents and can modify local projects, so changes involving approvals, sandboxing, budgets, Git state, or data deletion need extra care.

## Before you start

Read:

- [README.md](README.md) for setup, architecture, and known limitations.
- [AGENTS.md](AGENTS.md) for the public-repository content boundary.
- [SECURITY.md](SECURITY.md) for private vulnerability reporting.

Open an issue before starting a non-trivial feature or refactor. Small documentation, test, and bug-fix changes can go straight to a pull request.

## Development setup

Requirements:

- Windows 10/11 x64.
- Node.js 22+ and npm.
- Rust stable with MSVC Build Tools.
- WebView2 Runtime.
- Git, for worktrees, change review, and PR features.
- Optional: Chrome or Edge for the UI route audit.
- Optional: `gh` CLI for the "Create PR" flow.

```powershell
npm ci
npm run tauri dev
```

If Rust commands are not found, add `%USERPROFILE%\.cargo\bin` to `PATH` for the current shell.

## Workflow

1. Fork the repository and create a branch from `main`.
2. Use a descriptive branch name such as `feat/opencode-approvals`, `fix/approval-reset`, or `docs/security-policy`.
3. Keep commits small and focused.
4. Write commit subjects and bodies in English, using Conventional Commits style where practical.
5. Add or update tests for behavior changes.
6. Run the relevant quality gates before opening a pull request.
7. Update `CHANGELOG.md` for user-visible changes.

## Quality gates

Frontend:

```powershell
npm test
npm run build
npm run gate:ui
npm run test:e2e
```

Rust:

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

Public-repository hygiene:

```powershell
npm run guard:public
```

`npm run gate:ui` requires Chrome or Edge. The route audit checks both Chinese and English routes and fails on clipped shadows or missing expected content.

## Sensitive areas

Changes to these areas need failure-path tests and a short explanation of the threat model in the pull request:

- Approval parsing, rules, and "remember" behavior.
- Job Object and AppContainer isolation.
- Process launch, cancellation, and cleanup.
- Provider key storage, the local model proxy, and budget enforcement.
- Git worktrees, snapshots, review, cleanup, and data deletion.
- SQLite migrations and backup/restore.
- Updater configuration and release scripts.

## Public repository boundary

Do not commit or paste into an issue, pull request, log, screenshot, or chat:

- API keys, tokens, cookies, or database connection strings.
- Private keys, PFX/P12/PEM files, certificate passwords, or updater keys.
- `.env` files, local configuration, databases, logs, session exports, or build artifacts.
- Internal audit reports, private plans, or customer data.

Never use `git add -f` to bypass `.gitignore`. The complete rule is in [AGENTS.md](AGENTS.md).

## UI and language

English is the primary public language. Keep new English documentation, screenshots, demo recordings, commit messages, issues, and pull requests in English. Chinese is maintained only in `README.zh-CN.md` and the in-app translation layer.

For UI changes, include a screenshot or recording that shows the affected state. Do not add new Chinese strings to the English screenshots or demo media.

## Pull request checklist

- The change is focused and linked to an issue when appropriate.
- Tests cover the new behavior and important failure paths.
- UI changes include updated screenshots or a recording.
- `npm test`, `npm run build`, and relevant Rust checks pass.
- `npm run guard:public` passes.
- Documentation and `CHANGELOG.md` are updated for user-visible changes.
- The pull request text is in English and contains no secrets.

## Releases

Only maintainers create version tags. A `v*` tag triggers the release workflow, builds the Windows installer and updater artifacts, and publishes a GitHub Release. Do not create release tags from a pull request.
