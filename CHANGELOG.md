# Changelog

All notable changes to Lumen are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.3] - 2026-11-27

### Changed

- Replaced the bundled third-party CLI logos with self-drawn monochrome lettermarks.
- Rewrote `THIRD_PARTY_NOTICES.md` with the documented design references, the brand-asset policy, and the code-comparison record.
- Added third-party content and brand-mark rules to `AGENTS.md`.
- Reworded team-template names and role descriptions so no external UI copy is carried over.

## [0.1.2] - 2026-11-26

### Added

- Private vulnerability reporting, `SECURITY.md`, and security issue routing.
- `CONTRIBUTING.md`, bug and feature request forms, and this changelog.
- Playwright smoke test for the core create, dispatch, approval, and review workflow.
- Auto-update verified end to end from `0.1.1` to `0.1.2`.

### Changed

- Public README, screenshots, demo recording, and repository history now use English as the primary language.
- The demo recording was reduced from 2.5 MB to about 818 KB without changing its dimensions.

### Fixed

- The UI route audit now expects the English project name on the localized project page.

## [0.1.1] - 2026-11-26

### Added

- Windows NSIS installer and Tauri updater artifacts published through GitHub Releases.
- Auto-update verified end to end from `0.1.0` to `0.1.1`.
- Sandbox exit diagnostics with the `git_nul` reason and backward-compatible mapping.
- Public-repository hygiene check for credentials, private keys, internal material, local data, and release artifacts.

### Changed

- Approval-blocked presentation, dispatch reset, and frontend status copy.
- Code signing policy and the SignPath reapplication threshold.
- PowerShell 7 compatibility in the signature verification script.

### Fixed

- The public-repository guard now returns a zero exit code after a successful check.

## [0.1.0] - 2026-11-25

### Added

- First public release of the local-first multi-agent workbench.
- Projects, teams, seats, tasks, runs, approvals, budgets, usage tracking, and Git change review.
- Windows process isolation, AppContainer integration, SQLite persistence, backups, and updater support.
- MIT license and an explicit code signing policy.

[Unreleased]: https://github.com/MaJ2cxyK/Lumen/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/MaJ2cxyK/Lumen/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/MaJ2cxyK/Lumen/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/MaJ2cxyK/Lumen/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/MaJ2cxyK/Lumen/releases/tag/v0.1.0
