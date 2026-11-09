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
    (OUT / "Cargo.toml").write_text(f'''[package]
name = "klic-attachments-test"
version = "0.0.0"
edition = "2021"
publish = false

[workspace]

[lib]
path = {json.dumps((ROOT / "tests/klic_attachments.rs").as_posix())}

[dependencies]
roxmltree = {json.dumps(manifest["dependencies"]["roxmltree"])}
zip = {{ version = {json.dumps(zip_dep["version"])}, default-features = false, features = {json.dumps(zip_dep["features"])} }}

[target.'cfg(windows)'.dependencies]
windows-sys = {{ version = "0.61.2", features = ["Win32_System_Com", "Win32_System_Registry", "Win32_Security", "Win32_UI_Shell", "Win32_UI_WindowsAndMessaging"] }}
''', encoding="utf-8")
    shutil.copyfile(ROOT / "Cargo.lock", OUT / "Cargo.lock")
    if os.name == "nt":
        probe = OUT / "document-probe.exe"
        subprocess.run(["rustc", "--edition=2021", str(ROOT / "tests/support/klic_document_probe.rs"),
                        "-o", str(probe)], check=True)
        os.environ["KLIC_DOCUMENT_PROBE"] = str(probe)
    command = ["cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), "--jobs", "1"]
    if os.name == "nt":
        command += ["windows::windows_document_association", "--", "--exact", "--nocapture"]
    else:
        command += sys.argv[1:]
    return subprocess.run(command, cwd=ROOT).returncode


if __name__ == "__main__":
    sys.exit(main())
