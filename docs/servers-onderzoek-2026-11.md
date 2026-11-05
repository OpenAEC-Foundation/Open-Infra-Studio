# New public geodata services for Open Infra Studio, checked 2026-11-02

`servers-nieuw.jsonl` has **170 services**. Each one answered live on 2026-11-02, needs no key, and is not already in `assets/servers/world.json` (no matching id or url+layer). Every line has the world.json fields plus `checked`, `test_bbox`/`test_crs` (the sample area; `null` where a sample file or tileset was fetched instead), `check_detail` (what came back), `reader_hint` (for new kinds) and, where useful, `sample_url`, `native_bounds` (from the LAS header), `file_prefix` and `flags` (`non-commercial`, `see terms`).

90 entries are GIS2BIM services (WCS, ArcGIS, downloads) that `tools/import_gis2bim_servers.py` leaves out because of their kind. They were re-checked today and carry `"source": "gis2bim:…"`. The other 80 are new. The PDOK BGT OGC API and AHN WCS were skipped because the plugin already uses them (bgt.rs, ahn.rs).

## Counts
- **Kind:** download 47 · arcgis-featureserver 41 · wcs 17 · arcgis-mapserver 13 · stac 10 · 3dtiles 9 · i3s 9 · wfs 5 · arcgis-imageserver 4 · cog 4 · wms 4 · xyz 3 · ept 2 · copc 1 · laz 1
- **Category:** elevation 62 · buildings3d 29 · buildings 15 · cadastre 11 · zoning 9 · roads 8 · utilities 6 · structures 5 · basemap 4 · protected 4 · rail 4 · admin 3 · aerial 3 · water 3 · environment 2 · landcover 1 · subsurface 1
- **Country:** DE 18 · US 17 · AU 13 · NZ 12 · SG 12 · GLOBAL 11 · ZA 9 · CA 8 · CH 8 · GB 8 · JP 8 · IE 6 · FI 5 · LU 4 · NL 4 · BE 3 · BR 3 · IT 3 · CL 2 · ES 2 · TH 2 · AFRICA 1 · CZ 1 · DK 1 · EU 1 · FR 1 · HK 1 · ID 1 · LT 1 · NO 1 · PL 1 · SI 1 · SK 1

## What each check did
- **wms / xyz:** GetCapabilities parsed and the layer is listed. A 256×256 GetMap or tile came back as an image that is not one flat colour (checked with Pillow).
- **wfs:** GetCapabilities parsed and GetFeature returned at least one feature for the test area.
- **wcs:** GetCapabilities lists the coverage, and GetCoverage returned a GeoTIFF.
- **arcgis-featureserver:** the layer's JSON loaded and `/query` returned at least one feature for the test area.
- **arcgis-mapserver:** `/export` returned an image that is not empty.
- **arcgis-imageserver:** `/exportImage` returned a Float32 TIFF.
- **3dtiles:** `tileset.json` has `asset.version` and a `root`, and the first content file came back (the bytes were checked to be glTF, b3dm or a nested tileset).
- **i3s:** the SceneServer JSON loaded and node page 0 returned nodes.
- **copc / laz:** the LAS header was read with a Range request. It gave the LAS version, the COPC record (where present), the point count and the bounds.
- **ept:** `ept.json` loaded and the root node `0-0-0-0.laz` starts with `LASF`.
- **stac:** the items in the test area were listed, and a Range request on the first asset returned TIFF, ZIP or LAS bytes.
- **cog:** a Range request returned a TIFF header.
- **download:** the index loaded, and a Range request on a sample file returned ZIP, LAS, TIFF, XML, JSON, Parquet or gzip bytes.

## Which existing code can read what
- **COPC:** `w-fr-ign-lidarhd-copc` (IGN LiDAR HD, the whole of France, keyless COPC 1.4). `pointcloud.rs` with copc-rs can read it once the AHN base URL and tile naming become parameters. This is the best find.
- **LAZ that is not COPC:** NRW 3D-Messdaten, Bavaria, Brandenburg (zip), Slovenia ARSO, USGS TNM, Luxembourg 2024 (zip) and swissSURFACE3D (las.zip). laz-rs can decode these, but there is no octree, so a whole tile has to be read.
- **EPT:** USGS 3DEP on AWS (index plus Los Angeles) and NOAA Digital Coast. These use the same LAZ nodes and need a small EPT hierarchy reader.
- **CityGML / CityJSON downloads:** nine German LoD2 sources (BB, BE, BY, HH, MV, NI, NW, RP, ST), Zürich, Brussels UrbIS, Luxembourg 2023, NYC, Montréal, PLATEAU CityGML and swissBUILDINGS3D (STAC). These go to `citygml.rs` / `cityjson.rs` after unzipping.
- **ArcGIS tile caches:** the Lithuania geoportal, Auckland aerial 2024–25 and contours, and GISTDA land use are tiled MapServers, which `arcgis.rs` already reads. The other MapServers are dynamic (`/export`) only.
- **ArcGIS feature layers (41):** `/query?f=geojson` output can be read by `vector.rs::parse_geojson`. This needs a small client that handles paging.
- **WCS (17):** these are DEM GeoTIFFs. The `ahn.rs` WCS code can be made generic (coverage id, CRS and format as parameters).
- **GeoJSON downloads:** the twelve Singapore datasets and geoBoundaries can be read by `parse_geojson` (data is in WGS84).
- **No reader yet:** 3D Tiles (glTF/b3dm: swissBUILDINGS3D, 3DBAG, PDOK 3D Basisvoorziening, Helsinki, PLATEAU) and I3S (Esri OSM/Open 3D Buildings, HK 3D-BIT00, Ottawa, and point-cloud scene layers for Northern Ireland, Christchurch, Geneva, Cantabria and Bratislava). COG/STAC also has no reader yet: swisstopo, LINZ, NRCan, Copernicus DEM and Luxembourg. One GeoTIFF range reader would cover all of these.

## Notable finds
- **Point clouds, keyless:** IGN LiDAR HD COPC (FR) · USGS 3DEP EPT (US) · NOAA coastal EPT (US) · USGS TNM LAZ API (US) · swissSURFACE3D (CH) · NRW, Bavaria, Brandenburg LAZ (DE) · ARSO LAZ (SI) · ACT LiDAR 2024 (LU) · I3S point clouds for Northern Ireland, Christchurch, Geneva, Cantabria and Bratislava.
- **Elevation:** LINZ national 1 m DEM and DSM as COG (NZ) · NRCan HRDEM, HRDEM mosaic and MRDEM through STAC (CA) · swissALTI3D, swissSURFACE3D raster and SWISSIMAGE 10 cm through STAC (CH) · Copernicus GLO-30 and GLO-90 COG (worldwide) · USGS 3DEP ImageServer, ČÚZK DMR5G ImageServer, NSW 5 m ImageServer · GSI DEM10B and DEM1A PNG tiles (JP) · INGV TINITALY WMS (IT) · Luxembourg 2024 DTM and DSM GeoTIFF · re-checked DTM WCS for BE-VL, five German states, ES, GB-EA, IT, NL-RWS, NO, PL, US, Australia (GA) and Africa (DE Africa).
- **3D city models:** PLATEAU catalogue API, about 7,800 datasets in 307 cities. Besides buildings it includes roads LoD3, bridges, underground buildings and **underground utilities (manholes)**, all as 3D Tiles. Also swissBUILDINGS3D 3D Tiles, 3DBAG LoD2.2 3D Tiles, PDOK 3D Basisvoorziening buildings and terrain 3D Tiles, Helsinki 3D Tiles, and Esri OSM 3D Buildings and Open 3D Buildings (I3S, worldwide).
- **Utilities:** Landgate SLIP Western Australia has Water Corporation sewer gravity pipes, water pipes and drainage pipes (ArcGIS query, plus a WFS). Also HIFLD US transmission lines and PLATEAU manholes in Nagaoka.
- **Cadastre:** SLA Singapore land parcels and airspace parcels (whole island, open) · NSW lots · Helsinki properties · Ireland freehold parcels · Cape Town and Western Cape erven · NRCan CLSS parcels on Canada Lands.
- **Global downloads:** Overture buildings (GeoParquet), Microsoft and Google building footprints, TUM GlobalBuildingAtlas LoD1 (**CC BY-NC**, flagged non-commercial), and geoBoundaries.

## Singapore
- **Works without a key:** the data.gov.sg download API `api-open.data.gov.sg/v1/public/api/datasets/{id}/poll-download`. It returns a pre-signed S3 link to GeoJSON (WGS84) that is valid for one hour. The rate limit is about one request per 10 s without a key; a key raises it. Twelve datasets checked:
  - SLA cadastral land parcels, all lots (145 MB)
  - SLA cadastral airspace parcels
  - SLA vertical control (benchmarks)
  - SLA National Map Line (320 MB) and National Map Polygon (topography)
  - URA Master Plan 2019 building, land use and rail line layers
  - HDB existing buildings
  - LTA bridges/flyovers, retaining walls and kerb lines

  The licence is the Singapore Open Data Licence v1.0. Other usable collections not added yet (each has a dataset id in the collection listing): LTA traffic signs and lane markings, URA building-height controls, and Master Plan roads.
- **Not public:** the OneMap3D 3D Tiles (`/omapi/tilesets/sg_noterrain_tiles/tileset.json`) return `Direct download is not permitted`, so they were not used. No public national LiDAR, DEM or CityGML service was found; SLA's 3D national map and Virtual Singapore are not open data. The OneMap search API now needs a token, but OneMap XYZ tiles still work (three are already in world.json). The NUS Urban Analytics Lab HDB CityJSON is a research dataset, not a service, and was not checked.
- **Coordinates:** the GeoJSON is in WGS84. For design work transform it to SVY21 (EPSG:3414), with heights in SHD.

## Services that need a key (not in the jsonl)
| Service | What it offers | Terms constraint |
|---|---|---|
| Google Photorealistic 3D Tiles (Map Tiles API) | Global 3D mesh | API key and billing. No caching beyond the session, no offline use or extraction into CAD/GIS, Google attribution required. |
| Cesium ion (World Terrain, OSM Buildings, Bing) | Terrain and 3D Tiles | Access token. The free Community tier is for non-commercial use; commercial use needs a paid plan; content-specific terms apply. |
| Mapbox (Terrain-RGB/DEM, satellite, vector) | Tiles | Token. Restricts caching and offline storage; attribution required. |
| MapTiler | Terrain, satellite | Key. The free tier is non-commercial; attribution required. |
| OpenTopography API | Global DEMs, some LiDAR | Free API key. Rate limits; some datasets are academic-only. |
| Microsoft Planetary Computer | 3DEP COPC, Copernicus DEM, ESA WorldCover | Anonymous SAS token from the token endpoint, so keyless but token-gated. Licence per dataset. |
| Denmark Dataforsyningen / Datafordeler | DHM terrain and point cloud, matrikel, BBR | Free token after registration. Open data, commercial use allowed. |
| Finland NLS (MML) | Laser scanning, DEM 2 m, cadastre, WMTS | Free API key. CC BY 4.0. |
| Sweden Lantmäteriet (Geotorget) | Laser data, height model, topography | Account or key. Mostly CC0 open data. |
| Korea VWorld / NGII | Base maps, 3D, DEM | API key tied to a registered domain. Korean terms; access from abroad is limited. |
| LINZ Data Service / LINZ Basemaps | NZ parcels, building outlines, aerials | Free API key. CC BY 4.0. Only the S3 elevation STAC is keyless. |
| Ordnance Survey Data Hub (Maps, Features, Names) | GB mapping | API key. Free tier with premium limits. The OpenData downloads API is keyless and is included. |
| Esri ArcGIS Location Platform (basemaps, elevation) | Basemaps, elevation | API key and Esri terms. Note that Terrain3D and the OSM 3D layers answer without a key but carry Esri terms (flagged). |
| IGN Géoplateforme private WFS | LiDAR HD tile index (`IGNF_LIDAR-HD_TA`) | apikey (catalogue key). The Atom download of the COPC tiles itself is keyless. |
| Poland GUGiK LAZ / 3D building index services | Point-cloud and 3D-building tile indexes | Returned HTTP 401 (login now required). |

## Checked but left out
- **AT** Tirol Flächenwidmung MapServer: timed out from here (perhaps geo-blocked).
- **PE** MINAM protected areas: blocked by a WAF/bot challenge (HTTP 403, HTML). Not bypassed.
- **US** FEMA USA Structures: timed out twice.
- **DE-BW** LGL LoD2: HTTP 403.
- **IE** GSI LiDAR directory: HTTP 403.
- **CA** NRCan auto-extracted buildings: the file returned HTTP 403.
- **CA** NRCan elevation WCS: the coverage name was not accepted; the STAC is used instead.
- **AU-WA** Western Power underground/overhead carriers: 0 features in the test areas (probably restricted).
- **PL** KIEG cadastral WMS: blank images in every city tested (the service may filter clients).
- **LU** LiDAR 2019: replaced by the 2024 entries.
- **Index reachable, sample file not checked (left out):** Estonia Maa- ja Ruumiamet 3D LoD2 and DTM download pages (file links are built by JavaScript), Saxony LoD2 and DGM1 (Nextcloud share), and the Wales LiDAR tile catalogue.
- **Not checked:** Hessen and Bremen LoD2 (GIS2BIM's index is a shop or CSW, not a file list), and the UK EA DEFRA survey download (it is a POST API).

## Notes for the integrator
- `crs` for 3D Tiles is EPSG:4978 (ECEF). For downloads it is the CRS of the files.
- `bbox` comes from service metadata (WMS/WFS/STAC/I3S extents, 3D Tiles root region) when that falls inside the country; otherwise from the importer's `coverage_bbox()`.
- The 3DBAG 3D Tiles path contains the release (`v20250903`); 3DBAG versions without public tiles return HTTP 403.
- The PLATEAU asset URLs change with each data year. Read them from the catalogue API rather than hard-coding them.
