
<p align="center">
  <img src="assets/turbo-delete-icon-transparent-bg.png" />
</p>

<h1 align="center">Turbo Delete</h1>
<h4 align="center">A blazing fast alternative to the default Windows delete.</h4>
<br>

![comparison](https://user-images.githubusercontent.com/63039748/154819625-dce22227-0415-411f-8b53-708566ace569.gif)

Turbo Delete deletes large folders (think `node_modules` or build output) much
faster than Explorer, by deleting files in parallel on all CPU cores.

> Turbo Delete integrates with your context menu as well as the command line!

<br>

# :zap: Installation

Run this in PowerShell:

```powershell
irm https://raw.githubusercontent.com/Balls0fSteel/b0s-turbo-delete/master/dist/install.ps1 | iex
```

This downloads `turbo-delete-setup.exe` from the
[latest release](https://github.com/Balls0fSteel/b0s-turbo-delete/releases/latest)
and installs it for the current user (no administrator rights needed). The
installer:

- puts `td.exe` in `%LOCALAPPDATA%\Programs\turbo-delete` and adds that folder
  to your user `PATH` (open a new terminal afterwards),
- adds **Turbo Delete** to the right-click menu of folders in Explorer.

You can also download and run the installer from the releases page yourself.
Uninstall it from *Settings → Apps*; that removes the `PATH` entry and the
menu entry too.

# :rocket: Usage

```text
td <path> [-y|--yes]

  -y, --yes      Delete without asking for confirmation
  -h, --help     Print help
  -V, --version  Print the version
```

`td <path>` deletes the folder (or file) and everything inside it. It shows the
full path and asks `Delete "<path>"? [y/N]` first; answer `y` to go ahead.
`--yes` skips the question. Without `--yes`, `td` refuses to run when its input
is not a terminal (for example in a script), rather than waiting for an answer
that never comes. Put `--` before a path that starts with `-`.

The **Turbo Delete** context-menu entry runs `td --yes "<folder>"`: choosing
the menu entry is the confirmation, and deletion starts immediately.

**Deleted files do not go to the Recycle Bin.** They are gone for good.

When it is done, `td` prints a summary such as
`Deleted 41236 files, 5120 folders in 1.84s`. If some items could not be
deleted (for example a file that another program has open), it keeps going
with everything else, then lists the first few failures and exits with code 1.

### Safety

- **Links are never followed.** Symbolic links and junctions inside the folder
  (pnpm's `node_modules` is full of them) are removed as links; the folders
  they point to are not touched. If `<path>` itself is a link, only the link
  is removed.
- `td` refuses to delete a drive or filesystem root (such as `C:\`), your home
  folder or any folder that contains it, and the current folder or any folder
  that contains it. Paths are compared after resolving `.` and `..`, ignoring
  case on Windows.
- Read-only files and folders inside the target are deleted: on Windows the
  read-only attribute is cleared; on Linux and macOS the owner's permissions on
  the containing folder are fixed as needed. Nothing outside the target is
  changed.

# :hammer: Building from source

Install [Rust](https://rustup.rs) (stable, 1.85 or newer), then:

```sh
cargo build --release      # target/release/td.exe (td on Linux/macOS)
cargo test
```

To build the installer on Windows, also install
[Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:

```sh
python automate-release.py # bin\td.exe and bin\turbo-delete-setup.exe
```

Pushing a `v*` tag (for example `v1.0.0`) builds both files on GitHub Actions
and attaches them to a GitHub release. The tag must match `version` in
`Cargo.toml` and `MyAppVersion` in `dist/turbo-delete.iss`.

<br>

## :clap: Supporters

![Stargazers repo roster for @suptejas/turbo-delete](https://reporoster.com/stars/suptejas/turbo-delete)

<br>

## Authors

[Tejas](https://www.github.com/suptejas)

## Versioning

We use [SemVer](http://semver.org/) for versioning. For the versions available, see the [tags on this repository](https://github.com/Balls0fSteel/b0s-turbo-delete/tags).

## License

This project is licensed under the Apache 2.0 License - see the [LICENSE](LICENSE.txt) file for details.
