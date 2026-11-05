#!/usr/bin/env python3
"""NLCS++ Netbeheer round trip, end to end in a real Open CAD Studio: the
official Enexis example delivery (V11) imported, checked and delivered again
as V12.1; the delivery must give back every object and, when xmlschema and
the network are there, be valid against the Enexis XSD apart from the code
values the example itself has wrong.

    python3 tests/e2e/nlcspp_e2e.py [--ocs PATH] [--port 18824] [--no-install]

The report goes to target/e2e-nlcspp/rapport.md.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-nlcspp"
XML = REPO / "tests" / "fixtures" / "nlcspp" / "enexis-voorbeeld.xml"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)




def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18824)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    cfg = xdg / "OpenCADStudio" / "open-infra"
    cfg.mkdir(parents=True, exist_ok=True)
    (cfg / "settings.json").write_text(json.dumps({"language": "nl"}))
    api_dir = cfg / "api"
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    results, state = [], {}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, detail, time.time() - t))
        print(("✔" if ok else "✘"), name, "—", detail, flush=True)
        return ok

    def cmd(c, rid, timeout=600):
        path = api_dir / f"{rid}.json"
        if path.exists():
            path.unlink()
        ocs.run(f"{c} ID={rid}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if path.exists():
                try:
                    return json.loads(path.read_text())
                except json.JSONDecodeError:
                    pass
            time.sleep(0.5)
        raise TimeoutError(f"{c.split()[0]}: no answer in {timeout} s")

    source = XML.read_text(encoding="utf-8-sig")
    n_source = source.count("<Geometry>")

    def site():
        r = cmd("GIS_SITE RD 170752 369813 700 500 NLCS++ Enexis voorbeeld", "site", 60)
        return r["ok"], r["message"][:100]

    def imp():
        r = cmd(f"GIS_NLCSPP_IMPORT {XML}", "import")
        return r["ok"] and f"{n_source} objecten" in r["message"], r["message"][:160]

    def check():
        r = cmd("GIS_NLCSPP_CONTROLE NETBEHEERDER=Enexis AANNEMER=OpenAEC", "check")
        d = r.get("data") or {}
        # The example's own code-value errors (25 LSmof Functie, 7 LSkabel Kabelopbouw) and rule R.5.
        return r["ok"] and d.get("objecten") == n_source and d.get("fouten", 0) >= 32, r["message"][:200]

    def export():
        path = OUT / "levering.xml"
        r = cmd(f"GIS_NLCSPP_EXPORT NETBEHEERDER=Enexis PROJECTNUMMER=OI-E2E AANNEMER=OpenAEC TEKENINGTYPE=EINDREVISIE PAD={path}", "export")
        if not (r["ok"] and path.exists()):
            return False, r["message"][:200]
        text = path.read_text(encoding="utf-8")
        state["xml"] = path
        n = len(re.findall(r"<Geometry>", text)) - 1  # minus the project reference
        return n == n_source, f"{n}/{n_source} objects delivered → {path.name}"

    def xsd():
        try:
            import xmlschema
        except ImportError:
            return True, "xmlschema not installed: skipped"
        schema = xmlschema.XMLSchema(str(REPO / "tests" / "fixtures" / "nlcspp" / "xsd" / "NLCS_NetbeheerEnexisV12.1Import_Keuzelijst.xsd"))
        errors = list(schema.iter_errors(str(state["xml"])))
        other = [e for e in errors if "value must be one of" not in str(e.reason)]
        return not other, f"{len(errors)} XSD errors, {len(other)} structural" + (f": {other[0].path} {other[0].reason}" if other else "")

    if step("site", site) and step("import", imp):
        step("check", check)
        if step("export", export):
            step("XSD (Enexis V12.1)", xsd)
    ocs.stop()
    lines = ["# NLCS++ round trip e2e", "", "| Step | Result | Detail | s |", "|---|---|---|---|"]
    lines += [f"| {n} | {'✔' if ok else '✘'} | {d} | {t:.1f} |" for n, ok, d, t in results]
    (OUT / "rapport.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if results and all(ok for _, ok, _, _ in results) else 1)


if __name__ == "__main__":
    main()
