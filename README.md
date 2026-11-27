# StoneQL

**Old habits. New engine.**

**StoneAge SQL Admin** · Formerly dbAdmin.

A compact, phpMyAdmin-inspired database client. The **desktop app (macOS, Windows and Linux)** connects live to MySQL, MariaDB and PostgreSQL servers; the same Vue interface also runs in a browser as a **demo** with sample data and simulated operations.

## Download

**[StoneQL v0.1.2](https://github.com/stoneAce/stoneql/releases/tag/v0.1.2)** · [all releases](https://github.com/stoneAce/stoneql/releases)

All assets are built from one release commit; `SHA256SUMS` covers them all. See the [download guide](docs/guide/download.md) to choose a file and check it.

| Platform | Asset | Status |
| --- | --- | --- |
| macOS Apple Silicon (arm64) | `.dmg` | Ad-hoc signed, **not notarized** (first launch: Privacy & Security → Open Anyway). Verified on macOS 27.0.1 |
| macOS Intel (x86_64) | `.dmg` | Signed like the arm64 build. Not yet tested on an Intel Mac |
| Windows x64 | NSIS installer `.exe` in a `.zip` | **Unsigned** (SmartScreen: More info → Run anyway); per-user install, WebView2 downloaded if missing. Direct connections only: **no SSH tunnels on Windows yet**. Tested on GitHub's Windows Server 2025 runners, not yet on a real Windows 10/11 PC |
| Linux x86_64 | `.deb` (Ubuntu 22.04+, Debian 12+) | Install with `sudo apt install ./<file>.deb`. Tested on Ubuntu 26.04 under Xvfb and installed on Ubuntu 22.04 and Debian 12 containers; GNOME Keyring prompt, master-password flow and Wayland not yet checked by hand |

Verify on macOS with `shasum -a 256 -c --ignore-missing SHA256SUMS`, on Linux with `sha256sum -c --ignore-missing SHA256SUMS`, and on Windows by comparing `Get-FileHash` output with `SHA256SUMS`. Then follow the [install guide](docs/guide/install.md) for installing, the first launch, updating and uninstalling. [Connect to a database](docs/guide/connect.md) walks through the first connection; [Troubleshooting](docs/guide/troubleshooting.md) covers first-launch warnings, saved-password prompts and connection errors. Report problems as [GitHub issues](https://github.com/stoneAce/stoneql/issues) with your OS version and the asset you used.

## What the app does

- Live MySQL, MariaDB and PostgreSQL over Direct or SSH tunnel (SSH on macOS and Linux), with verified TLS (MySQL/MariaDB over Direct can turn TLS off, with an RSA-protected password exchange as chosen). Passwords can be saved: macOS login Keychain; Windows DPAPI-encrypted files for the user account; Linux Secret Service, or without one StoneQL's master-password encrypted file.
- Read-write by default (MySQL/MariaDB Direct and SSH, PostgreSQL Direct), with a saved **Read-only connection** option; PostgreSQL over SSH is read-only. PROD-tagged connections ask for confirmation.
- Browse with server-side paging, sort and filter; insert, edit and delete rows of tables with a primary key; SQL scripts and transactions.
- MySQL/MariaDB: the Explorer lists every database the account can see, switches between them and creates databases.
- Structure: add, change, rename and drop columns, and add or drop secondary indexes (MySQL, MariaDB, PostgreSQL), with an SQL preview and confirmation.
- Export a table, chosen tables or a whole database as SQL, CSV (one table) or JSON, optionally GZIP; import CSV into a table and SQL files (MySQL/MariaDB).

Not yet available for live connections: Search, table Operations (rename, copy, truncate, drop), bulk Copy, managing views/routines/triggers/events, the database Structure overview, CSV import at database level, PostgreSQL SQL import, switching databases on PostgreSQL and exporting views/routines/triggers/events. See [What's in this release](docs/guide/beta.md#not-available-yet).

## Status

StoneQL is early software: keep backups and verify important changes on the server. Milestones M002–M007 and M009 remain open with recorded engineering gaps; Developer ID signing and notarization (macOS) and code signing (Windows) are deferred. Browser-demo operations are simulations and never touch a real database. No Axum server exists yet.

## Run

Requires Node.js **26** and npm. From the repository root:

```sh
npm ci
npm run dev
npm test
npm run build
npx playwright install chromium
npm run test:browser
npm run docs:dev
npm run docs:build
npm run docs:preview
```

`npm run preview` serves the frontend production bundle. See [contributor setup](docs/development/contributor-setup.md) for details and [verification](docs/development/verification.md) for executed checks and environment blockers.

## Repository

- `apps/frontend/`: shared Vue 3/TypeScript/Vite/Pinia/Tailwind/Lucide/CodeMirror frontend, mocks and tests.
- `docs/`: canonical public guides and architecture proposals.
- `.website/`: VitePress configuration/theme; renders root docs directly.
- `.ref/planning/`, `.ref/ms/`: internal audit, decisions and milestone specifications.
- `.ref/workprogress.md`: ignored local continuity notes; never commit.
- `.github/workflows/docs.yml`: manually gated GitHub Pages workflow for the [documentation site](https://stoneace.github.io/stoneql/).
- `.github/workflows/rust.yml`: Rust foundation checks on macOS 15 and Windows Server 2025 runners (the Windows job also builds and smoke-tests the NSIS installer on release branches), run when a release is published or started manually.

Building the desktop app from source is documented in [Desktop development](docs/development/desktop-host.md); the server path remains reserved. Shared Rust crates (core, contracts, runtime) and an isolated driver lab exist; see [Rust development](docs/development/rust-foundation.md) and [driver evaluation](docs/architecture/driver-evaluation.md). Read [AGENTS.md](AGENTS.md) before changes. The frontend remains browser-runnable without native APIs.

## Documentation

**User guide:** [Download](docs/guide/download.md) · [Install](docs/guide/install.md) · [Connect to a database](docs/guide/connect.md) · [Browse and edit tables](docs/guide/browse-and-edit.md) · [Run SQL](docs/guide/run-sql.md) · [Import and export](docs/guide/import-export.md) · [Troubleshooting](docs/guide/troubleshooting.md) · [What's in this release](docs/guide/beta.md)

**Developer Docs:** [Overview](docs/development/index.md) · [Contributor setup](docs/development/contributor-setup.md) · [Browser demo](docs/development/browser-demo.md) · [Architecture](docs/architecture/overview.md) · [Roadmap](docs/architecture/roadmap.md) · [Verification](docs/development/verification.md) · [Demo database objects](docs/database-objects.md)

The [preserved prototype reference](docs/development/prototype-reference.md) retains the previous detailed workflow documentation. Advanced autocomplete helpers are not all integrated; see the [SQL completion status](docs/development/browser-demo.md#sql-completion-status) rather than assuming helper existence proves support.

Production signing, notarization and server decisions remain gated; no commit certifies unverified capabilities. The documentation site is published manually from `docs/`.

## Contributing and community

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) to set up a development environment and run the checks, and follow the [Code of Conduct](CODE_OF_CONDUCT.md). For help using StoneQL see [SUPPORT.md](SUPPORT.md); report security problems privately as described in [SECURITY.md](SECURITY.md).

## Licence

StoneQL is released under the [MIT Licence](LICENSE). Vendored third-party code keeps its own licence notice (for example `vendor/sqlx-mysql/LICENSE-MIT` and `vendor/sqlx-postgres/LICENSE-MIT`).
