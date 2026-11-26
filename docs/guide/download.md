# Download

StoneQL **v0.1.1** is available for macOS, Windows and Linux. Every file in it was built from the same release commit.

## Choose your file

Open the release page for **[StoneQL v0.1.1](https://github.com/stoneAce/stoneql/releases/tag/v0.1.1)**. Later builds appear on the [releases page](https://github.com/stoneAce/stoneql/releases). Under **Assets**, download the file for your computer, plus `SHA256SUMS`. The release notes list each file name with its platform.

| Your computer | File | Tested on |
| --- | --- | --- |
| Mac with Apple Silicon (M-series) | macOS Apple Silicon (arm64) `.dmg` | macOS 27.0.1 |
| Mac with an Intel processor | macOS Intel (x86_64) `.dmg` | Built for Intel; not yet tested on an Intel Mac |
| Windows 10 or 11, 64-bit (x64) | Windows x64 installer (`.exe` inside a `.zip`) | GitHub's Windows Server 2025 machines; not yet tested on a real Windows 10 or 11 PC |
| Ubuntu 22.04 or later, Debian 12 or later, 64-bit (x86_64) | Linux x64 `.deb` | Ubuntu 26.04, plus install and launch on Ubuntu 22.04 and Debian 12 |

Not sure which Mac you have? Open the Apple menu → **About This Mac**: **Chip** with an *Apple M…* name means Apple Silicon; **Processor** with an *Intel* name means Intel.

There are no builds yet for Windows on ARM, for Linux on ARM, or for Linux distributions that do not use `.deb` packages.

### Requirements

- **macOS:** StoneQL was tested on macOS 27.0.1 only. Older versions have not been tested.
- **Windows:** the installer needs Microsoft Edge WebView2. Windows 11 includes it and Windows 10 usually has it; if it is missing, the installer downloads it, so stay online while installing. No administrator rights are needed.
- **Linux:** WebKitGTK 4.1 version 2.38 or later (`libwebkit2gtk-4.1-0`) and glibc 2.34 or later. Ubuntu 22.04 gets that WebKitGTK version from its `jammy-updates` package source, which apt uses when updates are enabled. See [Install on Linux](./install.md#linux).

## Check the download

The checksum proves the file you downloaded is exactly the file that was published. `SHA256SUMS` lists one line per file: its SHA-256 checksum and its name. Keep `SHA256SUMS` in the same folder as the file you downloaded (for example Downloads).

### macOS

1. Open **Terminal** (in Applications → Utilities).
2. Run:

   ```sh
   cd ~/Downloads
   shasum -a 256 -c --ignore-missing SHA256SUMS
   ```

3. It must print your file name followed by `: OK`. `--ignore-missing` skips the files for other platforms that you did not download.

### Linux

In a terminal, run:

```sh
cd ~/Downloads
sha256sum -c --ignore-missing SHA256SUMS
```

It must print your file name followed by `: OK`.

### Windows

Windows has no built-in command that reads `SHA256SUMS`, so compare by hand.

1. Open **PowerShell** (Start menu → type *PowerShell*).
2. Run, with the name of the `.zip` you downloaded:

   ```powershell
   cd $HOME\Downloads
   Get-FileHash .\<installer>.zip
   ```

3. Open `SHA256SUMS` in Notepad and find the line that ends with the `.zip` file's name. The long value under **Hash** must match the checksum at the start of that line, character for character. Upper or lower case does not matter.

If the check fails, **do not open the file**: delete it and download it again.

## About signing

None of the release files is signed with a publisher certificate yet, so each system warns you the first time:

- **macOS:** the app is signed ad hoc and **not notarized by Apple** (it has no Developer ID signature). macOS blocks its first launch until you approve it.
- **Windows:** the installer is **unsigned**. Microsoft Defender SmartScreen shows “Windows protected your PC” until you choose to run it.
- **Linux:** the `.deb` package is not signed; `SHA256SUMS` is the way to check it.

[Install](./install.md) shows how to get past each warning. Only do this for a file whose checksum matched.

Next: [Install StoneQL](./install.md).
