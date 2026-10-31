# Open Infra outside the Netherlands, GIS files and the vehicle library

What the branch `internationaal-voertuigen` adds. Every part works anywhere in the world; the Dutch parts (NLCS, BGT, KLIC, AHN, GWSW) stay as they were.

## Interface language

Dutch or English. The choice is `language` in `<config>/OpenCADStudio/open-infra/settings.json` (`"nl"` or `"en"`). Without that setting, the system locale decides: `nl*` gives Dutch, anything else English.

The workbench window has a switch under the navigation. It takes effect at the next start of Open CAD Studio, because the ribbon is built when the plugin loads.

## Web services in 52 countries

The server lists on the raster and WFS pages hold 600 extra presets:
- 426 WMS, 131 WFS and 43 XYZ tile services;
- from national, regional and city geoportals in 52 countries, plus pan-European and worldwide sources.

Each one answered GetCapabilities and a sample request when the list was checked, and none needs an API key.

A filter at the top of the list finds a country, region or name.

The list comes from the checked web service catalogue of GIS2BIM (branch `worldwide-servers`). `tools/import_gis2bim_servers.py` rebuilds `assets/servers/world.json` from it.

## Heights everywhere

Spot heights, contour lines and length profiles work at any location:

| Where | Source | Grid | Reference |
|---|---|---|---|
| Netherlands | AHN4 (PDOK WCS) | RD, 0.5 m | NAP |
| Elsewhere | [Terrain Tiles](https://registry.opendata.aws/terrain-tiles/) (AWS Open Data) | the site's CRS (UTM when the site is in degrees), 5 m or coarser | about mean sea level |

The Terrain Tiles combine SRTM, GMTED and ETOPO1 with finer national models where those were open, such as USGS 3DEP and Austria. They need no API key.

Outside the Netherlands the contour interval is at least 1 m: finer intervals on 30 m data only draw noise. The AHN point cloud stays Netherlands only.

`gis::terrain::Source` chooses the source and gives the grid, the profile stations, the label, the attribution and the height reference.

## GIS files (issue #7)

**GIS-bestand / GIS file** in the ribbon opens a file dialog, then the workbench page. These formats are read:

| Format | CRS from |
|---|---|
| GeoJSON | the `crs` member, otherwise WGS 84 (RFC 7946) |
| Shapefile (`.shp` + `.dbf` + `.prj`) | the `.prj`: EPSG authority or a known ESRI name (RD_New, UTM zones, ...) |
| GeoPackage | `gpkg_spatial_ref_sys`; a layer choice when there are several |
| KML, KMZ | WGS 84 |
| GPX | WGS 84 (waypoints, routes, tracks) |
| CSV / TXT | `lon/lat` columns → WGS 84; `x/y`, `easting/northing`, `x_rd/y_rd` → the site's CRS (check it) |

Options:
- each feature's attributes go on its entities as XDATA `OPENINFRA` (up to 40);
- one layer per value of an attribute;
- labels from an attribute;
- text height and colour;
- clipping to the import area.

Commands: `GIS_BESTAND | GIS_FILE [path]` (window) and `GIS_BESTAND_IMPORT | GIS_FILE_IMPORT <path> [EPSG:code]` (straight in).

## GWSW sewers (issue #6)

`GIS_Q_GWSW` fetches the GWSW collections from the PDOK OGC API (`beheer-stedelijk-watersystemen-gwsw`, CC0): pipes, connection pipes, manholes, structures, pumps and connection points.

Each object goes on the most specific NLCS RI object, chosen in this order:
1. the system (GWA, HWA, DWA, ITR, PERSLEIDING, DRAINAGE), from the GWSW type or the system type;
2. the material;
3. the diameter.

An example layer is `B-XX-RI-GWA_RIOOLLEIDING_BETON_600-G`.

Manholes carry their name on the PUTNUMMER object, and every object carries its GWSW attributes as XDATA. Not every municipality publishes its network.

## Updating the BGT (issue #11)

`GIS_BGT_BIJWERKEN | GIS_BGT_UPDATE`, also in the "BGT · BRK · KLIC" menu, brings the BGT in the drawing up to date instead of importing it again. It fetches the current BGT for the collections that are in the drawing and compares object by object on `lokaal_id` from the object data:

| Case | What happens |
|---|---|
| New object | drawn |
| Changed object (other geometry, to the millimetre) | its entities are removed and it is drawn again |
| Object no longer in the BGT | moved to `OI-BGT-VERVALLEN` / `OI-BGT-REMOVED`, never deleted |
| Drawing work without Open Infra object data | left alone |

The message gives the counts. Cadastre and KLIC can follow the same way.

## GEODATA (issue #8, reading)

A drawing from other CAD programs with a geographic location (GEOGRAPHICLOCATION, GEOMAP) and no Open Infra site takes its site from the GEODATA object:
- **CRS:** from the MapGuide XML or the WKT;
- **absolute or relative:** absolute when the design point equals the reference point.

Writing GEODATA needs Open CAD Studio to offer it: the plugin API v2026.42 creates no objects other than XRecords.

## RD New with NSGI's 2018 procedure (issue #5)

Every transformation to or from RD New (EPSG:28992) now follows NSGI's ETRS89 ↔ RD procedure of 2018, implementation variant 1 (`src/gis/rdnap.rs`):
1. a seven-parameter step at a fixed height, with the parameters for each direction;
2. the `rdcorr2018` correction grid;
3. the RD double stereographic projection.

The `nlgeo2018` quasi-geoid gives NAP heights.

The grids are not part of the plugin:
- The first time a drawing works in RD, Open Infra downloads NSGI's official package once (about 7 MB) and keeps the two unchanged grid files with a NOTICE in `<config>/OpenCADStudio/open-infra/rdnap2018/`.
- Until the download is done, the correction is 0, as the procedure prescribes outside the grid. That gives the seven-parameter result, at most about 0.25 m off.

Checked against NSGI's self-validation set (`Z001_ETRS89andRDNAP.txt`, 10,000 points) with `NSGI_GRIDS=… NSGI_Z001=… cargo test --lib rdnap -- --ignored`:
- **Inside the grid area (Netherlands + EEZ, 7,959 points):** all within 0.0010 m and 1e-8°, in both directions and for NAP.
- **Outside the grid area (2,041 points):** the forward direction is all within tolerance. The inverse misses 1e-8° for 239 points more than about 750 km from Amersfoort, as NSGI's documentation says it will.

Open points:
- **Licence:** NSGI's documentation calls the grids CC BY 4.0, while `license.txt` in the package says CC BY-ND 4.0. That is why the grids are downloaded rather than shipped.
- **The name RDNAPTRANS™** may only be used after NSGI's written consent, which needs 100 % from their validation service in both directions. This branch does not use the name.

## Vehicle library

**Voertuigen / Vehicles** in the ribbon opens the library:
- search by name or standard;
- filter by category and region;
- see the vehicle in plan or side view;
- insert it into the drawing.

Each vehicle becomes a block, `OI_<ID>_TOP` or `OI_<ID>_SIDE`:
- **block geometry:** on layer 0, ByBlock;
- **inserts:** on `OI-VOERTUIGEN` / `OI-VEHICLES`, with the dimensions, standard and source as XDATA;
- **base point:** the centre of the front; the vehicle drives along +X.

| Command | What |
|---|---|
| `GIS_VOERTUIG \| GIS_VEHICLE <id> [ZIJ\|SIDE]` | pick the front, then the driving direction (Enter: east). A side view picked to the left is mirrored. |
| `GIS_VOERTUIGOVERZICHT \| GIS_VEHICLE_CATALOG [category] [ZIJ\|SIDE]` | a catalogue sheet: rows per category, with names and dimensions |
| `GIS_VOERTUIGEN \| GIS_VEHICLES` | the window |

The data is `assets/library/vehicles.json`. Each vehicle has:
- its units (tractor and semitrailer, bus and rear section, ...), each with its length and width;
- per unit, its axles with their positions, track, twin tyres and steering;
- the coupling points between units;
- the standard and the source page;
- which values were estimated.

## Swept paths

`GIS_VEEGPAD | GIS_SWEPT_PATH <id> [interval]` drives a vehicle along a selected polyline, or along picked points ended with Enter:
- the steer axle follows the line;
- each rear axle and each trailer follows as a tractrix (the low-speed model of swept-path software).

Drawn:

| Layer | What |
|---|---|
| `OI-VEEGPAD` / `OI-SWEPT-PATH` | the traces of every unit's body corners |
| `OI-VEEGPAD-WIELEN` / `-WHEELS` | the outer wheel edges |
| `OI-VEEGPAD-VOERTUIG` / `-VEHICLE` | the vehicle every interval (default 10 m); each unit is its own block (`OI_<ID>_U<n>_TOP`), so they turn apart |

The message gives the largest steering angle and the smallest outer radius. It warns when the path is tighter than the vehicle's design turning radius.

## Tests

- `cargo test --lib` covers the engine:
  - vehicle data and drawings, swept paths;
  - Terrain Tiles decoding;
  - every file format, using `tests/data/gisbestanden`, made by `maak.py`;
  - GWSW → NLCS mapping;
  - GEODATA definitions.
- `python3 tests/e2e/bibliotheek_e2e.py` runs its own Open CAD Studio with an isolated configuration folder. In Zürich it imports the six file formats, inserts vehicles and a catalogue, and draws contour lines and spot heights.
