# What's in this release

StoneQL **v0.1.1** is published as a [GitHub release](https://github.com/stoneAce/stoneql/releases/tag/v0.1.1). It follows three public betas: [v0.1.0-beta.3](https://github.com/stoneAce/stoneql/releases/tag/v0.1.0-beta.3), [v0.1.0-beta.2](https://github.com/stoneAce/stoneql/releases/tag/v0.1.0-beta.2), the first build for Windows and Linux, and [v0.1.0-beta.1](https://github.com/stoneAce/stoneql/releases/tag/v0.1.0-beta.1), which was for Apple Silicon Macs only.

## What's new in 0.1.1

- **Connections stay connected.** A live connection now ends only after 8 hours without use, instead of after a fixed hour. StoneQL keeps the sign-in in app memory and holds no connection to the server while idle: after 60 seconds without use it closes the database connection and signs in again on your next action. See [Manage connections](./connect.md#manage-connections).
- **Reconnects by itself** after a server restart or a dropped connection. If the connection did end and its password is saved, StoneQL reconnects with the same access and database. It never does this for a Production read-write connection or after you disconnect.
- **Security updates** to the libraries StoneQL is built with.

## What's new in beta.3

- **Use TLS is off for new MySQL and MariaDB connections over Direct**, for this computer and others, with a warning for other computers. Connections saved earlier keep their setting. See [TLS and certificates](./connect.md#tls-and-certificates).
- **Password without TLS:** after a MySQL restart, sign in without TLS by encrypting the password with the server's RSA public key, either one you trust or, as an explicit choice, one fetched from the server. RSA protects only the password; queries and results stay unencrypted.
- **Clearer errors** when a server accepts only TLS, or a sign-in without TLS fails.
- **Fix:** sign-in without TLS to MySQL servers whose default sign-in method is `mysql_native_password` (common on MySQL 8.0).
- **Windows:** the installer now comes inside a `.zip`.

## What's new in beta.2

- **Windows:** a 64-bit (x64) installer. Direct connections, read-write, and saved passwords encrypted for your Windows account.
- **Linux:** a 64-bit (x86_64) `.deb` package for Ubuntu 22.04 or later and Debian 12 or later. Direct and SSH-tunnel connections, read-write, and saved passwords in the desktop keyring or in StoneQL's own encrypted file.
- **Intel Macs:** a separate DMG built for Intel (x86_64).
- **Checksums:** one `SHA256SUMS` file covers every download; see [Check the download](./download.md#check-the-download).
- **Saved passwords:** the connection dialog says where a new password will be saved, and a saved password's field says which store holds it.

## Platforms

| Platform | Status | Tested on |
| --- | --- | --- |
| macOS, Apple Silicon (arm64) | Available. Ad-hoc signed, **not notarized**: the [first launch needs approval](./install.md#approve-the-first-launch) | macOS 27.0.1 only; older versions are untested |
| macOS, Intel (x86_64) | Available. Signed and approved like the Apple Silicon build | Not yet tested on an Intel Mac |
| Windows 10 and 11, x64 | Available. **Unsigned** installer: SmartScreen asks before it runs ([steps](./install.md#run-the-installer)). **No SSH tunnels yet** | GitHub's Windows Server 2025 machines, including real MySQL, MariaDB and PostgreSQL databases and a silent install and launch. Not yet tested on a real Windows 10 or 11 PC |
| Ubuntu 22.04+ and Debian 12+, x86_64 | Available. Unsigned `.deb` package | Ubuntu 26.04 in a virtual display; the package installed and launched on Ubuntu 22.04 and Debian 12. Not yet tested on a real desktop session |

Developer ID signing and notarization for macOS, and code signing for Windows, are not available yet.

## What works

- **Databases:** MySQL, MariaDB and PostgreSQL, live. SQLite is not included in the beta app.
- **Connections:** Direct, or through an SSH tunnel on macOS and Linux (SSH agent, password or Ed25519/ECDSA private key) with explicitly trusted host keys. **Use TLS** for MySQL/MariaDB over Direct (off for new connections), always verified TLS for PostgreSQL and SSH tunnels. Passwords can be saved: see [Saved passwords](./connect.md#saved-passwords). See [Connect to a database](./connect.md).
- **Access:** read-write by default (MySQL/MariaDB Direct and SSH, PostgreSQL Direct), with a saved **Read-only connection** option. PostgreSQL over SSH is always read-only. PROD connections ask for confirmation.
- **Browsing and editing:** server-side paging, sort and filter; insert, edit and delete rows in tables with a primary key. See [Browse and edit tables](./browse-and-edit.md).
- **Databases (MySQL/MariaDB):** list every database the account can see, switch between them and create databases.
- **Structure:** add, change, rename and drop columns, and add or drop secondary indexes, with an SQL preview and confirmation.
- **SQL:** single read-only queries, or read-write scripts with transactions, cancellation and honest “unknown” outcomes. See [Run SQL](./run-sql.md).
- **Import and export:** export a table, chosen tables or a database as SQL, CSV (one table) or JSON, optionally GZIP; import CSV into a table, and SQL files into MySQL/MariaDB. See [Import and export](./import-export.md).
- **Interface:** Light and Dark themes, several connections side by side, connection groups and colors.

## Known limitations by platform

### macOS

- Ad-hoc signed and not notarized: approve the first launch, and expect the [Keychain prompts](./troubleshooting.md#keychain-prompts) once after each update.
- The Intel build has not been tested on an Intel Mac.

### Windows

- **SSH tunnels are not available on Windows yet.** The SSH Tunnel option still appears in the connection dialog, but it cannot connect. Use Direct connections.
- Saved passwords are encrypted for your Windows account. Any program running as your Windows user can decrypt them, the same as with Windows Credential Manager.
- If StoneQL's window goes blank because its web view crashed, StoneQL does not recover by itself: quit and reopen it.
- The list of [unknown outcomes](./run-sql.md#unknown-outcomes) is stored with fewer safeguards than on macOS and Linux: StoneQL does not check the folder's access permissions, and a power cut at the wrong moment may lose the latest update to the list.
- Not yet tested on a real Windows 10 or 11 PC.

### Linux

- Saved passwords use the desktop keyring (Secret Service); only GNOME Keyring was tested. Without a running keyring, StoneQL uses its own encrypted file and a master password; see [Saved passwords](./connect.md#saved-passwords).
- Not yet checked on a real desktop session: the GNOME Keyring unlock prompt, the master-password dialog in the app window, and Wayland.
- Only `.deb` packages for x86_64; no AppImage, RPM or ARM build yet.

## Not available yet

These features are hidden in the desktop app rather than simulated:

- the Search tab and bulk Copy of selected rows;
- table Operations (rename, copy, truncate, drop);
- managing views, routines, triggers and events, and exporting them;
- the database-level Structure overview;
- switching to another database on PostgreSQL connections;
- CSV import at database level, and SQL import into PostgreSQL.

Also not supported yet:

- SSH tunnels on Windows;
- RSA SSH keys, `~/.ssh/config`, jump hosts and two-factor SSH sign-in;
- Teleport and AWS SSM connections (shown as *coming later*);
- SQLite files (development builds only).

## How it was tested

- **macOS Apple Silicon:** beta.1 was checked by hand against **MySQL over a direct connection** on macOS 27.0.1. MariaDB, PostgreSQL and SSH tunnels were tested through the app's connection engine against test servers (MySQL 8.4, MariaDB 11.4 and PostgreSQL 17), not by hand in the packaged app.
- **Windows:** automated tests on GitHub's Windows Server 2025 machines, including real MySQL, MariaDB and PostgreSQL databases, and a silent install and launch of the installer. Not yet tested on a real Windows 10 or 11 PC.
- **Linux:** automated tests on Ubuntu 26.04 in a virtual display, including real databases, SSH tunnels and a throwaway GNOME Keyring. The `.deb` package was installed and launched on Ubuntu 22.04 and Debian 12.
- **macOS Intel:** built for Intel; not yet tested on an Intel Mac.

StoneQL is not a backup tool. Keep backups, use accounts with only the privileges you need, and verify important changes on the server.

## What to test

If you try StoneQL, these are the most useful checks:

1. **Install** the file for your system, including the first-launch warning ([Install](./install.md)).
2. **Launch** StoneQL and reach the **Welcome to StoneQL** screen.
3. **Connect** to a database you can safely use ([Connect to a database](./connect.md)).
4. **Save the password and reconnect:** connect with the password saved, quit StoneQL, open it again and connect without typing the password.

On Linux, saving a password with and without a desktop keyring running is especially useful. On Windows, any result from a real Windows 10 or 11 PC helps.

## How to report a problem

Open an issue in the [StoneQL repository on GitHub](https://github.com/stoneAce/stoneql/issues). Include:

- your operating system and version (for example *Windows 11 24H2*, *Ubuntu 24.04*, *macOS 27.0.1*), and for a Mac whether it is Apple Silicon or Intel;
- the name of the file you installed;
- what you did, what you expected and what happened, with the exact message StoneQL showed.

Never include passwords, keys or private data in a report.

## Browser demo

The same interface also runs in a web browser with sample data and simulated operations, for trying the interface and for development. Nothing in the demo touches a real database; never enter real credentials there. See the [browser demo](../development/browser-demo.md) in the Developer Docs.
