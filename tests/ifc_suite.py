"""Valideer vaste exports lokaal: IFC4X3_ADD2, EXPRESS, geometrie en eigen IDS-eisen."""

import argparse
import hashlib
import importlib.metadata
import json
import subprocess
import sys
from pathlib import Path

import ifcopenshell
import xmlschema
from ifctester import ids

DATA = Path(__file__).with_name("data")


def load_ids():
    # Het IDS-schema van IfcTester importeert W3C XML via HTTP. Gebruik de
    # meegeleverde kopie en verbied externe schema's, ook op een koude runner.
    ids.schema = xmlschema.XMLSchema(
        Path(ids.__file__).with_name("ids.xsd"),
        locations={
            "http://www.w3.org/XML/1998/namespace": str(
                Path(xmlschema.__file__).parent / "schemas/XML/xml.xsd"
            )
        },
        allow="local",
    )
    return ids.open(str(DATA / "ifc43-export.ids"), validate=True)


def check_ids(model, contract):
    contract.validate(model, should_filter_version=True)
    return [s.name for s in contract.specifications if not s.status]


def check_case(path, expected, contract):
    result = {"bestand": path.name, "fouten": []}
    if not path.is_file():
        result["fouten"].append("Exportbestand ontbreekt")
        return result
    result["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    command = [sys.executable, str(Path(__file__).with_name("ifc_validate.py")), str(path)]
    validation = subprocess.run(command, capture_output=True, text=True, timeout=180)
    result["schema_exitcode"] = validation.returncode
    result["schema_uitvoer"] = validation.stdout + validation.stderr
    print(result["schema_uitvoer"], end="", flush=True)
    if validation.returncode:
        result["fouten"].append("Schema-, EXPRESS- of geometriecontrole mislukt")
        return result
    model = ifcopenshell.open(str(path))
    result["instanties"] = len(list(model))
    result["producten"] = len(model.by_type("IfcProduct"))
    result["ids_fouten"] = check_ids(model, contract)
    result["fouten"].extend(f"IDS: {name}" for name in result["ids_fouten"])
    for entity, count in expected["klassen"].items():
        actual = len(model.by_type(entity))
        if actual != count:
            result["fouten"].append(f"{entity}: verwacht {count}, gevonden {actual}")
    if result["producten"] < expected["min_producten"]:
        result["fouten"].append("Te weinig producten; mogelijke lege of onvolledige export")
    conversions = model.by_type("IfcMapConversion")
    actual_crs = [c.TargetCRS.Name for c in conversions]
    if actual_crs != expected["crs"]:
        result["fouten"].append(f"CRS: verwacht {expected['crs']}, gevonden {actual_crs}")
    if "lijnpunten" in expected:
        points = [[list(p) for p in c.Points.CoordList] for c in model.by_type("IfcIndexedPolyCurve")]
        if points != expected["lijnpunten"]:
            result["fouten"].append(f"Lijnpunten in meters: verwacht {expected['lijnpunten']}, gevonden {points}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    cases = json.loads((DATA / "ifc43-cases.json").read_text(encoding="utf-8"))
    contract = load_ids()
    results = []
    for name, expected in cases.items():
        try:
            result = check_case(args.directory / name, expected, contract)
        except Exception as error:
            result = {"bestand": name, "fouten": [f"Controle afgebroken: {error}"]}
        results.append(result)
        print(f"{name}: {len(result['fouten'])} fout(en), inclusief IDS", flush=True)
        for error in result["fouten"]:
            print(f"  {error}", flush=True)
    report = {
        "ifcopenshell": ifcopenshell.version,
        "ifctester": importlib.metadata.version("ifctester"),
        "ids_sha256": hashlib.sha256((DATA / "ifc43-export.ids").read_bytes()).hexdigest(),
        "modellen": results,
    }
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return 1 if any(r["fouten"] for r in results) else 0


if __name__ == "__main__":
    sys.exit(main())
