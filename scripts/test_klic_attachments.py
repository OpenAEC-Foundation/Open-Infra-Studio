#!/usr/bin/env python3
"""Test de echte bijlagecode zonder de CAD-interface te compileren."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "klic-attachments"


def main():
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    OUT.mkdir(parents=True, exist_ok=True)
    # Gebruik dezelfde versies en ZIP-features als de productiecode. De kopie
    # van Cargo.lock houdt ook de transitieve dependencies op dezelfde versies.
    zip_dep = manifest["dependencies"]["zip"]
    windows_dep = manifest["target"]["cfg(windows)"]["dependencies"]["windows-sys"]
    windows_features = windows_dep["features"] + ["Win32_Security"]
    (OUT / "Cargo.toml").write_text(f'''[package]
name = "klic-attachments-test"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[lib]
path = {json.dumps((ROOT / "tests/support/klic_attachments.rs").as_posix())}

[dependencies]
roxmltree = {json.dumps(manifest["dependencies"]["roxmltree"])}
zip = {{ version = {json.dumps(zip_dep["version"])}, default-features = false, features = {json.dumps(zip_dep["features"])} }}

[target.'cfg(windows)'.dependencies]
windows-sys = {{ version = {json.dumps(windows_dep["version"])}, features = {json.dumps(windows_features)} }}
''', encoding="utf-8")
    shutil.copyfile(ROOT / "Cargo.lock", OUT / "Cargo.lock")
    if os.name == "nt":
        probe = OUT / "document-probe.exe"
        subprocess.run(["rustc", "--edition=2021", str(ROOT / "tests/support/klic_document_probe.rs"),
                        "-o", str(probe)], check=True)
        os.environ["KLIC_DOCUMENT_PROBE"] = str(probe)
    command = ["cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), "--lib", "--jobs", "1"]
    if os.name == "nt":
        command += ["windows::windows_document_association", "--", "--exact", "--nocapture"]
    else:
        command += sys.argv[1:] or ["klic_extra::tests::"]
    return subprocess.run(command, cwd=ROOT).returncode


if __name__ == "__main__":
    sys.exit(main())
