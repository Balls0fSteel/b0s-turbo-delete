"""Build td.exe and the Windows installer.

Run from the repository root on Windows. Requires Rust (cargo) and Inno Setup 6
(iscc.exe on PATH, or installed in its default location).

Produces bin\\td.exe and bin\\turbo-delete-setup.exe.
"""

import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DEFAULT_ISCC = Path(r"C:\Program Files (x86)\Inno Setup 6\ISCC.exe")


def find_iscc():
    found = shutil.which("iscc")
    if found:
        return found
    if DEFAULT_ISCC.exists():
        return str(DEFAULT_ISCC)
    raise SystemExit("iscc.exe (Inno Setup 6) not found")


def main():
    subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)

    bin_dir = ROOT / "bin"
    bin_dir.mkdir(exist_ok=True)
    shutil.copyfile(ROOT / "target" / "release" / "td.exe", bin_dir / "td.exe")

    subprocess.run([find_iscc(), str(ROOT / "dist" / "turbo-delete.iss")], cwd=ROOT, check=True)
    print(f"Built {bin_dir / 'turbo-delete-setup.exe'}")


if __name__ == "__main__":
    main()
