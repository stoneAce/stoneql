# Install

Before you start, [download the file for your computer and check its checksum](./download.md). Jump to your system: [macOS](#macos), [Windows](#windows) or [Linux](#linux).

## macOS

The Apple Silicon and Intel DMGs install the same way.

### Install from the DMG

1. Double-click the `.dmg` file you downloaded. A window opens with **StoneQL** on the left and an **Applications** shortcut on the right.
2. Drag **StoneQL** onto **Applications**.
3. Close the window and eject the **StoneQL** disk (in Finder's sidebar, or drag it to the Trash).
4. Open **StoneQL** from your **Applications** folder, not from the DMG.

### Approve the first launch

Because StoneQL is not notarized, macOS blocks the first launch of a downloaded copy. You see a dialog titled **“StoneQL” Not Opened** (“Apple could not verify “StoneQL” is free of malware…”) with only two buttons, **Move to Trash** and **Done**. There is no Open button.

1. Choose **Done** (not Move to Trash).
2. Open **System Settings → Privacy & Security** and scroll down to the message about StoneQL.
3. Choose **Open Anyway**.
4. Confirm. macOS may ask for your login password.

From then on, open StoneQL from Applications as usual. Only approve a copy whose checksum matched.

StoneQL opens on a **Welcome to StoneQL** screen. Continue with [Connect to a database](./connect.md).

### Update to a newer build

1. Download the newer DMG and [check its checksum](./download.md#check-the-download).
2. Quit StoneQL.
3. Open the new DMG and drag **StoneQL** onto **Applications**, replacing the old copy.
4. Open StoneQL from Applications. A newly downloaded build may need the [first-launch approval](#approve-the-first-launch) again.

Your saved connections, groups, theme, grid preferences and saved passwords are kept. After an update, macOS asks once more before StoneQL can read a saved password; see [Keychain prompts](./troubleshooting.md#keychain-prompts).

### Uninstall from macOS

1. **Optional, to delete saved passwords:** in StoneQL, remove each connection that has a saved password (**Manage connections**, then remove the connection). Removing a connection deletes its saved password from the Keychain.
2. Quit StoneQL.
3. Move **StoneQL** from Applications to the Trash.

Moving the app to the Trash leaves your data behind, and a reinstalled StoneQL uses it again:

- Saved connections and preferences: `~/Library/WebKit/me.thixpin.stoneql` and `~/Library/Caches/me.thixpin.stoneql`.
- Trusted SSH host keys and the list of unknown outcomes: `~/Library/Application Support/me.thixpin.stoneql`.
- Saved passwords in your login Keychain. If you did not remove the connections first, open **Keychain Access** and delete the items named `me.thixpin.stoneql.credentials`.

To remove everything, delete those folders and Keychain items too.

## Windows

The installer is for 64-bit (x64) Windows. It installs StoneQL for your Windows account only, into `%LOCALAPPDATA%\StoneQL`, and needs no administrator rights.

::: warning Tested on Windows Server, not yet on a Windows PC
The installer and the app were tested on GitHub's Windows Server 2025 machines, including a silent install and launch. They have not yet been tested on a real Windows 10 or 11 PC. Please [report](./beta.md#how-to-report-a-problem) what you find.
:::

### Run the installer

1. Right-click the `.zip` you downloaded and choose **Extract All…**, then double-click the `.exe` installer inside it.
2. Windows shows a blue **Windows protected your PC** window, because the installer is not signed. If the [checksum matched](./download.md#check-the-download), choose **More info**, then **Run anyway**.
3. Follow the installer's steps. If Microsoft Edge WebView2 is missing, the installer downloads and installs it; this needs an internet connection.
4. Open **StoneQL** from the Start menu.

StoneQL opens on a **Welcome to StoneQL** screen. Continue with [Connect to a database](./connect.md).

### Update on Windows

Not yet tried on a real PC: updating from an earlier version to 0.1.2.

1. Download the newer `.zip`, [check its checksum](./download.md#check-the-download) and extract the installer.
2. Quit StoneQL.
3. Run the new installer, choosing **More info → Run anyway** again if SmartScreen asks.

### Uninstall from Windows

1. **Optional, to delete saved passwords:** in StoneQL, remove each connection that has a saved password (**Manage connections**, then remove the connection).
2. Quit StoneQL.
3. Open **Settings → Apps** (on Windows 11, **Settings → Apps → Installed apps**), find **StoneQL** and choose **Uninstall**.

Data that may be left behind, and that a reinstalled StoneQL uses again:

- Saved passwords: `%APPDATA%\me.thixpin.stoneql.credentials\`, one encrypted file per password.
- Trusted SSH host keys and the list of unknown outcomes: `%APPDATA%\me.thixpin.stoneql`.

To remove saved passwords you did not delete in StoneQL, delete the `me.thixpin.stoneql.credentials` folder.

## Linux

The `.deb` package is for 64-bit (x86_64) Ubuntu 22.04 or later and Debian 12 or later.

::: warning Tested on a server and in containers, not yet on a desktop
StoneQL was tested on Ubuntu 26.04 in a virtual display, and the package was installed and launched on Ubuntu 22.04 and Debian 12. Still to be checked on a real desktop: the GNOME Keyring unlock prompt, the master-password dialog in the app window, and Wayland sessions. Please [report](./beta.md#how-to-report-a-problem) what you find.
:::

### Install the .deb package

Install with `apt`, so that the packages StoneQL needs are installed too. In a terminal, in the folder with the download:

```sh
sudo apt update
sudo apt install ./<file>.deb
```

Type `./` before the file name; without it, apt looks for a package of that name online instead of the file.

StoneQL needs WebKitGTK 4.1 version 2.38 or later. Debian 12 has it. On Ubuntu 22.04 it comes from the `jammy-updates` package source, which apt uses when updates are enabled (the default). If apt reports unmet dependencies, see [apt cannot install the package](./troubleshooting.md#linux-apt-cannot-install-the-package).

Open **StoneQL** from your applications menu. It opens on a **Welcome to StoneQL** screen. Continue with [Connect to a database](./connect.md).

If no desktop keyring is running, StoneQL asks you to choose a master password the first time you save a password; see [Saved passwords](./connect.md#saved-passwords).

### Update on Linux

Not yet tried on a real desktop: updating from an earlier version to 0.1.2.

1. Download the newer `.deb` and [check its checksum](./download.md#check-the-download).
2. Quit StoneQL.
3. Install it the same way: `sudo apt install ./<file>.deb`.

### Uninstall from Linux

1. **Optional, to delete saved passwords:** in StoneQL, remove each connection that has a saved password (**Manage connections**, then remove the connection).
2. Quit StoneQL.
3. Remove the package. Its name is `stone-ql`:

   ```sh
   sudo apt remove stone-ql
   ```

Data left behind in your home folder, which a reinstalled StoneQL uses again:

- Trusted SSH host keys and the list of unknown outcomes: `~/.local/share/me.thixpin.stoneql`.
- StoneQL's encrypted password file, if you used one: `~/.local/share/me.thixpin.stoneql/credentials`.
- Passwords saved in the desktop keyring: items labelled **StoneQL saved connection credential** (for example in GNOME's **Passwords and Keys** app).

To remove everything, delete that folder and those keyring items too.
