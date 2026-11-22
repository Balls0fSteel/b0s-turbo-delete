<p align="center">
  <img src="assets/turbo-delete-icon-transparent-bg.png" alt="Turbo Delete" />
</p>

<h1 align="center">Turbo Delete</h1>
<p align="center">Fast folder deletion for Windows, from the right-click menu or the command line.</p>
<p align="center">
  <a href="https://github.com/sponsors/Balls0fSteel">
    <img src="https://img.shields.io/badge/Sponsor-Balls0fSteel-ea4aaa?logo=githubsponsors&logoColor=white" alt="Sponsor Balls0fSteel on GitHub" />
  </a>
</p>

Turbo Delete removes large folders (`node_modules`, build output) much faster
than Explorer by deleting in parallel on all CPU cores.

## Install

Run in PowerShell:

```powershell
irm https://raw.githubusercontent.com/Balls0fSteel/b0s-turbo-delete/master/dist/install.ps1 | iex
```

Or download from the [latest release](https://github.com/Balls0fSteel/b0s-turbo-delete/releases/latest):

- `turbo-delete-setup.exe` installs for the current user (no admin rights),
  adds `td` to your `PATH` and adds **Turbo Delete** to the folder right-click
  menu. Uninstall from *Settings → Apps*.
- `td.exe` is the standalone tool, with no installation.

## Usage

```text
td <path> [-y|--yes]

  -y, --yes      Delete without asking for confirmation
  -h, --help     Print help
  -V, --version  Print the version
```

`td` shows the full path and asks `Delete "<path>"? [y/N]` before deleting.
`--yes` skips the question; the right-click menu entry uses it, so deletion
starts as soon as you choose it.

**Deleted files do not go to the Recycle Bin.**

When done, it prints a summary such as
`Deleted 41236 files, 5120 folders in 1.84s`. Items that could not be deleted
(for example a file another program has open) are listed, the rest is still
deleted, and the exit code is 1.

## Safety

- **Links are never followed.** Symbolic links and junctions are removed as
  links; what they point to is not touched.
- **Protected folders.** `td` refuses to delete a drive root, your home folder,
  the current folder, or any folder containing one of those.
- **Read-only items** inside the target are deleted. Nothing outside the
  target is changed.
- Without `--yes`, `td` refuses to run when its input is not a terminal (for
  example in a script) instead of waiting for an answer.

## Build from source

Needs [Rust](https://rustup.rs) 1.85 or newer.

```sh
cargo build --release   # target/release/td.exe
cargo test
```

The installer also needs [Inno Setup 6](https://jrsoftware.org/isinfo.php):

```sh
python automate-release.py   # bin\td.exe and bin\turbo-delete-setup.exe
```

To publish a release, run the **Release** workflow on GitHub Actions (or push a
`v*` tag). The version must match in `Cargo.toml` and `dist/turbo-delete.iss`.

## Support

If Turbo Delete saves you time, you can
[sponsor the project on GitHub](https://github.com/sponsors/Balls0fSteel).

## Credits and license

Maintained by [Balls0fSteel](https://github.com/Balls0fSteel). Originally
created by [Tejas Ravishankar](https://github.com/suptejas).

Licensed under the [Apache License 2.0](LICENSE.txt).
