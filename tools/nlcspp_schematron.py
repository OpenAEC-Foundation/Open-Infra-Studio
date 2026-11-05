#!/usr/bin/env python3
"""The content rules (inhoudelijke validaties) of the network operators on an
NLCS++ V12.1 delivery, with SaxonC-HE from Python (no Java needed):

    pip install saxonche
    python3 tools/nlcspp_schematron.py <levering.xml> [--repo DIR] [--versie v2026.1.0] [--netbeheerder Enexis]

The rules are the Schematron of https://github.com/mijnaansluiting/Inhoudelijke-validaties
(Apache-2.0; cloned into target/ when --repo is not given). They are
transpiled per phase with SchXslt2, as that repository does, cached, and run
on the delivery. Prints the failed rules and writes a Markdown report next to
the file (<levering>_schematron.md). Exit code 1 when a rule with role
"error" (or without a role) fails.
"""

import argparse
import io
import re
import subprocess
import sys
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCHXSLT2 = "1.4.4"
SVRL = "{http://purl.oclc.org/dsdl/svrl}"


def ensure_repo(repo: Path) -> Path:
    if not repo.exists():
        subprocess.run(["git", "clone", "-q", "--depth", "1", "https://github.com/mijnaansluiting/Inhoudelijke-validaties.git", str(repo)], check=True)
    transpile = repo / "transpile.xsl"
    if not transpile.exists():
        url = f"https://codeberg.org/SchXslt/schxslt2/releases/download/v{SCHXSLT2}/schxslt2-{SCHXSLT2}.zip"
        data = urllib.request.urlopen(url, timeout=60).read()
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            transpile.write_bytes(z.read(f"schxslt2-{SCHXSLT2}/transpile.xsl"))
    return repo


def schemas(repo: Path, operator: str):
    """The XSDs some rules read, where the repository's install script puts them."""
    src = ROOT / "tests" / "fixtures" / "nlcspp" / "xsd"
    dst = repo / "schemas" / "12.1"
    dst.mkdir(parents=True, exist_ok=True)
    (dst / "NLCS_Netbeheer.xsd").write_bytes((src / "NLCS_Netbeheer.xsd").read_bytes())
    (dst / "NLCS_Netbeheer_Keuzelijst.xsd").write_bytes((src / f"NLCS_Netbeheer{operator}V12.1Import_Keuzelijst.xsd").read_bytes())


def phases(repo: Path, version: str):
    main = (repo / version / "rules" / "main.sch").read_text(encoding="utf-8")
    found = re.findall(r'<(?:sch:)?phase\s+id="([^"]+)"', main)
    return [p for p in found if p not in ("#ALL", "#DEFAULT")]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("xml", type=Path)
    ap.add_argument("--repo", type=Path, default=ROOT / "target" / "inhoudelijke-validaties")
    ap.add_argument("--versie", default="v2026.1.0")
    ap.add_argument("--netbeheerder", default="Enexis", choices=["Alliander", "Enexis", "Stedin"])
    a = ap.parse_args()
    from saxonche import PySaxonProcessor

    repo = ensure_repo(a.repo.resolve())
    schemas(repo, a.netbeheerder)
    cache = repo / a.versie / "rules" / "transpiled_phases"
    cache.mkdir(parents=True, exist_ok=True)
    results = []
    with PySaxonProcessor(license=False) as proc:
        xslt = proc.new_xslt30_processor()
        wrapper = xslt.compile_stylesheet(stylesheet_file=str(repo / "wrappers" / "transpile_phase_wrapper.xsl"))
        main_sch = str(repo / a.versie / "rules" / "main.sch")
        for phase in phases(repo, a.versie):
            compiled = cache / f"{phase}.xsl"
            if not compiled.exists():
                wrapper.set_parameter("phase", proc.make_string_value(phase))
                wrapper.transform_to_file(source_file=main_sch, output_file=str(compiled))
            rule = xslt.compile_stylesheet(stylesheet_file=str(compiled))
            svrl = rule.transform_to_string(source_file=str(a.xml.resolve()))
            root = ET.fromstring(svrl)
            for f in root.iter():
                if f.tag in (SVRL + "failed-assert", SVRL + "successful-report"):
                    text = " ".join("".join(t.itertext()).split()) if (t := f.find(SVRL + "text")) is not None else ""
                    results.append((phase, f.get("role") or "error", f.get("location") or "", text))
    errors = [r for r in results if r[1].lower() in ("error", "fatal", "fout")]
    lines = [f"# Inhoudelijke validaties ({a.versie}, {a.netbeheerder}): {a.xml.name}", "", f"{len(results)} bevindingen, waarvan {len(errors)} fout.", "", "| Regel | Rol | Plaats | Melding |", "|---|---|---|---|"]
    lines += [f"| {p} | {r} | {loc[-80:]} | {t.replace('|', '/')} |" for p, r, loc, t in results]
    report = a.xml.with_name(a.xml.stem + "_schematron.md")
    report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    by = {}
    for p, r, _, t in results:
        by.setdefault((p, r, t[:100]), 0)
        by[(p, r, t[:100])] += 1
    for (p, r, t), n in sorted(by.items(), key=lambda kv: -kv[1])[:25]:
        print(f"{n:5} × {p} [{r}] {t}")
    print(f"{len(results)} bevindingen, {len(errors)} fout; rapport: {report}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
