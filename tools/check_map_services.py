#!/usr/bin/env python3
"""Controleer openbare kaartbronnen zonder de catalogus of app te wijzigen."""
import argparse
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
MAX_BYTES = 16 * 1024 * 1024
USER_AGENT = "OpenInfraStudio-CatalogueHealth/1.0 (+https://github.com/OpenAEC-Foundation/Open-Infra-Studio)"


class ServiceError(Exception):
    """Een verwacht probleem bij een openbare dienst of proefaanvraag."""


@dataclass
class Response:
    body: bytes = b""
    status: int = 0
    elapsed: float = 0
    error: str = ""
    requests: int = 1


def builtin_services(path):
    """Lees uitsluitend de ondersteunde declaraties; stop bij gewijzigde syntax.

    Zo is geen Rust-build of tweede, handmatig bijgehouden catalogus nodig.
    Dit is bewust geen algemene Rust-parser.
    """
    source = path.read_text(encoding="utf-8")
    match = re.search(r"pub\(crate\) fn builtin\(\).*?vec!\[(.*?)\n    \]", source, re.S)
    if not match:
        raise ValueError("Ingebouwde catalogus niet herkenbaar in " + str(path))
    declarations = re.sub(r"(?m)^\s*//[^\n]*", "", match[1]).strip()
    string = r'("(?:[^"\\]|\\.)*")'
    preset = re.compile(r"preset\(\s*" + string + r",\s*(Wms|Wfs|Xyz),\s*(nl|vl|world),\s*"
                        + string + r",\s*" + string + r",\s*" + string + r",\s*" + string + r"\s*\),")
    historical = re.compile(r"tijdreis\((\d+)\),")
    entries = []
    while declarations:
        m = preset.match(declarations)
        h = historical.match(declarations)
        if m:
            ident, kind, region, name, url, layer, attribution = m.groups()
            entries.append(dict(id=json.loads(ident), kind=kind.lower(), name=json.loads(name),
                                url=json.loads(url), default_layer=json.loads(layer),
                                country={"nl": "NL", "vl": "BE", "world": "GLOBAL"}[region],
                                bbox={"nl": [3.2, 50.75, 7.23, 53.6],
                                      "vl": [2.54, 50.68, 5.92, 51.51],
                                      "world": [-180, -85, 180, 85]}[region], source="builtin"))
        elif h:
            year = h[1]
            function = source.split("pub fn tijdreis(", 1)[-1]
            template = re.search(r'&format!\(("https://[^"\n]*\{year\}[^"\n]*")\)', function)
            if not template:
                raise ValueError("URL van Topotijdreis niet herkenbaar")
            entries.append(dict(id="topotijdreis-" + year, kind="xyz", name="Topotijdreis " + year,
                                url=json.loads(template[1]).replace("{year}", year), default_layer="",
                                country="NL", bbox=[3.2, 50.75, 7.23, 53.6], source="builtin"))
        else:
            raise ValueError("Onbekende declaratie in ingebouwde catalogus: " + declarations[:80])
        declarations = declarations[(m or h).end():].strip()
    return entries


def load_catalogue(path, builtins_path=None):
    entries = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(entries, list) or not entries:
        raise ValueError("De wereldcatalogus moet een niet-lege lijst zijn")
    entries = [dict(s, source="world") if isinstance(s, dict) else s for s in entries]
    if builtins_path is not None:
        entries += builtin_services(builtins_path)
    seen = set()
    for s in entries:
        if not isinstance(s, dict) or any(not isinstance(s.get(k), str) or not s[k]
                                          for k in ("id", "name", "url", "kind")):
            raise ValueError("Catalogusvermelding mist id, name, url of kind")
        if s["id"] in seen or s["kind"] not in {"wms", "wfs", "wmts", "xyz"}:
            raise ValueError("Dubbel id of onbekend diensttype: " + s["id"])
        seen.add(s["id"])
    return entries


def query_url(url, parameters, remove=()):
    parts = urlsplit(url)
    replace = {k.lower() for k in parameters} | {k.lower() for k in remove}
    query = [(k, v) for k, v in parse_qsl(parts.query, keep_blank_values=True) if k.lower() not in replace]
    return urlunsplit(parts._replace(query=urlencode(query + list(parameters.items())), fragment=""))


def request_for(service):
    url, kind = service["url"], service["kind"]
    if urlsplit(url).scheme not in {"http", "https"} or not urlsplit(url).hostname:
        raise ServiceError("Geen geldige HTTP(S)-URL")
    if kind in {"wms", "wfs", "wmts"}:
        return "capabilities", query_url(url, {"SERVICE": kind.upper(), "REQUEST": "GetCapabilities"},
                                         remove=("version", "acceptversions", "request", "service"))
    if ("/MapServer" in url or "/ImageServer" in url) and "{z}" not in url:
        return "arcgis", query_url(url, {"f": "json"})
    if not all("{" + k + "}" in url for k in ("z", "x", "y")):
        raise ServiceError("Tegeltemplate mist {z}, {x} of {y}")
    bbox = service.get("bbox")
    if not isinstance(bbox, list) or len(bbox) != 4 or not all(isinstance(v, (int, float)) and math.isfinite(v) for v in bbox):
        raise ServiceError("Geen geldige WGS84-dekking voor een proeftegel")
    west, south, east, north = bbox
    if not (-180 <= west <= east <= 180 and -90 <= south <= north <= 90):
        raise ServiceError("Ongeldige WGS84-dekking voor een proeftegel")
    z = min(14, service.get("max_zoom") or 14)
    lon, lat = (west + east) / 2, max(-85.0511, min(85.0511, (south + north) / 2))
    n = 2 ** z
    x = max(0, min(n - 1, int((lon + 180) / 360 * n)))
    y = max(0, min(n - 1, int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)))
    for key, value in {"z": z, "x": x, "y": y}.items():
        url = url.replace("{" + key + "}", str(value))
    if "{" in url or "}" in url:
        raise ServiceError("Onbekende variabele in tegeltemplate")
    return "tile", url


def fetch_url(url, timeout):
    """curl begrenst ook DNS, redirects en het volledig lezen van het antwoord."""
    start = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="infra-health-") as tmp:
        body = Path(tmp) / "body"
        command = ["curl", "-q", "--silent", "--show-error", "--location", "--max-redirs", "2",
                   "--proto", "=http,https", "--proto-redir", "=http,https",
                   "--connect-timeout", str(min(3, timeout)), "--max-time", str(timeout),
                   "--max-filesize", str(MAX_BYTES), "--user-agent", USER_AGENT,
                   "--output", str(body), "--write-out", "%{http_code}\n%{num_redirects}\n", "--url", url]
        try:
            run = subprocess.run(command, capture_output=True, text=True, timeout=timeout + 2)
        except subprocess.TimeoutExpired:
            return Response(elapsed=time.monotonic() - start, error="time-out (proceslimiet)")
        # Lokale configuratie-, schrijf- en uitvoeringsfouten moeten de job laten falen.
        if run.returncode in {2, 4, 23, 26, 27, 43, 48, 77}:
            raise RuntimeError("Lokale curl-fout: " + run.stderr.strip())
        code, redirects = run.stdout.strip().splitlines()
        error = ""
        if run.returncode:
            error = {5: "DNS-fout bij proxy", 6: "DNS-fout", 7: "Verbinding mislukt",
                     28: "time-out", 35: "TLS-verbinding mislukt", 47: "Te veel redirects",
                     60: "TLS-certificaat ongeldig", 63: "Antwoord groter dan 16 MiB"}.get(
                         run.returncode, "Netwerkfout (curl " + str(run.returncode) + ")")
        return Response(body=body.read_bytes() if body.exists() and not error else b"",
                        status=int(code), elapsed=time.monotonic() - start, error=error,
                        requests=1 + int(redirects))


def local_name(tag):
    return tag.rsplit("}", 1)[-1]


def capability_layers(body, kind):
    try:
        root = ET.fromstring(body)
    except ET.ParseError as e:
        raise ServiceError("Geen geldige capabilities-XML: " + str(e)) from e
    if local_name(root.tag) in {"ExceptionReport", "ServiceExceptionReport"}:
        codes = [n.get("exceptionCode", n.get("code", "")) for n in root.iter()]
        raise ServiceError("OGC-fout: " + ", ".join(c for c in codes if c))
    expected = {"wms": {"WMS_Capabilities", "WMT_MS_Capabilities"},
                "wfs": {"WFS_Capabilities"}, "wmts": {"Capabilities"}}[kind]
    if local_name(root.tag) not in expected or (kind == "wmts" and not root.tag.startswith("{http://www.opengis.net/wmts/1.0}")):
        raise ServiceError("Onverwacht document voor " + kind.upper() + ": " + local_name(root.tag))
    container, field = {"wms": ("Layer", "Name"), "wfs": ("FeatureType", "Name"),
                        "wmts": ("Layer", "Identifier")}[kind]
    layers = {n.text.strip() for layer in root.iter() if local_name(layer.tag) == container
              for n in layer if local_name(n.tag) == field and n.text and n.text.strip()}
    if not layers:
        raise ServiceError("Capabilities bevat geen benoemde lagen of objecttypen")
    return layers


def validate_tile(body):
    # Controleer de inhoud, niet alleen HTTP-status of het MIME-type.
    if len(body) >= 45 and body.startswith(b"\x89PNG\r\n\x1a\n") and body[12:16] == b"IHDR" and body[-8:-4] == b"IEND":
        return "PNG"
    if len(body) >= 32 and body.startswith(b"\xff\xd8\xff") and body.rstrip().endswith(b"\xff\xd9"):
        return "JPEG"
    if len(body) >= 20 and body.startswith(b"RIFF") and body[8:12] == b"WEBP":
        return "WebP"
    raise ServiceError("Geen herkenbare PNG/JPEG/WebP-proeftegel (mogelijk foutpagina of onvolledig beeld)")


def arcgis_tile_url(base, body):
    try:
        data = json.loads(body)
        tile, extent = data["tileInfo"], data["fullExtent"]
        level = min(tile["lods"], key=lambda lod: abs(float(lod["resolution"]) - 10))
        resolution = float(level["resolution"])
        x, y = (float(extent["xmin"]) + float(extent["xmax"])) / 2, (float(extent["ymin"]) + float(extent["ymax"])) / 2
        col = math.floor((x - tile["origin"]["x"]) / (tile["cols"] * resolution))
        row = math.floor((tile["origin"]["y"] - y) / (tile["rows"] * resolution))
        return base.split("?")[0].rstrip("/") + f"/tile/{int(level['level'])}/{row}/{col}"
    except (ValueError, KeyError, TypeError, ZeroDivisionError, OverflowError) as e:
        raise ServiceError("ArcGIS-antwoord bevat geen bruikbare tegelcache en dekking") from e


def require_success(response):
    if response.error:
        raise ServiceError(response.error)
    if not 200 <= response.status < 300:
        raise ServiceError("HTTP " + str(response.status))


def check_services(services, *, fetch=None, timeout=8, slow=3, workers=6):
    fetch = fetch or fetch_url
    started = datetime.now(timezone.utc).isoformat()
    groups, results = defaultdict(list), {}
    for i, s in enumerate(services):
        try:
            mode, url = request_for(s)
            groups[(s["kind"], mode, url)].append((i, s))
        except ServiceError as e:
            results[i] = dict(s, status="kapot", reason=str(e), elapsed_seconds=0,
                              http_status=0, probe="catalogus", request_url="", requests=0)
    locks = {urlsplit(url).hostname: threading.Lock() for _, _, url in groups}

    def check_group(key, entries):
        kind, mode, url = key
        with locks[urlsplit(url).hostname]:
            response = fetch(url, timeout)
            elapsed, requests, last_url = response.elapsed, response.requests, url
            reason, layers = "", None
            try:
                require_success(response)
                if mode == "capabilities":
                    layers = capability_layers(response.body, kind)
                    reason = "GetCapabilities geldig; " + str(len(layers)) + " lagen/objecttypen"
                else:
                    if mode == "arcgis":
                        last_url = arcgis_tile_url(url, response.body)
                        response = fetch(last_url, timeout)
                        elapsed += response.elapsed
                        requests += response.requests
                        require_success(response)
                    reason = "Proeftegel ontvangen (" + validate_tile(response.body) + ")"
            except ServiceError as e:
                reason = str(e)
                failed = True
            else:
                failed = False
            output = []
            for i, s in entries:
                missing = [] if layers is None else [n.strip() for n in s.get("default_layer", "").split(",") if n.strip() and n.strip() not in layers]
                error = failed or bool(missing)
                detail = "Standaardlaag ontbreekt: " + ", ".join(missing) if missing and not failed else reason
                output.append((i, dict(s, status="kapot" if error else "traag" if elapsed >= slow else "werkt",
                                       reason=detail, elapsed_seconds=round(elapsed, 3), http_status=response.status,
                                       probe=mode, request_url=last_url, requests=requests)))
            return output, requests

    request_count = 0
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(check_group, key, entries) for key, entries in groups.items()]
        for future in as_completed(futures):
            output, requests = future.result()  # Programmeerfouten blijven fouten.
            results.update(output)
            request_count += requests
    ordered = [results[i] for i in range(len(services))]
    return dict(started_at=started, completed_at=datetime.now(timezone.utc).isoformat(),
                timeout_seconds=timeout, slow_seconds=slow, workers=workers, requests=request_count,
                counts={status: sum(r["status"] == status for r in ordered) for status in ("werkt", "traag", "kapot")},
                services=ordered)


def write_reports(report, directory):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "rapport.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    def cell(value):
        return " ".join(str(value).split()).replace("\\", "\\\\").replace("|", "\\|").replace("<", "&lt;").replace(">", "&gt;")
    counts = report["counts"]
    lines = ["# Gezondheid van de kaartcatalogus", "", "Gemeten vanaf: " + report["started_at"],
             "Afgerond: " + report["completed_at"], "",
             f"Totaal {len(report['services'])}: **{counts['werkt']} werkt**, **{counts['traag']} traag**, **{counts['kapot']} kapot**.",
             f"{report['requests']} HTTP-verzoeken inclusief redirects; time-out {report['timeout_seconds']} s per verzoek; traag vanaf {report['slow_seconds']} s.", "",
             "WMS/WFS/WMTS: geldige GetCapabilities en de ingestelde standaardlaag. XYZ/WMTS-templates: één proeftegel bij het midden van de dekking, zoom 14 (of max_zoom). ArcGIS: cachemetadata en één tegel in de eigen coördinaten.", "",
             "Dit is een momentopname vanaf deze meetomgeving. ‘Kapot’ betekent dat deze proef mislukte: een time-out, toegangsblokkade of ontbrekende proeftegel bewijst niet dat de volledige dienst overal uitgevallen is. ‘Werkt’ en ‘traag’ bewijzen geen geslaagde kaart- of objectimport in de app. Tegelinhoud wordt op bestandskenmerken gecontroleerd, niet volledig gedecodeerd.", ""]
    if report.get("sources"):
        lines += ["## Bronnen", "", "| Bestand | SHA-256 |", "|---|---|"]
        lines += [f"| {cell(name)} | {digest} |" for name, digest in report["sources"].items()]
        lines += [""]
    for status, title in (("kapot", "Kapotte diensten"), ("traag", "Trage diensten"), ("werkt", "Werkende diensten")):
        lines += ["## " + title, "", "| ID | Dienst | Land | Type | Seconden | Reden |", "|---|---|---|---|---:|---|"]
        for s in report["services"]:
            if s["status"] == status:
                lines.append("| " + " | ".join(cell(s.get(k, "")) for k in ("id", "name", "country", "kind", "elapsed_seconds", "reason")) + " |")
        lines += [""]
    (directory / "rapport.md").write_text("\n".join(lines), encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalogue", type=Path, default=ROOT / "assets/servers/world.json")
    parser.add_argument("--builtins-source", type=Path, default=ROOT / "src/gis/servers.rs")
    parser.add_argument("--no-builtins", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / "target/catalogue-health")
    parser.add_argument("--timeout", type=float, default=8)
    parser.add_argument("--slow", type=float, default=3)
    parser.add_argument("--workers", type=int, default=6)
    args = parser.parse_args(argv)
    if not (math.isfinite(args.timeout) and math.isfinite(args.slow) and 0 < args.slow < args.timeout <= 60 and 1 <= args.workers <= 12):
        parser.error("Vereist: 0 < slow < timeout <= 60, en 1 <= workers <= 12")
    if not shutil.which("curl"):
        parser.error("curl is vereist")
    try:
        services = load_catalogue(args.catalogue, None if args.no_builtins else args.builtins_source)
        paths = [Path(__file__).resolve(), args.catalogue] + ([] if args.no_builtins else [args.builtins_source])
        sources = {str(p.relative_to(ROOT) if p.is_relative_to(ROOT) else p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
        print(f"Controle van {len(services)} vermeldingen; maximaal {args.workers} verzoeken tegelijk, één per host.", flush=True)
        report = check_services(services, timeout=args.timeout, slow=args.slow, workers=args.workers)
        report["sources"] = sources
        write_reports(report, args.output)
    except (OSError, ValueError, RuntimeError) as e:
        print("Controle afgebroken: " + str(e), file=sys.stderr)
        return 1
    print(json.dumps(report["counts"], ensure_ascii=False), flush=True)
    print("Rapport: " + str(args.output / "rapport.md"), flush=True)
    return 0  # Uitgevallen diensten zijn rapportgegevens, geen scriptfout.


if __name__ == "__main__":
    sys.exit(main())
