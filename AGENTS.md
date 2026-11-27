# Lumen Repository Rules

## Public repository release boundary

This repository is a public open-source project. The following content must never be staged with `git add`, committed, pushed, or exposed in issues, pull requests, commit messages, logs, screenshots, or chat output:

1. **Real credentials**: API keys, access tokens, refresh tokens, OAuth secrets, GitHub tokens, cloud keys, database connection strings, cookies, and session data.
2. **Private keys and signing material**: updater private keys, PFX/P12/PEM/KEY/JKS/keystore files, password files, code-signing certificate private keys, and their backups.
3. **Local private configuration**: `.env*`, `.keys`, `.certs`, local paths or account configuration, proxy credentials, `tauri.publish.conf.json`, and other release-injection files.
4. **Internal material**: audit reports, internal plans, unpublished security analysis, user or customer data, session exports, and screenshots containing sensitive information.
5. **Runtime and local data**: databases, SQLite/WAL files, settings, caches, browser and agent user directories, logs, crash dumps, HAR files, and backup archives.
6. **Build and release artifacts**: `dist`, `target`, `release`, installers, update packages, signing files, coverage reports, and generated `latest.json`.
7. **Third-party content that must not be published**: proprietary code, data, assets, or large binaries without authorization.

Public keys, clearly marked placeholders, and test-only fake values may be published only after confirming they do not point to real accounts, endpoints, or data and contain no authentication material.

`.gitignore` is a guard against accidental staging, not a substitute for judgment. Never use `git add -f` to bypass this rule, even for an ignored path.

## Before every commit and push

1. Run `npm run guard:public`. It scans the working tree and Git history for sensitive paths, credential patterns, and private-key markers.
2. Inspect `git diff --cached --name-only` and confirm that every staged file is safe to publish.
3. For release scripts, configuration, or CI changes, confirm that no local signing key, certificate password, or real update endpoint is written into the repository.
4. The CI `hygiene` job runs the same guard against the full history. Every failure must be fixed; it must never be skipped or ignored.

## If a leak occurs

1. Stop committing and pushing immediately, then revoke or rotate any exposed credential.
2. Record the affected files and commit range without repeating secret values in any output.
3. Remove the content from all reachable history and rewrite references. Deleting it only from the latest commit is not a fix.
4. After rewriting, verify that old commit SHAs, raw-file URLs, and cached URLs are inaccessible. If a platform cache remains, use the platform's supported cleanup mechanism.

## Ongoing requirements

Every subsequent agent, maintainer, and contributor must follow these rules. Exceptions are limited to public, sanitized placeholders explicitly approved by a maintainer. Real secrets have no exception.

## Public-facing language

All public-facing content must be written in English: the repository description, the primary `README.md`, release notes, commit subjects and bodies, issue and pull request text, screenshots, and demo media. Chinese is published only through the explicit `README.zh-CN.md` translation linked from the English README. Do not introduce new Chinese text into the English README, UI screenshots, demo recordings, or commit messages.

## Third-party content and brand marks

1. Do not add third-party logo files, icon fonts, brand color palettes, product screenshots, or marketing assets. Lumen renders self-drawn monochrome lettermarks and names compatible tools in text only.
2. Do not copy code, SQL, comments, or documentation from another project unless its license is compatible. MIT and Apache-2.0 code requires notice retention; Apache-2.0 also requires `NOTICE` handling. Never copy unlicensed, GPL, AGPL, or SSPL code.
3. Record every new design reference in `THIRD_PARTY_NOTICES.md` with its repository, license, concepts referenced, and whether any code or assets are included.
4. Screenshots and demo media must not contain third-party logos or copyrighted UI from another product. Keep reference screenshots, mood boards, and competitor UI captures outside version control; if they must stay local, use the ignored `参考图/` folder and never stage it.
5. Keep the trademark and affiliation disclaimer in both READMEs; it must state that names identify compatible tools only and imply no endorsement.
