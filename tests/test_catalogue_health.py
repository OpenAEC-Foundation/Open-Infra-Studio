"""Vaste controles; geen openbare diensten of internet nodig."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import check_map_services as health

FIXTURES = ROOT / "tests/fixtures/catalogue-health"


def service(kind="wms", **values):
    return dict(id="test", name="Proefdienst", kind=kind,
                url="https://example.invalid/ows", default_layer="sample",
                country="NL", bbox=[4, 51, 6, 53], **values)


def reply(filename="wms.xml", **values):
    return health.Response(body=(FIXTURES / filename).read_bytes(),
                           status=values.pop("status", 200),
                           elapsed=values.pop("elapsed", 0.1), **values)


class CatalogueHealthTest(unittest.TestCase):
    def test_capabilities_and_default_layer_for_each_protocol(self):
        for kind in ("wms", "wfs", "wmts"):
            with self.subTest(kind=kind):
                self.assertIn("sample", health.capability_layers(reply(kind + ".xml").body, kind))
        legacy = b'<!DOCTYPE WMT_MS_Capabilities SYSTEM "https://example.invalid/no-network.dtd"><WMT_MS_Capabilities><Capability><Layer><Name>sample</Name></Layer></Capability></WMT_MS_Capabilities>'
        self.assertEqual(health.capability_layers(legacy, "wms"), {"sample"})

    def test_http_200_is_not_sufficient(self):
        for body in (b"<html>Login</html>", b"", b"<broken", b"<WMS_Capabilities/>",
                     reply("wfs.xml").body):
            with self.subTest(body=body):
                with self.assertRaises(health.ServiceError):
                    health.capability_layers(body, "wms")
        with self.assertRaisesRegex(health.ServiceError, "LayerNotDefined"):
            health.capability_layers(reply("exception.xml").body, "wms")

    def test_existing_query_parameters_survive_without_duplicate_ogc_parameters(self):
        s = service()
        s["url"] += "?map=province.map&service=WFS&Request=GetMap&VERSION=9&tokenless=yes"
        mode, url = health.request_for(s)
        from urllib.parse import parse_qs, urlsplit
        q = parse_qs(urlsplit(url).query)
        self.assertEqual(mode, "capabilities")
        self.assertEqual(q, {"map": ["province.map"], "tokenless": ["yes"],
                             "SERVICE": ["WMS"], "REQUEST": ["GetCapabilities"]})

    def test_wmts_tile_template_keeps_axis_order_and_uses_coverage(self):
        s = service("xyz")
        s["url"] = "https://example.invalid/wmts/{z}/{y}/{x}.png"
        mode, url = health.request_for(s)
        self.assertEqual(mode, "tile")
        self.assertEqual(url, "https://example.invalid/wmts/14/5411/8419.png")
        self.assertNotIn("{", url)

    def test_tile_must_have_image_content(self):
        self.assertEqual(health.validate_tile(reply("tile.png").body), "PNG")
        with self.assertRaises(health.ServiceError):
            health.validate_tile(b"<html>Not a tile</html>")
        with self.assertRaises(health.ServiceError):
            health.validate_tile(b"\x89PNG\r\n\x1a\n")

    def test_counts_default_layer_failures_slow_and_transport_failures(self):
        services = [service(), service(), service(), service(), service()]
        for n, s in enumerate(services):
            s["id"] = str(n)
            s["url"] += "/" + str(n)
        services[1]["default_layer"] = "removed"
        responses = [reply(), reply(), reply(elapsed=3),
                     reply(status=503), health.Response(error="time-out", elapsed=8)]
        def fetch(url, timeout):
            return responses[int(url.split("?")[0].rsplit("/", 1)[1])]
        report = health.check_services(services, fetch=fetch, workers=1)
        self.assertEqual(report["counts"], {"werkt": 1, "traag": 1, "kapot": 3})
        self.assertEqual([r["status"] for r in report["services"]],
                         ["werkt", "kapot", "traag", "kapot", "kapot"])
        self.assertIn("removed", report["services"][1]["reason"])
        self.assertIn("503", report["services"][3]["reason"])

    def test_shared_endpoint_is_fetched_once_but_layers_are_checked_separately(self):
        a, b = service(), service()
        b.update(id="missing", default_layer="gone")
        urls = []
        def fetch(url, timeout):
            urls.append(url)
            return reply()
        report = health.check_services([a, b], fetch=fetch)
        self.assertEqual(len(urls), 1)
        self.assertEqual(report["requests"], 1)
        self.assertEqual(report["counts"]["kapot"], 1)

    def test_arcgis_uses_native_extent_and_one_tile_after_metadata(self):
        s = service("xyz")
        s["url"] = "https://example.invalid/MapServer"
        urls = []
        def fetch(url, timeout):
            urls.append(url)
            return reply("arcgis.json" if "f=json" in url else "tile.png")
        report = health.check_services([s], fetch=fetch)
        self.assertEqual(urls, [s["url"] + "?f=json", s["url"] + "/tile/2/3/2"])
        self.assertEqual(report["counts"]["werkt"], 1)
        self.assertEqual(report["requests"], 2)
        self.assertEqual(report["services"][0]["elapsed_seconds"], 0.2)

    def test_script_defects_are_not_disguised_as_broken_services(self):
        def broken_fetch(url, timeout):
            raise RuntimeError("fout in controle")
        with self.assertRaisesRegex(RuntimeError, "fout in controle"):
            health.check_services([service()], fetch=broken_fetch)

    def test_reports_are_complete_and_escape_markdown(self):
        s = service()
        s["name"] = "Proef | dienst\nvolgende regel"
        report = health.check_services([s], fetch=lambda url, timeout: reply(status=404))
        with tempfile.TemporaryDirectory() as tmp:
            health.write_reports(report, Path(tmp))
            data = json.loads((Path(tmp) / "rapport.json").read_text())
            md = (Path(tmp) / "rapport.md").read_text()
        self.assertEqual(data["counts"]["kapot"], 1)
        self.assertIn("Proef \\| dienst volgende regel", md)
        self.assertIn("HTTP 404", md)
        self.assertIn("Kapotte diensten", md)

    def test_builtins_are_read_from_current_source_including_historical_caches(self):
        entries = health.load_catalogue(ROOT / "assets/servers/world.json", ROOT / "src/gis/servers.rs")
        ids = {s["id"] for s in entries}
        self.assertEqual(len(entries), 629)
        self.assertEqual(len(ids), 629)
        self.assertIn("pdok-ruimtelijke-plannen-wms", ids)
        self.assertIn("topotijdreis-1900", ids)

    def test_changed_builtin_syntax_fails_visibly_instead_of_skipping_services(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "servers.rs"
            p.write_text((ROOT / "src/gis/servers.rs").read_text().replace(
                'tijdreis(1900)', 'unknown_preset(1900)'))
            with self.assertRaises(ValueError):
                health.load_catalogue(ROOT / "assets/servers/world.json", p)

    def test_invalid_or_empty_catalogue_fails(self):
        for entries in ([], {}, [service(), service()], [{"id": "missing-fields"}]):
            with self.subTest(entries=entries), tempfile.TemporaryDirectory() as tmp:
                p = Path(tmp) / "catalogue.json"
                p.write_text(json.dumps(entries))
                with self.assertRaises(ValueError):
                    health.load_catalogue(p)

    def test_curl_uses_deadlines_and_transport_failure_is_a_result(self):
        completed = subprocess.CompletedProcess([], 28, stdout="000\n0\n", stderr="Timed out")
        with patch.object(health.subprocess, "run", return_value=completed) as run:
            result = health.fetch_url("https://example.invalid", 2)
        command = run.call_args.args[0]
        self.assertEqual(command[:2], ["curl", "-q"])
        self.assertIn("--max-time", command)
        self.assertIn("--max-filesize", command)
        self.assertNotIn("--retry", command)
        self.assertNotIn("--insecure", command)
        self.assertLessEqual(run.call_args.kwargs["timeout"], 4)
        self.assertIn("time-out", result.error)


if __name__ == "__main__":
    unittest.main()
