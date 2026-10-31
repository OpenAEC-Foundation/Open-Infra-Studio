#!/usr/bin/env python3
"""Open Infra MCP server: civil, GIS and 3D modelling in Open CAD Studio.

A Model Context Protocol server (stdio, JSON-RPC 2.0, Python standard
library only). It drives the Open Infra plugins (GIS tab: GIS_ commands,
Civiel tab: CIV_ commands) in a running Open CAD Studio through its REST
API, and reads each command's outcome back: the plugins write it to
<config>/OpenCADStudio/open-infra/api/<id>.json when a command carries ID=<id>.

Every Open CAD Studio window is found through its automation descriptor
(<config>/OpenCADStudio/automation/*.json: port, token, pid); no setup is
needed. OI_SESSION picks a window (session id, pid or drawing name);
OI_REST_PORT with OCS_API_TOKEN uses a window's REST API (--http) instead.

Coordinates are project coordinates: metres in the project CRS (RD,
EPSG:28992, in the Netherlands), set with oi_site.
"""

import base64
import glob
import json
import os
import socket
import sys
import time
import urllib.error
import urllib.request
import uuid

VERSION = "2026.45"
PROTOCOL = "2025-06-18"


def config_dir():
    if sys.platform == "win32":
        return os.environ.get("APPDATA", os.path.expanduser("~"))
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support")
    return os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")


AUTOMATION = os.path.join(config_dir(), "OpenCADStudio", "automation")
RESULTS = os.path.join(config_dir(), "OpenCADStudio", "open-infra", "api")


class OcsError(Exception):
    pass


# ── Sessions ─────────────────────────────────────────────────────────────────
#
# Every Open CAD Studio window runs an automation endpoint and describes it
# in <config>/OpenCADStudio/automation/<session>.json (port, token, pid): one
# JSON line in, one reply out, over TCP on 127.0.0.1. With OI_REST_PORT and
# OCS_API_TOKEN set, the REST API of a window started with --http is used
# instead.

def _alive(pid):
    if not pid or sys.platform == "win32":
        return True
    try:
        os.kill(int(pid), 0)
        return True
    except OSError:
        return False


def _exchange(desc, request, timeout=120):
    req = dict(request, token=desc["token"], session_id=desc["session_id"], protocol=1)
    wire = (json.dumps(req) + "\n").encode()
    with socket.create_connection(("127.0.0.1", int(desc["port"])), timeout=timeout) as s:
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        s.settimeout(timeout)
        s.sendall(wire)
        buf = b""
        while True:
            chunk = s.recv(1 << 16)
            if not chunk:
                break
            buf += chunk
    if not buf.strip():
        raise OcsError("geen antwoord van Open CAD Studio")
    return json.loads(buf)


def _rest(port, token, path, body=None, timeout=120):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/api/v1{path}",
        data=None if body is None else json.dumps(body).encode(),
        method="POST" if body is not None else "GET",
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            return json.loads(e.read())
        except ValueError:
            return {"ok": False, "error": f"HTTP {e.code}"}


def sessions():
    """Live windows, newest first: their session id, document, file and version."""
    out = []
    if os.environ.get("OI_REST_PORT") and os.environ.get("OCS_API_TOKEN"):
        port, token = os.environ["OI_REST_PORT"], os.environ["OCS_API_TOKEN"]
        st = _rest(port, token, "/state", timeout=5)
        if "document_id" in st:
            out.append({"kind": "rest", "session_id": f"rest:{port}", "port": port, "token": token, "pid": None,
                        "version": st.get("version"), "document_id": st.get("document_id")})
        return out
    paths = sorted(glob.glob(os.path.join(AUTOMATION, "*.json")), key=os.path.getmtime, reverse=True)
    for p in paths:
        try:
            with open(p) as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue
        if not d.get("port") or not d.get("token") or not _alive(d.get("pid")):
            continue
        try:
            st = _exchange(d, {"op": "state"}, timeout=3)
        except (OSError, ValueError, OcsError):
            continue
        if "document_id" not in st:
            continue
        docs = st.get("documents") or []
        cur = next((x for x in docs if x.get("id") == st.get("document_id")), {}) if isinstance(docs, list) else {}
        out.append({"kind": "automation", "session_id": d.get("session_id"), "port": d["port"], "token": d["token"], "pid": d.get("pid"),
                    "version": st.get("version"), "document_id": st.get("document_id"),
                    "document": cur.get("path") or cur.get("name") or cur.get("title")})
    return out


class Ocs:
    def __init__(self):
        self.session = None

    def pick(self, select=None):
        live = sessions()
        want = select or os.environ.get("OI_SESSION")
        if want:
            match = [s for s in live if want in (s["session_id"], str(s["pid"]), str(s["port"])) or (s.get("document") and want.lower() in str(s["document"]).lower())]
            if select and not match:
                raise OcsError(f"geen venster '{select}'; kies uit: " + ", ".join(f"{s['session_id']} ({s.get('document')})" for s in live))
            live = match or live
        if not live:
            raise OcsError("Geen Open CAD Studio-venster gevonden. Start Open CAD Studio (met de Open Infra-plugins).")
        self.session = live[0]
        return self.session

    def request(self, op, body=None, timeout=120):
        """One automation operation (state, run, capture, save, open, …)."""
        s = self.session or self.pick()
        body = dict(body or {})
        if s["kind"] == "rest":
            return _rest(s["port"], s["token"], "/state" if op == "state" else f"/{op}", None if op == "state" else body, timeout)
        if op != "state":
            body.setdefault("request_id", "mcp-" + uuid.uuid4().hex)
        try:
            return _exchange(s, dict(body, op=op), timeout)
        except (OSError, ValueError):
            # The window went away: find it (or another) again.
            self.session = None
            s = self.pick()
            return _exchange(s, dict(body, op=op), timeout)

    def document_id(self):
        # While a drawing loads the window may not answer at once.
        for _ in range(60):
            d = self.request("state").get("document_id")
            if d is not None:
                return d
            time.sleep(1)
        raise OcsError("Open CAD Studio geeft geen actieve tekening")

    def host(self, cmd):
        """A host command (no Open Infra answer to wait for)."""
        r = self.request("run", {"cmd": cmd, "document_id": self.document_id()})
        if r.get("ok") is False:
            raise OcsError(f"{cmd}: {r.get('error')}")
        return r

    def run(self, cmd, timeout=900):
        """An Open Infra command; waits for its JSON answer."""
        rid = "mcp" + uuid.uuid4().hex[:12]
        path = os.path.join(RESULTS, rid + ".json")
        r = self.request("run", {"cmd": f"{cmd} ID={rid}", "document_id": self.document_id()})
        # A busy window answers "still running"; the command goes on and
        # its result file comes when it is done.
        busy = "may continue" in str(r.get("error") or "") or r.get("status") in ("pending", "running", "accepted")
        if r.get("ok") is False and not busy:
            raise OcsError(f"{cmd.split()[0]}: {r.get('error')}")
        t0 = time.time()
        while time.time() - t0 < timeout:
            if os.path.exists(path):
                with open(path) as f:
                    out = json.load(f)
                try:
                    os.remove(path)
                except OSError:
                    pass
                out["seconds"] = round(time.time() - t0, 1)
                return out
            time.sleep(0.25)
        raise OcsError(f"{cmd.split()[0]}: geen antwoord binnen {timeout} s (draait het nog? zie de opdrachtregel)")


OCS = Ocs()


# ── Argument helpers ─────────────────────────────────────────────────────────

def _num(v):
    return ("%.6f" % float(v)).rstrip("0").rstrip(".")


def _pt(p):
    return ",".join(_num(c) for c in p)


def _pts(ps):
    return ";".join(_pt(p) for p in ps)


def _list(vs):
    return ";".join(_num(v) for v in vs)


def _kv(**kw):
    out = []
    for k, v in kw.items():
        if v is None or v == "" or v == []:
            continue
        if isinstance(v, bool):
            v = "ja" if v else "nee"
        elif isinstance(v, (int, float)):
            v = _num(v)
        elif " " in str(v) and not str(v).startswith('"'):
            v = f'"{v}"'
        out.append(f"{k}={v}")
    return " ".join(out)


def _quote(path):
    return f'"{path}"' if " " in path else path


# ── Tools ────────────────────────────────────────────────────────────────────

NUM = {"type": "number"}
STR = {"type": "string"}
PT = {"type": "array", "items": NUM, "minItems": 2, "maxItems": 3, "description": "[x, y] or [x, y, z], project coordinates (m)"}
PTS = {"type": "array", "items": PT}
AXIS = {"type": "string", "description": "Handle of the axis (hex, from civ_alignment or oi_status); empty = the selection"}
SURF = {"type": "string", "enum": ["B", "N"], "description": "B = existing ground, N = design surface"}

TOOLS = []


def tool(name, description, properties=None, required=None):
    def wrap(fn):
        TOOLS.append({
            "name": name,
            "description": description,
            "inputSchema": {"type": "object", "properties": properties or {}, "required": required or []},
            "fn": fn,
        })
        return fn
    return wrap


@tool("infra_catalog", "Search the NLCS 5.0.2 infrastructure library; returns parameters, connectors, material variants and source conflicts.",
      {"query": STR, "category": STR})
def t_infra_catalog(a):
    return OCS.run("GIS_INFRA_CATALOG " + _kv(QUERY=a.get("query"), CATEGORY=a.get("category")))


def _infra_parameters(values):
    import math
    out = []
    for key, value in values.items():
        if not key.replace('_', '').isalnum() or not key.isascii():
            raise ValueError("Invalid parameter name")
        if isinstance(value, bool) or not isinstance(value, (float, int)) or not math.isfinite(value):
            raise ValueError(f"{key}: a finite number is required")
        out.append(f"{key.upper()}={value}")
    return ' '.join(out)


@tool("infra_place", "Place a parametric infrastructure object or attach it to an existing instance. Coordinates in project metres; rotation in degrees. Conflict items require confirm=true and explicit design parameters. Returns instance, handles, ports and warnings.",
      {"item": STR, "at": PT, "path": PTS, "rotation": NUM, "scale": NUM,
       "status": {"type":"string", "enum":["B","N","T","V"]}, "discipline": STR,
       "parameters": {"type":"object", "additionalProperties": NUM},
       "instance": STR, "connect": STR, "port": STR, "own": STR, "confirm": {"type":"boolean"}}, ["item"])
def t_infra_place(a):
    parameters = _infra_parameters(a.get('parameters', {}))
    return OCS.run("GIS_INFRA_PLACE " + _kv(ITEM=a['item'], AT=_pt(a['at']) if 'at' in a else None,
        PATH=_pts(a['path']) if 'path' in a else None, ROTATION=a.get('rotation'), SCALE=a.get('scale'),
        STATUS=a.get('status'), DISCIPLINE=a.get('discipline'), INSTANCE=a.get('instance'),
        CONNECT=a.get('connect'), PORT=a.get('port'), OWN=a.get('own'),
        CONFIRM='true' if a.get('confirm') else None) + ' ' + parameters)


@tool("infra_inspect", "Read saved infrastructure instances, parameters, connector positions and warnings from the active drawing.", {"instance": STR})
def t_infra_inspect(a):
    return OCS.run("GIS_INFRA_INSPECT " + _kv(INSTANCE=a.get('instance')))


@tool("infra_update", "Regenerate a saved infrastructure instance and its connections atomically. Invalid or incompatible edits leave the drawing unchanged.",
      {"instance": STR, "parameters": {"type":"object", "additionalProperties": NUM}, "path": PTS}, ["instance"])
def t_infra_update(a):
    parameters = _infra_parameters(a.get('parameters', {}))
    return OCS.run("GIS_INFRA_UPDATE " + _kv(INSTANCE=a['instance'], PATH=_pts(a['path']) if 'path' in a else None) + ' ' + parameters)


@tool("oi_sessions", "List the running Open CAD Studio windows; `select` picks one by session id, pid or drawing name.",
      {"select": STR})
def t_sessions(a):
    if a.get("select"):
        OCS.pick(a["select"])
    live = sessions()
    cur = OCS.session or (live[0] if live else None)
    return {"sessions": [{k: s.get(k) for k in ("session_id", "pid", "version", "document_id", "document")} for s in live],
            "selected": cur and cur["session_id"]}


@tool("oi_status", "The model: project location, axes (handles), length and design profiles, corridors, existing (B) and design (N) terrain, survey points.")
def t_status(a):
    return OCS.run("CIV_INFO")


@tool("oi_command", "Run one command line: Open Infra (GIS_…, CIV_…; waits for its answer) or a host command (e.g. 'ZOOM EXTENTS'). See GIS_HELP and CIV_HELP for the arguments.",
      {"command": STR, "timeout_s": NUM}, ["command"])
def t_command(a):
    c = a["command"].strip()
    if c.upper().startswith(("GIS_", "CIV_")):
        return OCS.run(c, int(a.get("timeout_s") or 900))
    OCS.host(c)
    return {"ok": True, "message": f"{c}: uitgevoerd"}


@tool("oi_site", "Set the project location and import area: RD centre (x, y) or WGS84 (lon, lat), width and height in m. Everything else works in this area.",
      {"x": NUM, "y": NUM, "lon": NUM, "lat": NUM, "width": NUM, "height": NUM, "name": STR, "relative": {"type": "boolean"}})
def t_site(a):
    w, h = a.get("width", 300), a.get("height", a.get("width", 250))
    rel = " RELATIEF" if a.get("relative") else ""
    name = (" " + a["name"]) if a.get("name") else ""
    if a.get("x") is not None and a.get("y") is not None:
        return OCS.run(f"GIS_SITE RD {_num(a['x'])} {_num(a['y'])} {_num(w)} {_num(h)}{rel}{name}")
    if a.get("lon") is not None and a.get("lat") is not None:
        return OCS.run(f"GIS_SITE {_num(a['lon'])} {_num(a['lat'])} {_num(w)} {_num(h)}{rel}{name}")
    raise OcsError("geef x en y (RD) of lon en lat")


DATASETS = {
    "bgt": "GIS_Q_BGT", "brk": "GIS_Q_BRK", "luchtfoto": "GIS_Q_LUCHTFOTO", "kadastrale_kaart": "GIS_Q_KADASTER",
    "ahn_kaart": "GIS_Q_AHN", "bag": "GIS_Q_BAG", "percelen": "GIS_Q_PERCELEN", "nwb": "GIS_Q_NWB",
    "hectometer": "GIS_Q_HECTO", "spoor": "GIS_Q_SPOOR", "osm": "GIS_Q_OSM", "osm_kaart": "GIS_Q_OSMKAART",
    "satelliet": "GIS_Q_SATELLIET", "tekenblad": "GIS_TEKENBLAD", "ahn_hoogtelijnen": "GIS_HOOGTELIJNEN",
    "ahn_hoogtepunten": "GIS_HOOGTEPUNTEN", "ahn_puntenwolk": "GIS_PUNTENWOLK",
}


@tool("gis_import", "Import a dataset into the project area on NLCS layers: " + ", ".join(DATASETS),
      {"dataset": {"type": "string", "enum": list(DATASETS)}}, ["dataset"])
def t_gis(a):
    return OCS.run(DATASETS[a["dataset"]])


SERVICE_CATEGORIES = ["cadastre", "aerial", "elevation", "subsurface", "zoning", "buildings", "buildings3d", "basemap", "environment",
                      "roads", "protected", "water", "rail", "landcover", "admin", "structures", "utilities"]


@tool("gis_services", "Search the catalogue of about 620 public WMS, WFS and XYZ tile services in 50+ countries (cadastral parcels, "
      "aerial photos, elevation, geology, zoning, buildings …). Free text in Dutch or English (e.g. 'cadastral parcels Bavaria', "
      "'luchtfoto', 'Flurstücke'); `here` keeps only services covering the project location, or give lon/lat. Returns id, type, "
      "name, provider, country, coverage, category, url, default_layer, crs (request CRS first), license and bbox per service. "
      "country filters ISO codes separated by semicolons, e.g. US;GLOBAL; omit it for all regions.",
      {"query": STR, "category": {"type": "string", "enum": SERVICE_CATEGORIES}, "type": {"type": "string", "enum": ["WMS", "WFS", "XYZ"]},
       "country": STR, "here": {"type": "boolean"}, "lon": NUM, "lat": NUM, "limit": NUM})
def t_services(a):
    words = " HIER" if a.get("here") and a.get("lon") is None else ""
    return OCS.run("GIS_SERVICES " + _kv(ZOEK=a.get("query"), CATEGORIE=a.get("category"), TYPE=a.get("type"),
                                         COUNTRY=a.get("country"), LON=a.get("lon"), LAT=a.get("lat"), LIMIET=a.get("limit") or 50) + words)


@tool("gis_service_load", "Load one layer of a catalogue service (id from gis_services) into the project area: WMS and tiles as "
      "images, WFS as features with attributes, reprojected to the project CRS. `layer` defaults to the service's preset layer.",
      {"service": STR, "layer": STR, "resolution": NUM, "limit": NUM}, ["service"])
def t_service_load(a):
    return OCS.run("GIS_SERVICE_LOAD " + _kv(DIENST=a["service"], LAAG=a.get("layer"), RESOLUTIE=a.get("resolution"), LIMIET=a.get("limit")))


@tool("planning_search", "Find legacy IMRO/TAM plans at an RD point or the project location. Requires OPENINFRA_RP_API_KEY in the plugin process; never pass a key as an argument. Returns plan IDs, status and dates.", {"x": NUM, "y": NUM})
def t_planning_search(a):
    if (a.get("x") is None) != (a.get("y") is None):
        raise ValueError("Provide both RD x and y")
    return OCS.run("GIS_PLAN_SEARCH " + _kv(X=a.get("x"), Y=a.get("y")))


@tool("planning_import", "Download a selected legacy IMRO/TAM plan as editable styled geometry. Requires local OPENINFRA_RP_API_KEY and a project location.", {"plan": STR}, ["plan"])
def t_planning_import(a):
    return OCS.run("GIS_PLAN_IMPORT " + _kv(PLAN=a["plan"]), 1800)


@tool("planning_gml", "Import local IMRO GML with planning styles and source metadata; no API key required.", {"path": STR}, ["path"])
def t_planning_gml(a):
    return OCS.run("GIS_PLAN_GML " + _quote(a["path"]), 1800)


@tool("nlcs_check", "NLCS 5.02 check of the drawing (report beside the drawing); repair=true sets layer styles and ByLayer.",
      {"repair": {"type": "boolean"}})
def t_nlcs(a):
    return OCS.run("GIS_NLCS_HERSTEL" if a.get("repair") else "GIS_NLCS_RAPPORT")


@tool("bgt_mutation", "The NLCS design as a BGT mutation (IMGeo-GML 2.1.1) with a report, beside the drawing. "
      "Layers N become plan topography (as_built=true: built), R built (changed when it carries a BGT lokaal_id), "
      "V ends the BGT object. Type and function come from the BGT object data, the BGT object beneath or the NLCS layer; "
      "ambiguous objects are listed in the report, not exported.",
      {"bronhouder": STR, "as_built": {"type": "boolean"}, "selection": {"type": "boolean"}})
def t_mutation(a):
    words = (" GEREALISEERD" if a.get("as_built") else "") + (" SELECTIE" if a.get("selection") else "")
    return OCS.run(f"GIS_BGT_MUTATIE {_kv(BRONHOUDER=a.get('bronhouder'))}{words}")


@tool("ifc_export", "Export the drawing as IFC 4.3 (IFC4X3_ADD2), beside it as <drawing>_IFC4X3.ifc or to `path`. "
      "Every object becomes its IFC class from its NLCS layer and object data: terrain (TIN, photo terrain), 3D BAG and CityGML "
      "buildings, pavements, green, water, structures, cables and pipes, sewers, earthworks, CPTs and boreholes (IfcBorehole with "
      "IfcGeotechnicalStratum); axes become IfcAlignments with horizontal and vertical layout. Georeferenced from the project "
      "location; object data as property sets. Answers with the counts per IFC class.",
      {"path": STR, "selection": {"type": "boolean"}, "texts": {"type": "boolean"}})
def t_ifc(a):
    words = (" SELECTIE" if a.get("selection") else "") + (" TEKST=nee" if a.get("texts") is False else "")
    return OCS.run(f"CIV_IFC {_kv(BESTAND=a.get('path'))}{words}", 1800)


@tool("quantities", "Quantities per NLCS layer (count, length, area) as CSV beside the drawing.")
def t_quantities(a):
    return OCS.run("GIS_HOEVEELHEDEN_CSV")


@tool("civ_points_import", "Import a survey point file (PNEZD/PENZD/CSV; separator and decimal comma detected).",
      {"path": STR, "format": {"type": "string", "enum": ["AUTO", "PNEZD", "PENZD", "NEZD", "ENZD"]}}, ["path"])
def t_points_import(a):
    return OCS.run(f"CIV_PUNTEN_IMPORT {_quote(a['path'])} {_kv(FORMAAT=a.get('format'))}")


@tool("civ_points", "Place survey points; without z the ground height (TIN or AHN) is used.",
      {"points": PTS, "number": STR, "code": STR}, ["points"])
def t_points(a):
    return OCS.run(f"CIV_PUNT {' '.join(_pt(p) for p in a['points'])} {_kv(NR=a.get('number'), CODE=a.get('code'))}")


@tool("civ_points_export", "Export the drawing's points (or the selection) to a point file.", {"path": STR, "format": STR})
def t_points_export(a):
    return OCS.run(f"CIV_PUNTEN_EXPORT {_kv(PAD=a.get('path') and _quote(a['path']), FORMAAT=a.get('format'))}")


@tool("civ_landxml", "Import LandXML: points, surfaces (TIN) and alignments (lines, curves, spirals).", {"path": STR}, ["path"])
def t_landxml(a):
    return OCS.run(f"CIV_LANDXML {_quote(a['path'])}")


@tool("civ_tin", "Make a terrain (TIN, 3D faces on OG-TRIANGULATIE): from AHN in the project area, the survey points, the selection or a layer. status N makes a design surface.",
      {"source": {"type": "string", "enum": ["AHN", "PUNTEN", "SELECTIE"]}, "layer": STR, "spacing": NUM, "max_edge": NUM, "status": SURF})
def t_tin(a):
    return OCS.run(f"CIV_TIN {_kv(BRON=a.get('source'), LAAG=a.get('layer'), AFSTAND=a.get('spacing'), MAXRIBBE=a.get('max_edge'), STATUS=a.get('status'))}")


@tool("civ_contours", "Contour lines of a terrain (default: existing ground).", {"interval": NUM, "major_every": NUM, "surface": SURF})
def t_contours(a):
    return OCS.run(f"CIV_HOOGTELIJNEN {_kv(INTERVAL=a.get('interval'), HOOFD=a.get('major_every'), OPPERVLAK=a.get('surface'))}")


@tool("civ_analysis", "Height analysis: the terrain coloured in height bands with a legend.", {"bands": NUM, "surface": SURF})
def t_analysis(a):
    return OCS.run(f"CIV_ANALYSE {_kv(BANDEN=a.get('bands'), OPPERVLAK=a.get('surface'))}")


@tool("civ_volume", "Earthwork (cut/fill m³): design surface (N, or selected faces) against the existing ground, or the ground against a level.",
      {"level": NUM, "cell": NUM})
def t_volume(a):
    if a.get("level") is not None:
        return OCS.run(f"CIV_VOLUME ONTWERP=PEIL {_kv(PEIL=a['level'], CEL=a.get('cell'))}")
    return OCS.run(f"CIV_VOLUME {_kv(CEL=a.get('cell'))}")


@tool("civ_alignment", "Create an axis (alignment) with stationing, tangent points and radii: through points of intersection with curve radii (by layout), or from a line by handle.",
      {"pis": PTS, "radii": {"type": "array", "items": NUM}, "handle": STR, "name": STR, "start_station": NUM})
def t_alignment(a):
    if a.get("pis"):
        return OCS.run(f"CIV_AS PI={_pts(a['pis'])} {_kv(R=_list(a.get('radii') or []), NAAM=a.get('name'), BEGIN=a.get('start_station'))}")
    return OCS.run(f"CIV_AS {_kv(AS=a.get('handle'), NAAM=a.get('name'), BEGIN=a.get('start_station'))}")


@tool("civ_offsets", "Offset lines beside an axis (m; left < 0).", {"axis": AXIS, "offsets": {"type": "array", "items": NUM}}, ["offsets"])
def t_offsets(a):
    return OCS.run(f"CIV_AS_OFFSET {_kv(AS=a.get('axis'), VERSCHUIVING=_list(a['offsets']))}")


@tool("civ_station", "Station and offset of points relative to an axis.", {"axis": AXIS, "points": PTS}, ["points"])
def t_station(a):
    return OCS.run(f"CIV_STATION {_kv(AS=a.get('axis'))} {' '.join(_pt(p) for p in a['points'])}")


@tool("civ_length_profile", "Length profile of an axis over the existing terrain (TIN, else AHN), with stationing, heights and alignment band.",
      {"axis": AXIS, "at": PT, "step": NUM, "exaggeration": NUM})
def t_length_profile(a):
    return OCS.run(f"CIV_LENGTEPROFIEL {_kv(AS=a.get('axis'), PLAATS=a.get('at') and _pt(a['at']), STAP=a.get('step'), OVERHOGING=a.get('exaggeration'))}")


@tool("civ_design_profile", "Design (vertical) profile of an axis from PVIs [station, height] with parabolic vertical curves; drawn in its length profile.",
      {"axis": AXIS, "pvis": {"type": "array", "items": {"type": "array", "items": NUM, "minItems": 2, "maxItems": 2}}, "curve_length": NUM}, ["pvis"])
def t_design(a):
    pvi = ";".join(f"{_num(s)}:{_num(z)}" for s, z in a["pvis"])
    return OCS.run(f"CIV_ONTWERPPROFIEL {_kv(AS=a.get('axis'), PVI=pvi, BOOG=a.get('curve_length'))}")


@tool("civ_cross_sections", "Cross sections along an axis over the terrain, in a grid of views.",
      {"axis": AXIS, "at": PT, "every": NUM, "left": NUM, "right": NUM, "exaggeration": NUM, "per_row": NUM})
def t_sections(a):
    return OCS.run(f"CIV_DWARSPROFIELEN {_kv(AS=a.get('axis'), PLAATS=a.get('at') and _pt(a['at']), OM=a.get('every'), LINKS=a.get('left'), RECHTS=a.get('right'), DOVERHOGING=a.get('exaggeration'), PERRIJ=a.get('per_row'))}")


@tool("civ_corridor", "Corridor: an assembly swept along an axis and its design profile (or a level / height above ground), daylight to the ground, design TIN (N), feature lines, cut/fill per section and volumes; optional section views.",
      {"axis": AXIS, "assembly": {"type": "string", "enum": ["weg", "fietspad", "snelweg", "dijk"]}, "level": NUM, "above_ground": NUM,
       "lane_width": NUM, "shoulder_width": NUM, "cross_slope_pct": NUM, "shoulder_slope_pct": NUM, "fill_slope": NUM, "cut_slope": NUM,
       "crest_half_width": NUM, "superelevation_pct": NUM, "reach": NUM, "frequency": NUM, "sections": {"type": "boolean"}, "sections_at": PT})
def t_corridor(a):
    views = a.get("sections_at") and _pt(a["sections_at"]) or (True if a.get("sections") else None)
    return OCS.run("CIV_CORRIDOR " + _kv(
        AS=a.get("axis"), SJABLOON=a.get("assembly"), NIVEAU=a.get("level"), BOVEN=a.get("above_ground"),
        RIJSTROOK=a.get("lane_width"), BERM=a.get("shoulder_width"), HELLING=a.get("cross_slope_pct"), BERMHELLING=a.get("shoulder_slope_pct"),
        TALUD=a.get("fill_slope"), INGRAVING=a.get("cut_slope"), KRUIN=a.get("crest_half_width"), VERKANTING=a.get("superelevation_pct"),
        REIKWIJDTE=a.get("reach"), FREQUENTIE=a.get("frequency"), DWARSPROFIELEN=views))


@tool("civ_bag3d", "3D BAG buildings of the project area (LoD 1.2, 1.3 or 2.2): roof and wall meshes per building. "
      "style: mooi (roof colours from the aerial photo, walls by building year, windows per storey, crisp planes), "
      "maquette (white architectural model), realistisch (two plain colours) or nlcs (layer colours).",
      {"lod": {"type": "string", "enum": ["1.2", "1.3", "2.2"]}, "max": NUM, "style": {"type": "string", "enum": ["mooi", "maquette", "realistisch", "nlcs"]},
       "windows": {"type": "boolean", "description": "false: plain facades without windows"}})
def t_bag3d(a):
    ramen = "nee" if a.get("windows") is False else None
    return OCS.run(f"CIV_BAG3D {_kv(LOD=a.get('lod'), MAX=a.get('max'), STIJL=a.get('style'), RAMEN=ramen)}", 1800)


@tool("civ_photo_terrain", "The AHN ground of the project area as a 3D surface coloured by the PDOK aerial photo.", {"cell": NUM})
def t_photo(a):
    return OCS.run(f"CIV_FOTOTERREIN {_kv(CEL=a.get('cell'))}", 1800)


@tool("civ_water_level", "A water surface at a level (m NAP) over the project area, to see what floods; call again with another "
      "level to move it. Returns the share of the area below that level (AHN ground).",
      {"level": NUM}, ["level"])
def t_water(a):
    return OCS.run(f"CIV_WATERPEIL PEIL={_num(a['level'])}")


@tool("gis_server", "Import any map server of the catalogue by id (PDOK, 600 worldwide services, HisGIS, Topotijdreis, the user's own): "
      "tile/WMS as image, WFS as vectors. year gives Topotijdreis of that year (1815-2014).",
      {"id": STR, "layer": STR, "resolution": NUM, "year": NUM})
def t_server(a):
    if a.get("year"):
        return OCS.run(f"GIS_Q_SERVER JAAR={int(a['year'])} {_kv(RES=a.get('resolution'))}", 900)
    return OCS.run(f"GIS_Q_SERVER {a['id']} {_kv(LAAG=a.get('layer'), RES=a.get('resolution'))}", 900)


@tool("civ_klic3d", "KLIC cables and pipes as 3D tubes under the AHN ground (depth from the delivery, else the usual cover depth per network type), "
      "in IMKL colours. Without path: the delivery imported last. scale thickens the tubes for overview pictures.",
      {"path": STR, "scale": NUM})
def t_klic3d(a):
    return OCS.run(f"CIV_KLIC3D {a.get('path') or ''} {_kv(VERGROOT=a.get('scale'))}", 1800)


@tool("civ_see_through", "Make the ground surfaces transparent (percent, default 70) so cables, pipes and CPT columns show beneath; "
      "all=true also buildings; percent=0 makes the ground opaque again.",
      {"percent": NUM, "all": {"type": "boolean"}})
def t_see_through(a):
    pct = a.get("percent")
    if pct is not None and float(pct) <= 0:
        return OCS.run("CIV_DOORZICHT UIT")
    return OCS.run(f"CIV_DOORZICHT {_kv(PCT=pct)} {'ALLES' if a.get('all') else ''}")


@tool("civ_citygml", "Import a CityGML file (2.0 or 3.0, also IMGeo/BGT GML) or CityJSON (1.x/2.0, also CityJSONSeq .jsonl) as 3D meshes in the project area: buildings with roof and wall "
      "meshes (LoD1/LoD2, walls by year of construction), roads, green, water, bridges and relief on NLCS layers; 2D objects draped on the AHN. "
      "The file's CRS is transformed to the project location.",
      {"path": STR, "style": {"type": "string", "enum": ["mooi", "maquette", "realistisch", "nlcs"]}}, ["path"])
def t_citygml(a):
    return OCS.run(f"CIV_CITYGML {a['path']} {_kv(STIJL=a.get('style'))}", 1800)


@tool("civ_bgt3d", "BGT in 3D: areas draped on the ground, buildings and walls extruded to the AHN surface, water at its level, bridges at deck height, posts and trees as symbols.",
      {"groups": {"type": "array", "items": {"type": "string", "enum": ["wegen", "terrein", "water", "gebouwen", "kunstwerken", "scheidingen", "spoor", "inrichting", "vegetatie"]}},
       "buildings": {"type": "boolean"}, "colours": {"type": "string", "enum": ["realistisch", "nlcs"]}})
def t_bgt3d(a):
    groups = ";".join(a.get("groups") or [])
    return OCS.run(f"CIV_BGT3D {_kv(GROEPEN=groups, PANDEN=a.get('buildings'), KLEUR=a.get('colours'))}", 1800)


@tool("civ_subsurface", "The subsurface from the BRO (Dutch key register, successor of DINOloket): cone penetration tests (CPT) and "
      "geotechnical boreholes (BHR-GT) of the project area as 3D columns from the surface level (m NAP) down, one mesh per soil "
      "layer coloured by soil type (sand yellow, clay green, peat brown, silt grey-green, gravel orange); CPT layers from qc and Rf "
      "(Robertson 2010 SBT), boreholes from their described soil names; qc/Rf graphs beside the CPTs. plan_only=true draws just "
      "the plan symbols with BRO id (no measurements fetched). Returns per column the BRO id, surface level and layers.",
      {"source": {"type": "string", "enum": ["alles", "sonderingen", "boringen"]}, "max": NUM, "radius": NUM, "min_layer": NUM,
       "graphs": {"type": "boolean"}, "labels": {"type": "boolean"}, "plan_only": {"type": "boolean"}})
def t_subsurface(a):
    if a.get("plan_only"):
        return OCS.run(f"GIS_Q_SONDERINGEN {_kv(BRON=a.get('source'))}", 600)
    return OCS.run(f"CIV_SONDERINGEN3D {_kv(BRON=a.get('source'), MAX=a.get('max'), STRAAL=a.get('radius'), MINLAAG=a.get('min_layer'), QC=a.get('graphs'), LABELS=a.get('labels'))}", 1800)


@tool("oi_view", "Set the view: preset (TOP, SWISO, SEISO, NEISO, NWISO, FRONT…), visual style (GOURAUDSHADED, FLATSHADED, WIREFRAME2D, HIDDENLINE…) and zoom to extents.",
      {"preset": STR, "style": STR, "zoom_extents": {"type": "boolean"}})
def t_view(a):
    done = []
    if a.get("preset"):
        OCS.host(f"-VIEW {a['preset'].upper()}")
        done.append(a["preset"].upper())
    if a.get("style"):
        OCS.host(f"VSCURRENT {a['style'].upper()}")
        done.append(a["style"].upper())
    if a.get("zoom_extents", True):
        OCS.host("ZOOM EXTENTS")
        done.append("zoom extents")
    return {"ok": True, "message": ", ".join(done)}


@tool("oi_capture", "Capture the drawing viewport as PNG (returned as an image). region = [xmin, ymin, xmax, ymax] in drawing coordinates.",
      {"region": {"type": "array", "items": NUM, "minItems": 4, "maxItems": 4}, "max_dimension": NUM, "path": STR})
def t_capture(a):
    path = a.get("path") or os.path.join(RESULTS, "capture-" + uuid.uuid4().hex[:8] + ".png")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    body = {"path": path, "document_id": OCS.document_id()}
    if a.get("region"):
        body.update({"view": "region", "bounds": a["region"]})
    if a.get("max_dimension"):
        body["max_dimension"] = int(a["max_dimension"])
    r = OCS.request("capture", body, timeout=180)
    if r.get("ok") is False:
        raise OcsError(f"capture: {r.get('error')}")
    # The window writes the image after answering.
    t0 = time.time()
    while not (os.path.exists(path) and os.path.getsize(path) > 0) and time.time() - t0 < 60:
        time.sleep(0.25)
    if not os.path.exists(path):
        raise OcsError("capture: geen afbeelding ontvangen (is het venster zichtbaar?)")
    time.sleep(0.3)
    with open(path, "rb") as f:
        png = base64.b64encode(f.read()).decode()
    return {"__image__": png, "ok": True, "message": f"capture: {path}"}


@tool("oi_save", "Save the drawing (QSAVE), or as a new file with path.", {"path": STR})
def t_save(a):
    if a.get("path"):
        r = OCS.request("save", {"path": a["path"], "document_id": OCS.document_id()}, timeout=300)
        if not r.get("ok"):
            raise OcsError(f"save: {r.get('error')}")
        return {"ok": True, "message": f"opgeslagen: {a['path']}"}
    OCS.host("QSAVE")
    return {"ok": True, "message": "opgeslagen"}


@tool("oi_open", "Open a drawing (DWG/DXF) or start a new one from a template.", {"path": STR, "template": STR})
def t_open(a):
    r = OCS.request("open", {"path": a["path"]}, timeout=300) if a.get("path") else OCS.request("new", {"template": a.get("template", "")}, timeout=300)
    if not r.get("ok", True) and r.get("error"):
        raise OcsError(f"open: {r.get('error')}")
    return {"ok": True, "message": json.dumps(r)[:300]}


# ── MCP protocol (JSON-RPC over stdio) ───────────────────────────────────────

INSTRUCTIONS = (
    "Open Infra: infra and civil modelling in Open CAD Studio (Netherlands: PDOK, AHN, BGT, 3D BAG, NLCS). "
    "Start with oi_sessions and oi_site (project location and area), then import data (gis_import), "
    "terrain (civ_tin), axes (civ_alignment), profiles (civ_length_profile, civ_design_profile), corridors (civ_corridor), "
    "earthwork (civ_volume), the 3D model (civ_photo_terrain, civ_bag3d, civ_bgt3d) and the subsurface (civ_subsurface: BRO CPTs and boreholes). oi_status lists handles; "
    "oi_view and oi_capture show the result. Coordinates are project coordinates in metres (RD in NL)."
)


def call_tool(name, args):
    t = next((t for t in TOOLS if t["name"] == name), None)
    if not t:
        return {"content": [{"type": "text", "text": f"onbekende tool {name}"}], "isError": True}
    try:
        r = t["fn"](args or {})
    except (OcsError, urllib.error.URLError, OSError, ValueError, KeyError) as e:
        return {"content": [{"type": "text", "text": f"{name}: {e}"}], "isError": True}
    content = []
    if isinstance(r, dict) and "__image__" in r:
        content.append({"type": "image", "data": r.pop("__image__"), "mimeType": "image/png"})
    ok = not (isinstance(r, dict) and r.get("ok") is False)
    text = r.get("message") if isinstance(r, dict) and "message" in r else json.dumps(r, ensure_ascii=False)
    if isinstance(r, dict) and r.get("data"):
        text += "\n" + json.dumps(r["data"], ensure_ascii=False)[:20000]
    content.insert(0, {"type": "text", "text": text})
    out = {"content": content, "isError": not ok}
    if isinstance(r, dict) and isinstance(r.get("data"), dict):
        out["structuredContent"] = r["data"]
    return out


def handle(msg):
    method, mid, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
    if mid is None:
        return None  # a notification
    if method == "initialize":
        result = {
            "protocolVersion": params.get("protocolVersion") or PROTOCOL,
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "open-infra", "version": VERSION},
            "instructions": INSTRUCTIONS,
        }
    elif method == "ping":
        result = {}
    elif method == "tools/list":
        result = {"tools": [{k: t[k] for k in ("name", "description", "inputSchema")} for t in TOOLS]}
    elif method == "tools/call":
        result = call_tool(params.get("name"), params.get("arguments"))
    else:
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": f"onbekende methode {method}"}}
    return {"jsonrpc": "2.0", "id": mid, "result": result}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        reply = handle(msg)
        if reply is not None:
            sys.stdout.write(json.dumps(reply, ensure_ascii=False) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
