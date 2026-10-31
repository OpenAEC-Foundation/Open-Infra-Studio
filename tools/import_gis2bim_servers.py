#!/usr/bin/env python3
"""Turn the checked web service list of GIS2BIM into Open Infra presets.

Source: GIS2BIM_Data.json (branch `worldwide-servers` of
OpenAEC-Foundation/GIS2BIM). Every service in it was checked with
GIS2BIM's supportfiles/check_servers.py: GetCapabilities answered and a sample
request returned data. Only public services without an API key are listed.

WMS, WFS and XYZ tile templates are kept (the service kinds Open Infra reads);
WCS, ArcGIS REST, downloads and WFS with curves are left out. Dutch and Flemish
services that Open Infra already has as built-in presets are skipped.

Each preset carries what the service browser (GIS_SERVICES) searches and
filters on: the coordinate systems (`crs`, the request CRS first), `provider`,
`license`, the `coverage` text and a coverage bounding box `bbox` in WGS 84
[west, south, east, north]. GIS2BIM's `testbbox` is only a small sample area,
so the box comes from the tables below: the state, province or city the
coverage text names, otherwise the whole country or region.

Ids are made from GIS2BIM's titles and stay the same between runs.

Usage:
  git -C ../GIS2BIM show worldwide-servers:GIS2BIM_Data.json > /tmp/gis2bim.json
  # or: curl -L -o /tmp/gis2bim.json https://raw.githubusercontent.com/OpenAEC-Foundation/GIS2BIM/worldwide-servers/GIS2BIM_Data.json
  python3 tools/import_gis2bim_servers.py /tmp/gis2bim.json > assets/servers/world.json
"""

import json
import re
import sys
import unicodedata
from urllib.parse import parse_qs, urlsplit

# Country / region code → (Dutch, English) name.
NAMES = {
    "AR": ("Argentinië", "Argentina"), "AT": ("Oostenrijk", "Austria"), "AU": ("Australië", "Australia"),
    "BE": ("België", "Belgium"), "BG": ("Bulgarije", "Bulgaria"), "BR": ("Brazilië", "Brazil"),
    "CA": ("Canada", "Canada"), "CH": ("Zwitserland", "Switzerland"), "CL": ("Chili", "Chile"),
    "CO": ("Colombia", "Colombia"), "CZ": ("Tsjechië", "Czechia"), "DE": ("Duitsland", "Germany"),
    "DK": ("Denemarken", "Denmark"), "EE": ("Estland", "Estonia"), "ES": ("Spanje", "Spain"),
    "FI": ("Finland", "Finland"), "FR": ("Frankrijk", "France"), "GB": ("Verenigd Koninkrijk", "United Kingdom"),
    "GR": ("Griekenland", "Greece"), "HK": ("Hongkong", "Hong Kong"), "HR": ("Kroatië", "Croatia"),
    "ID": ("Indonesië", "Indonesia"), "IE": ("Ierland", "Ireland"), "IL": ("Israël", "Israel"),
    "IN": ("India", "India"), "IS": ("IJsland", "Iceland"), "IT": ("Italië", "Italy"), "JP": ("Japan", "Japan"),
    "LI": ("Liechtenstein", "Liechtenstein"), "LT": ("Litouwen", "Lithuania"), "LU": ("Luxemburg", "Luxembourg"),
    "LV": ("Letland", "Latvia"), "MX": ("Mexico", "Mexico"), "NL": ("Nederland", "Netherlands"),
    "NO": ("Noorwegen", "Norway"), "NZ": ("Nieuw-Zeeland", "New Zealand"), "PE": ("Peru", "Peru"),
    "PH": ("Filipijnen", "Philippines"), "PL": ("Polen", "Poland"), "PT": ("Portugal", "Portugal"),
    "SE": ("Zweden", "Sweden"), "SG": ("Singapore", "Singapore"), "SI": ("Slovenië", "Slovenia"),
    "SK": ("Slowakije", "Slovakia"), "TH": ("Thailand", "Thailand"), "TW": ("Taiwan", "Taiwan"),
    "US": ("Verenigde Staten", "United States"), "UY": ("Uruguay", "Uruguay"), "ZA": ("Zuid-Afrika", "South Africa"),
    "AFRICA": ("Afrika", "Africa"), "EU": ("Europa", "Europe"), "GLOBAL": ("Wereld", "World"),
}

# Country / region code → coverage in WGS 84 [west, south, east, north].
# Mainland plus the islands national services usually cover; far overseas
# territories are left out (one box per code).
COUNTRY_BBOX = {
    "AR": [-73.6, -55.1, -53.6, -21.8], "AT": [9.5, 46.37, 17.17, 49.03], "AU": [112.9, -43.7, 153.7, -9.0],
    "BE": [2.54, 49.49, 6.41, 51.51], "BG": [22.35, 41.23, 28.61, 44.22], "BR": [-74.0, -33.8, -34.7, 5.3],
    "CA": [-141.0, 41.7, -52.6, 83.1], "CH": [5.95, 45.82, 10.49, 47.81], "CL": [-75.7, -56.0, -66.4, -17.5],
    "CO": [-79.0, -4.3, -66.8, 12.5], "CZ": [12.09, 48.55, 18.86, 51.06], "DE": [5.87, 47.27, 15.04, 55.06],
    "DK": [8.07, 54.56, 15.2, 57.75], "EE": [21.76, 57.51, 28.21, 59.68], "ES": [-18.2, 27.6, 4.33, 43.8],
    "FI": [20.55, 59.8, 31.59, 70.09], "FR": [-5.2, 41.3, 9.6, 51.1], "GB": [-8.65, 49.86, 1.77, 60.86],
    "GR": [19.37, 34.8, 28.25, 41.75], "HK": [113.82, 22.15, 114.44, 22.57], "HR": [13.49, 42.39, 19.45, 46.55],
    "ID": [95.0, -11.0, 141.02, 6.08], "IE": [-10.48, 51.42, -5.99, 55.39], "IL": [34.27, 29.49, 35.9, 33.34],
    "IN": [68.1, 6.7, 97.4, 35.5], "IS": [-24.55, 63.3, -13.5, 66.57], "IT": [6.62, 35.49, 18.52, 47.09],
    "JP": [122.93, 24.04, 145.82, 45.52], "LI": [9.47, 47.05, 9.64, 47.27], "LT": [20.94, 53.9, 26.84, 56.45],
    "LU": [5.73, 49.45, 6.53, 50.18], "LV": [20.97, 55.67, 28.24, 58.09], "MX": [-118.4, 14.5, -86.7, 32.72],
    "NL": [3.2, 50.75, 7.23, 53.6], "NO": [4.5, 57.95, 31.1, 71.2], "NZ": [166.4, -47.3, 178.6, -34.4],
    "PE": [-81.4, -18.4, -68.65, -0.04], "PH": [116.9, 4.6, 126.6, 21.1], "PL": [14.12, 49.0, 24.15, 54.84],
    "PT": [-31.3, 32.6, -6.19, 42.15], "SE": [10.96, 55.34, 24.17, 69.06], "SG": [103.6, 1.16, 104.1, 1.48],
    "SI": [13.38, 45.42, 16.61, 46.88], "SK": [16.83, 47.73, 22.57, 49.61], "TH": [97.34, 5.61, 105.64, 20.46],
    "TW": [118.2, 21.9, 122.0, 26.4], "US": [-179.2, 17.6, -64.5, 71.4], "UY": [-58.44, -35.0, -53.07, -30.08],
    "ZA": [16.45, -34.84, 32.9, -22.12],
    "AFRICA": [-25.4, -35.0, 63.5, 37.6], "EU": [-31.3, 27.6, 44.8, 71.2], "GLOBAL": [-180.0, -90.0, 180.0, 90.0],
}

# Sub-national coverage: (words in the coverage text, bbox). The first match
# wins, so cities come before their state, "South Tyrol" before "Tyrol" and
# "New South Wales" before "Wales". Text after "except" is ignored
# ("Spain (except Basque Country and Navarre)" is Spain).
REGION_BBOX = [
    # Germany: cities, then the Länder.
    (("munich", "munchen"), [11.36, 48.06, 11.72, 48.25]),
    (("cologne", "koln"), [6.77, 50.83, 7.16, 51.08]),
    (("city of bremen",), [8.48, 53.01, 8.99, 53.23]),
    (("brandenburg and berlin",), [11.26, 51.36, 14.77, 53.56]),
    (("baden-wurttemberg",), [7.51, 47.53, 10.5, 49.79]),
    (("bavaria", "bayern"), [8.97, 47.27, 13.84, 50.57]),
    (("berlin, germany",), [13.08, 52.33, 13.77, 52.68]),
    (("brandenburg",), [11.26, 51.36, 14.77, 53.56]),
    (("bremen",), [8.48, 53.01, 8.99, 53.61]),
    (("hamburg",), [9.73, 53.39, 10.33, 53.74]),
    (("hesse", "hessen"), [7.77, 49.39, 10.24, 51.66]),
    (("mecklenburg",), [10.59, 53.11, 14.41, 54.69]),
    (("niedersachsen", "lower saxony"), [6.65, 51.29, 11.6, 53.89]),
    (("north rhine-westphalia", "nordrhein-westfalen"), [5.87, 50.32, 9.46, 52.53]),
    (("rhineland-palatinate", "rheinland-pfalz"), [6.11, 48.97, 8.51, 50.94]),
    (("saarland",), [6.36, 49.11, 7.4, 49.64]),
    (("saxony-anhalt", "sachsen-anhalt"), [10.56, 50.94, 13.19, 53.04]),
    (("saxony", "sachsen"), [11.87, 50.17, 15.04, 51.68]),
    (("schleswig-holstein",), [7.87, 53.36, 11.31, 55.06]),
    (("thuringia", "thuringen"), [9.88, 50.2, 12.65, 51.65]),
    # Italy before Austria: South Tyrol is not Tyrol.
    (("south tyrol", "bolzano"), [10.38, 46.22, 12.48, 47.09]),
    (("emilia-romagna",), [9.2, 43.73, 12.76, 45.14]),
    (("tuscany", "toscana"), [9.69, 42.24, 12.37, 44.47]),
    # Austria.
    (("vienna", "wien"), [16.18, 48.12, 16.58, 48.32]),
    (("tyrol", "tirol"), [10.1, 46.65, 12.97, 47.75]),
    (("vorarlberg",), [9.53, 46.84, 10.24, 47.6]),
    (("styria", "steiermark"), [13.56, 46.61, 16.17, 47.83]),
    (("carinthia", "karnten"), [12.65, 46.37, 15.07, 47.13]),
    (("salzburg",), [12.08, 46.94, 13.99, 48.04]),
    (("lower austria", "niederosterreich"), [14.45, 47.42, 17.07, 49.02]),
    # Switzerland.
    (("city of zurich",), [8.45, 47.32, 8.63, 47.44]),
    (("canton of zurich",), [8.36, 47.16, 8.99, 47.7]),
    # Belgium.
    (("flanders", "vlaanderen"), [2.54, 50.68, 5.92, 51.51]),
    (("wallonia", "wallonie"), [2.84, 49.49, 6.41, 50.82]),
    (("brussels",), [4.24, 50.76, 4.48, 50.92]),
    # Australia, before the United Kingdom (New South Wales).
    (("new south wales",), [140.99, -37.51, 153.64, -28.15]),
    (("western australia",), [112.9, -35.2, 129.0, -13.6]),
    (("queensland",), [137.99, -29.18, 153.55, -9.1]),
    (("tasmania",), [143.8, -43.7, 148.5, -39.5]),
    (("victoria, australia",), [140.96, -39.2, 150.0, -33.98]),
    (("australian capital territory",), [148.76, -35.92, 149.4, -35.12]),
    # United Kingdom.
    (("northern ireland",), [-8.18, 54.02, -5.43, 55.31]),
    (("england",), [-6.42, 49.86, 1.77, 55.81]),
    (("scotland",), [-8.65, 54.63, -0.72, 60.86]),
    (("wales",), [-5.35, 51.37, -2.65, 53.44]),
    # Spain.
    (("catalonia", "catalunya"), [0.16, 40.52, 3.33, 42.86]),
    (("andalusia", "andalucia"), [-7.52, 35.94, -1.63, 38.73]),
    (("basque country", "euskadi"), [-3.45, 42.47, -1.73, 43.46]),
    (("spain (peninsula",), [-9.3, 35.9, 4.33, 43.8]),
    # Brazil.
    (("sao paulo",), [-46.83, -24.01, -46.36, -23.36]),
    (("rio de janeiro",), [-43.8, -23.08, -43.1, -22.75]),
    (("distrito federal", "brasilia"), [-48.29, -16.05, -47.31, -15.5]),
    # North America.
    (("north america",), [-179.2, 14.5, -52.6, 83.1]),
    (("montreal",), [-73.98, 45.41, -73.47, 45.7]),
    (("toronto",), [-79.64, 43.58, -79.11, 43.86]),
    (("british columbia",), [-139.06, 48.3, -114.03, 60.0]),
    (("ontario",), [-95.16, 41.68, -74.34, 56.86]),
    (("quebec",), [-79.76, 44.99, -57.1, 62.59]),
    (("new york city",), [-74.26, 40.49, -73.7, 40.92]),
    (("new york state",), [-79.76, 40.49, -71.85, 45.02]),
    (("massachusetts",), [-73.51, 41.23, -69.92, 42.89]),
    (("los angeles county",), [-118.95, 32.8, -117.64, 34.82]),
    (("north carolina",), [-84.33, 33.84, -75.46, 36.59]),
    (("conterminous",), [-124.8, 24.4, -66.9, 49.4]),
    # New Zealand.
    (("wellington",), [174.6, -41.37, 174.9, -41.14]),
    (("auckland",), [174.4, -37.3, 175.3, -36.1]),
    (("christchurch",), [172.39, -43.65, 172.82, -43.38]),
    # South Africa.
    (("cape town",), [18.3, -34.36, 19.0, -33.47]),
    (("johannesburg",), [27.75, -26.53, 28.2, -25.9]),
    (("gauteng",), [27.14, -26.91, 28.84, -25.11]),
    (("western cape",), [17.75, -34.84, 24.22, -30.43]),
    # Other cities and regions.
    (("prague",), [14.22, 49.94, 14.71, 50.18]),
    (("copenhagen",), [12.45, 55.61, 12.73, 55.73]),
    (("helsinki metropolitan",), [24.5, 60.05, 25.26, 60.41]),
    (("espoo",), [24.5, 60.05, 24.86, 60.36]),
    (("tampere",), [23.5, 61.4, 24.1, 61.84]),
    (("lyon",), [4.69, 45.65, 5.06, 45.92]),
    (("lisbon",), [-9.23, 38.69, -9.09, 38.8]),
    (("mainland portugal",), [-9.53, 36.96, -6.19, 42.15]),
    (("ciudad autonoma de buenos aires",), [-58.53, -34.71, -58.33, -34.53]),
    (("buenos aires province",), [-63.4, -41.04, -56.66, -33.26]),
    (("cordoba province",), [-65.78, -35.0, -61.78, -29.5]),
    (("tel aviv",), [34.74, 31.98, 34.99, 32.2]),
    (("amsterdam",), [4.73, 52.28, 5.07, 52.43]),
    (("africa, latin america",), [-118.4, -56.0, 141.02, 37.6]),
]


def fold(text):
    """Lower case without accents: "Baden-Württemberg" → "baden-wurttemberg"."""
    t = unicodedata.normalize("NFKD", text.lower())
    return "".join(c for c in t if not unicodedata.combining(c))


def coverage_bbox(country, coverage):
    """The region the coverage text names, else the country's box."""
    text = fold(coverage).split("except")[0]
    for words, box in REGION_BBOX:
        if any(w in text for w in words):
            return box
    return COUNTRY_BBOX[country]


def inside(inner, outer, slack=0.5):
    return all((inner[0] >= outer[0] - slack, inner[1] >= outer[1] - slack, inner[2] <= outer[2] + slack, inner[3] <= outer[3] + slack))


def crs_list(entry, kind):
    """Coordinate systems, the request CRS first, without repeats."""
    out = []
    for c in entry.get("crs") or []:
        c = str(c).strip()
        if c and c not in out:
            out.append(c)
    if kind == "xyz" and not out:
        out = ["EPSG:3857"]
    return out


# Built-in presets already cover these endpoints.
SKIP_HOSTS = ("service.pdok.nl", "geo.api.vlaanderen.be/OMWRGBMRVL", "geo.api.vlaanderen.be/GRB/wfs", "tile.openstreetmap.org", "ows.terrestris.de/osm", "wms.gebco.net", "gibs.earthdata.nasa.gov/wms/epsg4326")


def param(qs, *names):
    for n in names:
        for k, v in qs.items():
            if k.lower() == n and v:
                return v[0]
    return ""


def main(path):
    data = json.load(open(path))["GIS2BIMserversRequests"]["webserverRequests"]
    out, seen = [], set()
    for e in data:
        country = e.get("country")
        if not country or country not in NAMES:
            continue
        kind = {"WMS": "wms", "WFS": "wfs", "WMTS_TMS": "xyz"}.get(e.get("service", ""))
        if not kind:
            continue
        url = e["serverrequestprefix"]
        if any(h in url for h in SKIP_HOSTS):
            continue
        layer = ""
        if kind == "xyz":
            if not all(t in url for t in ("{z}", "{x}", "{y}")):
                continue
            base = url
        else:
            parts = urlsplit(url)
            qs = parse_qs(parts.query, keep_blank_values=True)
            layer = param(qs, "layers") if kind == "wms" else param(qs, "typename", "typenames")
            # Keep vendor parameters (map=...) that select the service.
            keep = {k: v[0] for k, v in qs.items() if k.lower() in ("map",)}
            base = f"{parts.scheme}://{parts.netloc}{parts.path}"
            if keep:
                base += "?" + "&".join(f"{k}={v}" for k, v in keep.items())
        key = (kind, base, layer)
        if key in seen:
            continue
        seen.add(key)
        title = e.get("title", "")
        ident = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")[:80]
        nl, en = NAMES[country]
        lic = e.get("license", "")
        provider = e.get("provider", "")
        attribution = provider + (f" ({lic})" if lic and lic.lower() not in ("see terms",) else "")
        name = e.get("description") or title
        coverage = e.get("coverage", "")
        if coverage and coverage not in (en,) and coverage.lower() not in name.lower():
            name = f"{name} — {coverage}"
        bbox = coverage_bbox(country, coverage)
        if not inside(bbox, COUNTRY_BBOX[country]):
            print(f"warning: {ident}: coverage box {bbox} ({coverage}) lies outside {country}", file=sys.stderr)
        out.append({
            "id": "w-" + ident,
            "kind": kind,
            "name": name[:140],
            "region_nl": nl,
            "region_en": en,
            "country": country,
            "category": e.get("category", ""),
            "url": base,
            "default_layer": layer,
            "attribution": attribution[:200],
            "terms": e.get("terms", ""),
            "verified": e.get("verified", ""),
            "provider": provider,
            "license": lic,
            "coverage": coverage,
            "crs": crs_list(e, kind),
            "bbox": bbox,
        })
    order = {c: i for i, c in enumerate(["GLOBAL", "EU"])}
    out.sort(key=lambda s: (order.get(s["country"], 2), s["region_en"], s["category"], s["name"]))
    # One service per line: readable, and a changed service is a one-line diff.
    sys.stdout.write("[\n" + ",\n".join(json.dumps(s, ensure_ascii=False) for s in out) + "\n]\n")
    print(f"{len(out)} presets", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1])
