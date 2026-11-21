# Install

Download the archive for your host, verify it against `SHA256SUMS`, and run
`dosker doctor`. Release artifacts are unsigned by default. A missing signing
certificate does not change the archive contents.

## Linux x86_64

1. Download `dosker-<version>-linux-x86_64.tar.gz` and `SHA256SUMS`.
2. Run `sha256sum -c SHA256SUMS` from the download directory.
3. Extract the archive with `tar -xzf dosker-<version>-linux-x86_64.tar.gz`.
4. Put `dosker` on `PATH`.
5. Install `qemu-system-x86` from the distribution package manager.
6. Run `dosker doctor`.

## macOS arm64

1. Download `dosker-<version>-macos-arm64.tar.gz` and `SHA256SUMS`.
2. Run `shasum -a 256 -c SHA256SUMS` from the download directory.
3. Extract the archive with `tar -xzf dosker-<version>-macos-arm64.tar.gz`.
4. Put `dosker` on `PATH`.
5. Install QEMU with `brew install qemu`.
6. Install Bochs with `brew install bochs`.
7. Install DOSBox-X from the official arm64 release or Homebrew.
8. Run `dosker doctor`.

An unsigned build may be blocked by Gatekeeper. To allow it, open the file
context menu and select **Open**, or remove the quarantine attribute with
`xattr -d com.apple.quarantine dosker`.

## Windows x86_64

1. Download `dosker-<version>-windows-x86_64.zip` and `SHA256SUMS`.
2. Run `Get-FileHash -Algorithm SHA256` on the archive and compare the printed hash with the line for the archive in `SHA256SUMS`.
3. Extract the archive.
4. Add the directory that holds `dosker.exe` to `PATH`.
5. Install the official QEMU build.
6. Set `DOSKER_QEMU` when `qemu-system-i386` is not on `PATH`.
7. Run `dosker doctor`.

## Build from source

```sh
cargo build --release
cargo install --path crates/dosker-cli
```

## Verify the download

`SHA256SUMS` lists bare archive filenames and covers every artifact. Run the
check from the directory that holds the downloaded archives. The three
verification commands above are the only accepted forms.

## Engines

DOSker does not bundle engines. The pinned engine versions and checksum
variables are in [Engines](engines.md).

## Security

DOSker does not sandbox untrusted images. QEMU TCG is not a security boundary.
Use an outer virtual machine or an isolated CI runner for untrusted input.

## License

DOSker is dual-licensed under the MIT License or the Apache License 2.0. See
[LICENSE-MIT](../LICENSE-MIT) and [LICENSE-APACHE](../LICENSE-APACHE).
