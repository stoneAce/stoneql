# Connect to a database

StoneQL connects to **MySQL**, **MariaDB** and **PostgreSQL** servers, either directly or through an SSH tunnel (SSH tunnels are not available on Windows yet). You need the server's host name and port and a database account (user name and password).

::: tip Start safely
New connections are **read-write**: SQL scripts, row edits and structure changes can change data. When you explore a server that matters, use an account with only the privileges you need, or tick **Read-only connection** (step 6).
:::

## Create your first connection

1. **Open the connection manager.** On the **Welcome to StoneQL** screen choose **Manage connections**. (Later you can also use the **+** next to the connection tabs, or **Manage connections** at the top right.)
2. **Start a new connection.** Choose **New connection** at the bottom of the connection manager.
3. **Name it and pick the engine.** Enter a **Connection name** (for example *Local development*) and choose the **Database engine**: MySQL, MariaDB or PostgreSQL. The port changes to the engine's default (3306 or 5432). SQLite is listed but is not included in the released app.
4. **Optional: label it.** Choose an **Environment** (Local, Development, Staging, Production or Custom), a **Group** and a **Connection color**. Connections tagged **Production** show a **PROD** badge and ask for extra confirmation; see [Production connections](#production-connections).
5. **Enter the server and account.** Keep **Direct** selected (or see [Connect through an SSH tunnel](#connect-through-an-ssh-tunnel)), then fill in **Host**, **Port**, **Username** and **Password**. These fields are used exactly as typed.
6. **Choose read-write or read-only.** Leave **Read-only connection** unticked for full access, or tick it to allow only reading. The choice is saved with the connection. See [Read-only or read-write](#read-only-or-read-write).
7. **Check encryption.** **Use TLS** is off for a new MySQL or MariaDB connection. Turn it on to encrypt the connection and verify the server; see [TLS and certificates](#tls-and-certificates).
8. **Decide whether to remember the password.** **Remember database password** is ticked by default; the note under it says where the password will be saved. See [Saved passwords](#saved-passwords). Untick it to type the password each time you connect.
9. **Test and choose a database.** Choose **Test connection**. A successful test shows the databases your account can see in **Databases on this server**: pick one. You can instead type a name in **Default database**. A default database is required to save.
10. **Connect.** Choose **Save & connect**. (**Save** stores the connection without connecting.)

### You are connected when…

- The **Explorer** on the left lists your databases (MySQL/MariaDB) or the tables of your database (PostgreSQL).
- The status bar at the bottom shows **Live read-write · connected** or **Live read-only · connected**.
- Selecting a table opens its rows in **Browse**.

If the connection fails, the dialog shows a message. Look it up in [Troubleshooting](./troubleshooting.md#connection-errors).

Next: [Browse and edit tables](./browse-and-edit.md).

## Read-only or read-write

- **Read-write** (the default) lets you run SQL scripts, use transactions, edit rows, change table structure and import data. Confirmations in StoneQL are safeguards, not permissions: your database account decides what is allowed.
- **Read-only connection** lets you browse, run single `SELECT` queries and export. The database itself enforces it: StoneQL runs each query inside a read-only transaction.
- **PostgreSQL through an SSH tunnel** is always read-only for now.

The SQL tab and the status bar show **Read-only** or **Read-write** for the open connection. To change it, edit the connection and connect again.

## Production connections

Set **Environment** to **Production** for servers that matter. These connections show **PROD**, and each time one connects read-write StoneQL asks you to confirm with **PROD — I understand**. Destructive actions on them always ask for confirmation.

## TLS and certificates

> **New in beta 3.** **Use TLS** off for new connections (including other computers) and **Password without TLS**. In beta 2, TLS could be turned off only for this computer, and after a MySQL restart you signed in once with the `mysql` command-line client before connecting without TLS.

- **Use TLS on:** StoneQL verifies the server's certificate and host name. It never falls back to an unencrypted connection.
- **MySQL or MariaDB over Direct** shows **Use TLS**, off for new connections. Without TLS, queries, results and the server's identity are unprotected on the network; for a server on another computer the dialog warns about this. Turn **Use TLS** on whenever the server has a certificate you can trust. Connections saved before this option for another computer keep TLS on. StoneQL never switches between TLS and no TLS by itself.
- **PostgreSQL** and SSH-tunnelled connections always use verified TLS.
- **Password without TLS:** with **Use TLS** off, MySQL sometimes asks for a full sign-in, which sends the password itself: for `caching_sha2_password` accounts (the MySQL 8 default) after the server restarts, until the account has signed in once. Without TLS the password must then be encrypted with the server's RSA public key. Choose under **Password without TLS**:
  - **Don't send it without TLS** (default): the sign-in stops before the password is sent. Accounts the server has already cached, and MariaDB accounts, still connect.
  - **Encrypt it with the server's public key (trusted):** paste the server's public key (`-----BEGIN PUBLIC KEY-----`). StoneQL never asks the server for it. It is saved with the connection; it is not secret. To get it, ask the server's administrator, copy `public_key.pem` from the server's data directory, or run this once over a connection you trust:

    ```sh
    mysql --raw -N -B -e "SELECT VARIABLE_VALUE FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Caching_sha2_password_rsa_public_key'"
    ```

  - **Fetch the key from the server without verifying it:** convenient, but anyone who can impersonate the server on the network can send their own key and read your password. Use it only when you trust the network path.

  RSA protects only the password. Queries and results are still sent unencrypted; use TLS for that.
- **Your own certificate authority:** if the server's certificate is issued by a private CA, choose its PEM file under **Trusted CA certificate (optional, session only)**. It is kept only until StoneQL quits (16 KiB maximum; private keys are rejected). **Use system trust roots** removes it.

## Saved passwords

With **Remember database password** ticked, StoneQL saves the password in your system's password store, never in its connection settings. The note under the password field says where a new password goes, and the field of a saved password names the store that holds it (for example *Saved in Keychain*). The saved password works only for the same server, account and route; if you change those, type it again.

Unticking the option, or removing the connection, deletes the saved password. If the store refuses (for example because it is locked), StoneQL tells you and saves nothing; removing a connection then offers **Remove anyway**.

The same applies to **Remember SSH password** and **Remember key passphrase** for SSH tunnels.

### macOS: the Keychain

Passwords are saved in your login Keychain. The first time a new StoneQL build reads a password saved by an earlier build (for example after an update), macOS asks for your login password, usually twice. Choose **Always Allow**. See [Keychain prompts](./troubleshooting.md#keychain-prompts).

### Windows: encrypted for your Windows account

Passwords are saved as encrypted files in `%APPDATA%\me.thixpin.stoneql.credentials\`, using Windows' own encryption for your user account (DPAPI). Only your Windows account can decrypt them, but **any program running as your Windows user can**, the same as with Windows Credential Manager. The field of a saved password shows *Saved (Windows encrypted)*.

### Linux: the desktop keyring, or StoneQL's encrypted file

- **With a desktop keyring** (Secret Service, such as GNOME Keyring; only GNOME Keyring was tested), passwords are saved there. If the keyring is locked, it asks you to unlock it. If you dismiss that prompt, or the keyring refuses for another reason, the password is **not saved**: StoneQL never falls back to its own file while a keyring is running.
- **Without a desktop keyring**, StoneQL saves passwords in its own encrypted file, protected by a **master password**:
  - The first time you save a password, StoneQL asks you to choose a master password of at least 8 characters.
  - You enter it once each time StoneQL starts, when a saved password is first needed.
  - **It cannot be recovered.** If you forget it, choose **Reset saved passwords…** in the unlock dialog: this deletes every password saved in the file. Your connections keep their other settings and ask for their passwords again.

A password saved in one store stays there: if a keyring starts running later, new passwords go to the keyring and passwords already in the file still work from the file.

## Connect through an SSH tunnel

Use this when the database is reachable only from a server you can SSH into.

::: warning Not available on Windows yet
SSH tunnels work on macOS and Linux. On Windows the **SSH Tunnel** option still appears, but it cannot connect; use **Direct**.
:::

1. In the connection dialog choose **SSH Tunnel**.
2. Enter **Host** and **Port** of the database **as the SSH server sees it**, plus the database **Username** and **Password**. Connections through a tunnel always use verified TLS, checked against this **Host**, so use a host name that the database server's certificate covers.
3. Under **SSH tunnel configuration**, enter the **SSH host**, **SSH port** and **SSH username**.
4. Choose an **Authentication** method:
   - **SSH agent:** uses the SSH agent available to StoneQL. Agent forwarding is never requested.
   - **Password:** enter the **SSH password**; optionally tick **Remember SSH password**.
   - **Private key:** use **Choose key file…** and, if the key is encrypted, enter the **Key passphrase** (optionally saved with **Remember key passphrase**). Ed25519 and ECDSA keys work; RSA keys are not supported yet. Choose the file again after restarting StoneQL.
5. **Trust the host key.** Choose **Fetch host key** (or **Test connection**). StoneQL shows the key type and its SHA256 fingerprint. Compare it with the fingerprint from the server's administrator (for example from `ssh-keygen -lf` on the server). Only if they match, choose **Trust this host key** and confirm. Otherwise choose **Do not trust**.
6. Continue from step 8 of [Create your first connection](#create-your-first-connection).

Trust applies to every connection that uses the same SSH server and port. Changing the SSH host or port shows whether the new server is trusted. Connections saved by earlier StoneQL versions need their host key trusted again. If the server later presents a different key, StoneQL refuses to connect; see [SSH host key changed](./troubleshooting.md#ssh-host-key-unknown-or-changed).

While a test or connection is in progress, **Cancel connecting** stops it and discards the partial tunnel.

## Manage connections

- **Groups:** create, rename and delete collapsible groups in the connection manager, and move a connection with its **Group** selector. Deleting a group keeps its connections under *Ungrouped*.
- **Edit or remove** a connection from the connection manager.
- **Several connections at once:** each open connection has its own tab with its own Explorer, SQL tabs and results.
- **Disconnect** from the SQL tab, or close the connection's tab.
- **Staying connected:** while a connection is open, StoneQL keeps its password in the app's memory (never on disk, and never in the window's web page). It doesn't hold a database connection while you're idle: it closes it after a minute and signs in again on your next action, so a restarted server or a dropped network connection doesn't disconnect you. The session ends when you disconnect, close its tab, reload or quit, and after **8 hours unused**. While your computer stays awake, StoneQL may keep the password for that long.
- **Reconnecting by itself:** when a session ends after 8 hours unused, or its SSH tunnel closes, StoneQL reconnects by itself if the connection's password is saved, with the same access and on the same database. A read-only query that failed because of it runs once more. Otherwise choose **Connect** again. StoneQL doesn't reconnect by itself:
  - after you disconnect,
  - when a transaction was open (it ended with the session; StoneQL tells you how),
  - for a read-write session on a **Production** connection, where you confirm read-write access again.

  Changes and imports that failed are never run again.
- After restarting StoneQL, connect again; you are asked for the password unless it is saved.

StoneQL stores connection names, groups, colors, tags and other non-secret settings on your computer. Passwords and key passphrases are kept only in the password store described in [Saved passwords](#saved-passwords), and CA certificates are never stored.

## For developers

The [connection management architecture](../architecture/connection-management.md) and [SSH and credentials](../development/ssh-credentials.md) pages describe the internals.
