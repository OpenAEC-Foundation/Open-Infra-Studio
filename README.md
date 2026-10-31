# Open Infra Studio — GIS, NLCS and civil design for Open CAD Studio

[![Release](https://img.shields.io/github/v/release/OpenAEC-Foundation/Open-Infra-Studio?label=release)](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/releases/latest)
[![Open CAD Studio](https://img.shields.io/badge/Open%20CAD%20Studio-v2026.43-2196dc)](https://github.com/HakanSeven12/OpenCADStudio)
[![Plugin API](https://img.shields.io/badge/plugin%20API-13-6db7ed)](https://github.com/HakanSeven12/OpenCADStudio/tree/master/crates/ocs_plugin_api)
[![NLCS](https://img.shields.io/badge/NLCS-5.02-98c379)](https://www.nlcs.nl)
[![Licence](https://img.shields.io/badge/licence-GPL--3.0--only-lightgrey)](LICENSE)

Open, scriptable GIS and civil design inside CAD. Open Infra Studio is a plugin for [Open CAD Studio](https://github.com/HakanSeven12/OpenCADStudio) for infrastructure and civil engineering, anywhere in the world. It is a free, open-source (GPL) alternative to commercial civil-CAD and desktop-GIS suites: GIS context and civil design in one drawing, driven from the ribbon, from scripts or by an AI agent.

**Worldwide:**

- **GIS workbench**, modelled on the FreeCAD GIS(2BIM) workbench. Set the project location anywhere in the world, in a built-in coordinate system, automatic UTM or any EPSG code. Then pull aerial photos, WMS/WFS/XYZ layers and OpenStreetMap straight into the DWG.
- **Civil design (second ribbon tab "Civiel")**, after Hakan Seven's FreeCAD Road workbench. It covers:
  - survey points and LandXML;
  - terrain (TIN, contours, height analysis);
  - alignments with stationing;
  - length profiles, design profiles and cross sections;
  - corridors with daylight, earthworks and a mass balance.
- **CityGML and CityJSON import**: 3D city models from anywhere (CityGML 1.0–3.0, CityJSON 1.x/2.0 and CityJSONSeq), reprojected to the project location and streamed object by object, so files of a whole city district load in seconds.
- **IFC 4.3 export**: the whole drawing as IFC4X3_ADD2 for BIM and infra software. Every object gets its IFC class (terrain, buildings, pavements, cables and pipes, boreholes, earthworks …), axes become IfcAlignments with horizontal and vertical layout, georeferenced, with object data as property sets.
- **Scriptable and agent-ready**: every command takes arguments and reports back as JSON. An MCP server ([`mcp/`](mcp/README.md)) lets an AI agent build whole models.

**Plus deep Dutch integration:**

- **Dutch base data to NLCS**: BGT, cadastral map (BRK) and KLIC deliveries on NLCS layers, with object data and a finished drawing sheet. Design changes go back to the BGT as a BGT mutation (IMGeo-GML).
- **AHN heights**: spot heights, contour lines and length profiles from AHN4, and the AHN5 point cloud attached in 3D.
- **3D city and landscape model**: the AHN terrain coloured by the aerial photo, 3D BAG buildings (LoD 1.2–2.2), the whole BGT in 3D, KLIC cables and pipes in 3D under a see-through ground, and the subsurface: BRO cone penetration tests and boreholes as columns of soil layers.

**Regional profiles:** Netherlands, United States and International. Interface language, application defaults and drawing jurisdiction are separate choices under **Localisation** (`GIS_LOCALISATION`). The US profile provides USGS topography/3DEP, Census boundaries and NRCS soil-map shortcuts, plus a regional service filter. Existing civil/library tools still produce NLCS layers; choosing the US profile does not certify US CAD-standard compliance. Use a metric project CRS; drawing units may be metres, international feet or historical US survey feet. See [localisations and international research](docs/localisations.md) for sources, tested services, geodetic requirements and current limits.

[![Open Infra Studio in one minute: watch the video](docs/video/open-infra-poster.jpg)](docs/video/open-infra.mp4)

<sub>Open Infra Studio in one minute (MP4, 64 s): the application and its 3D model, 2D drawings to NLCS, map services in Paris, Cologne and Zurich, historical maps of the Netherlands, KLIC and CPTs under a see-through ground. Click the image to play.</sub>

![Example: existing situation around the Grote Kerk in Dordrecht, A1 1:500](examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.png)

<sub>[Example drawing](examples/dordrecht-nlcs): BGT, cadastre and a KLIC delivery around the Grote Kerk in Dordrecht. A1 at 1:500, made with five commands. Available as DWG, PDF and PNG.</sub>

## What makes Open Infra Studio different

Features you will not easily find together in one tool, let alone a free one:

1. **GIS and civil design in one CAD drawing.** No export–import between a GIS program and a CAD program: map services, open data, terrain, alignments, corridors and drawing sheets all live in the DWG.
2. **600+ map services in 45 countries, searchable.** A built-in catalogue of checked public WMS, WFS and tile services without API keys. Search by place or theme in several languages (*cadastre*, *kadaster*, *Flurstücke*, *cadastro*), see what covers the place on the map, and load it reprojected into the project's coordinate system.
3. **Any coordinate system.** Built-in systems, automatic UTM worldwide, any EPSG code fetched from epsg.io once; WMS, WFS and tile data are reprojected on the fly.
4. **Open 3D city models from anywhere.** CityGML 1.0–3.0 and CityJSON 1.x/2.0, streamed object by object and filtered to the project area while reading: a 97 MB city tile loads in under two seconds.
5. **Underground in 3D.** KLIC cables and pipes as tubes at their (assumed) depth, BRO cone penetration tests and boreholes as soil columns classified with Robertson (2010), and a see-through ground to look at it all from below.
6. **History at the click of a button.** Every Dutch topographic map since 1815 (Topotijdreis) and the cadastral maps of 1832 (HisGIS) as underlays, next to Natura 2000, the geomorphological map and the soil map.
7. **A full 3D city model in one command.** Terrain coloured by the aerial photo, every building from 3D BAG (roof colours from the photo, walls by year of construction), the whole BGT in 3D, trees, water at its level.
8. **Drawing-standard aware.** Every object lands on its NLCS layer with its source attributes as object data; NLCS check and repair, quantities per object, and design changes back to the BGT as an IMGeo-GML mutation.
9. **Civil design after the FreeCAD Road workbench.** Survey points, LandXML, TIN, alignments with stationing, length, design and cross sections, corridors with daylight and a mass balance.
10. **Agent-ready.** Every command takes arguments and answers in JSON; an MCP server exposes 39 tools, so an AI assistant can build whole models.
11. **International first, Dutch when you need it.** English and Dutch interface following Open CAD Studio's language; the Dutch modules sit in one group and can be switched off.
12. **Open BIM out.** One command writes the drawing as IFC 4.3, with alignments, the georeference and the object data: ready for any IFC 4.3 viewer or BIM tool.
13. **Free and open source (GPL-3.0).** No licence fees, no lock-in, built on the open-source Open CAD Studio.

## At a glance

| | |
|---|---|
| **Coordinate systems** | 14 built in, automatic UTM worldwide, any EPSG code via epsg.io |
| **Service catalogue** | 600 checked public services (426 WMS, 131 WFS, 43 XYZ) in 45 countries, Europe, Africa and worldwide, searchable by place, theme and coordinate system, plus built-in presets and your own servers |
| **3D city models** | CityGML 1.0–3.0 and CityJSON 1.x/2.0 import, reprojected and filtered to the project area while reading |
| **Historical maps** | Topotijdreis (every Dutch topographic map 1815–2014) and HisGIS 1832 cadastral maps, via map tile caches and XYZ tiles |
| **Map backgrounds** | 5: OpenStreetMap, PDOK BRT (standard and grey), PDOK aerial photo, Sentinel-2 |
| **One-click imports** | 11 OpenStreetMap themes, 11 PDOK datasets, 11 quick buttons |
| **Civil design** | survey points (PNEZD/PENZD/CSV), LandXML 1.x, TIN, alignments, length, design and cross sections, corridors with 4 assemblies, earthworks with a mass balance |
| **File formats** | 24 file formats to open (vector GIS, rasters and height grids, point clouds, survey points, LandXML, CityGML/CityJSON, KLIC), 9 kinds of output files including IFC 4.3, 12 web service and data protocols: see [File formats](#file-formats) |
| **Automation** | arguments and JSON results for every `CIV_` command and the GIS data commands; MCP server with 39 tools |
| **Commands** | 35 `GIS_*` commands for the ribbon, scripts and automation |
| **Code** | about 29,000 lines of Rust, 120 unit tests; Linux, Windows and macOS builds |

**Plus, for the Netherlands:**

| | |
|---|---|
| **NLCS 5.02 built in** | 3,659 objects in 32 main groups, 10 disciplines, 4 drawing statuses, 256 colours |
| **NLCS linetypes** | 543 (107 plain dash patterns, 436 with symbols or text) |
| **BGT → NLCS** | 719 rules from the official NLCS-BGT 5.0 mapping; designs go back to the BGT as IMGeo-GML 2.1.1 |
| **BGT** | 42 PDOK collections in 11 groups, current objects only |
| **KLIC** | 13 IMKL 2.0 themes: electricity (LS/MS/HS), data, gas (LD/HD), water, district heating, sewer, chemicals |
| **AHN** | AHN4 DTM/DSM at 0.5 m (PDOK WCS); AHN4/AHN5 point clouds (COPC), only the octree nodes of the project area are downloaded |
| **3D BAG** | every building in LoD 1.2, 1.3 or 2.2, with BAG id, year of construction and height as object data |
| **KLIC in 3D** | cables and pipes as tubes at their (assumed) depth under a see-through ground |
| **BRO subsurface** | cone penetration tests and boreholes as 3D soil columns (Robertson 2010 classification) |

## In Open CAD Studio

![Open CAD Studio with the Open Infra ribbon tab and the example drawing](docs/images/open-cad-studio-open-infra.png)

The ribbon tab **"Open Infra — GIS"** puts the worldwide tools first. Everything that only works in the Netherlands sits in one compact group of menus, and can be switched off (see [Language and Dutch modules](#language-and-dutch-modules)). Labels follow the interface language; the Dutch label is given in brackets.

| Group | Button | What it does |
|---|---|---|
| Location | **Geolocation** (Geo-locatie) | Set the project location. Click it on the map, or search an address, street, place, parcel, `lat, lon` or RD `x y`; worldwide via Nominatim, in the Netherlands also PDOK. Choose the coordinate system (built in, automatic UTM, or any EPSG code via epsg.io), draw or type the bounding box, and choose absolute or relative coordinates. The location is stored in the drawing (NOD XRecord `OPENINFRA/SITE`). |
| | **Area** · **Point info** · **Coordinates** | Pick the import area in the drawing, read a point's coordinates, convert coordinates. |
| Maps & data | **Services worldwide** | The service browser: search 600+ WMS, WFS and tile services in 45 countries, see what covers a place, and load a layer (see below). |
| | **WMS / TMS** · **WFS** | Any server: browse its layers or feature types, **Test** on the map, then import as image underlay or as vectors with attributes. Historical maps too: Topotijdreis (every Dutch topographic map 1815–2014, an ArcGIS tile cache in RD) and the HisGIS cadastral maps of 1832. |
| | **OpenStreetMap** | Worldwide themes: buildings, roads, rail, water, civil structures, land use, cables and pipelines, fences, house numbers. |
| | **Quick (world)** ▾ | One click: OpenStreetMap map, Sentinel-2 satellite image, OpenStreetMap vectors. |
| | **Open file** | GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX, GML, CSV, GeoTIFF, ASC, JPG/PNG with world file, LAS/LAZ. |
| Elevation | **Spot heights** · **Contours** · **Profile** | Heights from AHN4 in the Netherlands and from Terrain Tiles elsewhere: spot heights in a grid, labelled contour lines, and length profiles with a band of distances and heights. |
| Netherlands | **Dutch base data** ▾ | BGT · BRK · KLIC window, BGT import and update, cadastre, KLIC delivery, GWSW sewers, BRO CPTs and boreholes, the AHN window and point cloud, the PDOK catalogue. |
| | **Dutch maps** ▾ · **Dutch vector data** ▾ | One click: aerial photo, cadastral map, AHN, Natura 2000, geomorphological map, soil map, HisGIS 1832, Topotijdreis; BAG, parcels, NWB roads and hectometre posts, ProRail tracks. |
| | **NLCS** ▾ | NLCS check and repair, quantities, bill of quantities, BGT mutation. |
| Output | **Drawing sheet** (Tekenblad) · Servers | Drawing sheet around the import area: frame, title block, legend of the layers used, north arrow, scale bar and coordinate crosses. Manage presets and your own WMS/WFS/XYZ servers. |
| Library | **Infrastructure** · Vehicles · Catalogue · Swept path | 94 parametric infrastructure items on NLCS layers, checked connectors and regeneration of connected objects; design vehicles and swept paths. |

### Infrastructure library and drawing skills

The infrastructure library covers paving, kerbs, sewers and fittings, drainage, utilities, lighting, green spaces, street furniture, signs, annotations and configurable sections. Connections check system, diameter, direction and socket occupancy. Kerbs can follow a surface edge; dependent objects are regenerated when parameters change. The geometry is custom and schematic, classified using NLCS 5.0.2.

See the [library guide](docs/infra-bibliotheek.md), [complete item inventory](docs/infra-objecten.md) and [DWG/DXF examples](examples/infra-bibliotheek). Six repository skills in [`.agents/skills`](.agents/skills) support civil drawing workflows. MCP provides `infra_catalog`, `infra_place`, `infra_inspect` and `infra_update`.

### Language and Dutch modules

- **Language:** English or Dutch. Open Infra Studio follows Open CAD Studio's interface language (`language` in its `settings.json`; with "system" the system locale, as the host does) and falls back to English. A choice made in the workbench (NL / EN in the navigation column) overrides that.
- **Dutch modules on/off:** next to the language choice. Off removes the Netherlands group from the GIS ribbon and the Dutch 3D menu from the Civiel ribbon, for users elsewhere. Both are stored in `<config>/OpenCADStudio/open-infra/settings.json` (`language`, `netherlands`) and take effect after a restart.
- **Icons:** the workbench windows have their own icon in the taskbar (a map pin for GIS, a profile for Civiel).

## Service browser: WMS, WFS and tiles worldwide

The page **Services wereldwijd / Services worldwide** (ribbon button in *Kaarten & data*, command `GIS_SERVICES`) searches one catalogue:
- the 19 built-in presets;
- 600 checked public services without an API key from GIS2BIM's worldwide list ([`assets/servers/world.json`](assets/servers/world.json));
- your own servers.

What it offers:
- **Search** in free text, in Dutch or English, over name, provider, country, region and theme. Theme words work in several languages: *kadaster*, *percelen*, *cadastre*, *parcels*, *Flurstücke* and *cadastro* all find cadastral services; *luchtfoto* / *aerial* / *orthophoto*, *hoogte* / *elevation* / *DEM* and *bodem* / *geology* work the same way. Place names work too, also in other languages (*Bayern*, *Vlaanderen*, *Wenen*).
- **Filter chips** for the 17 categories and for the type (WMS, WFS, XYZ tiles).
- **Op deze locatie / At this location** keeps the services whose coverage contains the centre of the workbench map, or the project location. Pan and zoom the map to a place, or click it, to see what is available there. Local services come first, then national ones, then worldwide ones. Open CAD Studio does not share its view with plugins, so the workbench map stands in for it.
- **Results** are grouped by country. Each shows the name, provider, type, category, coordinate systems (the request CRS first) and licence.
- **A chosen service** fetches its GetCapabilities in the background. The page then lists the layers or feature types, each with its WGS 84 extent and its coordinate systems, and says whether the location lies inside. The map shows the service's coverage (orange) and the layer's extent (green).
- **Coordinate systems side by side:** the project CRS next to the service's. Each service CRS is marked *built in*, *via epsg.io* or *cannot transform*, and a line says how the data will reach the project CRS. When none of the service's systems can be transformed, the page warns.
- **Load into the drawing** for the project area, through the same code as the WMS and WFS pages. WMS and tiles become image underlays, reprojected when the server lacks the project CRS. WFS features come with their attributes as object data. **Add to my servers** keeps a copy among your own servers. **Open on the WMS/WFS page** gives all the import options.
- **Other service (URL):** open any WMS, WFS or tile URL in the same view.

Coverage is a bounding box in WGS 84 per service. `tools/import_gis2bim_servers.py` takes the state, province or city the coverage text names (German Länder, Austrian and Australian states, Flanders, Wallonia, Brussels, UK nations, Spanish and Italian regions, cities such as Vienna, Prague, Toronto and Cape Town), otherwise the country or region. A box is coarse: it can include a strip of a neighbouring country.

For scripts and the MCP server (`gis_services`, `gis_service_load`):

```
GIS_SERVICES ZOEK="cadastral parcels" HIER ID=s1          → data.services: id, type, name, provider, country, coverage,
                                                             category, url, default_layer, crs, license, terms, bbox
GIS_SERVICES ZOEK=luchtfoto CATEGORIE=aerial TYPE=WMS LON=16.37 LAT=48.21 LIMIET=10
GIS_SERVICE_LOAD DIENST=w-de-by-ldbv-alkis-parzellarkarte-25832 [LAAG=<layer>] [RESOLUTIE=0.25] [LIMIET=5000]
```

## The Civiel tab (civil design)

A second plugin, `openaec.open-infra-civiel`, adds the ribbon tab **"Open Infra — Civiel"**. It shares the engine and the project location with the GIS tab. Everything goes on NLCS layers.

| Group | Tools | What they do |
|---|---|---|
| Punten (points) | **Punten importeren** (import points) · Punt plaatsen (place point) · Punten exporteren (export points) · LandXML | Survey points from PNEZD/PENZD/CSV files: separator, decimal comma and north/east order are detected. Points go on `OG-HOOGTEPUNT` with number, height and code. Clicked points get their ground height from the TIN or AHN. LandXML 1.x brings in points, surfaces and alignments (lines, curves, clothoid spirals). |
| Oppervlak (surface) | **TIN maken** (create TIN) · Hoogtelijnen · Hoogteanalyse (height analysis) · Grondverzet (earthworks) | A TIN (Delaunay, longest-edge filter) from points, 3D lines, contour lines or AHN, as 3D faces on `OG-TRIANGULATIE`: status **B** for existing ground, **N** for a design. Then: exact contours on the TIN; elevation bands with a legend; cut and fill between design (N) and existing (B), or against a level. |
| As (alignment) | **As maken** (create alignment) · As verschuiven (offset alignment) · Station / offset | An alignment from a polyline or from points of intersection with curve radii (*by layout*). It gets stationing `0+100` on `AM-KILOMETRERING`, tangent points BB/EB/HP on `AM-TANGENTPUNT` and radius labels. Then: offset lines; station and offset of any point. |
| Profielen (profiles) | **Lengteprofiel** (length profile) · Ontwerpprofiel (design profile) · Dwarsprofielen (cross sections) | Length profile over the TIN or AHN, with stationing, heights and an alignment band. The design profile comes from PVIs or a line drawn in the profile: parabolic vertical curves (crest and sag, with L and R), and grade labels. Cross sections along the alignment in a grid of views. |
| 3D model | **CityGML/JSON** · See-through ground · **3D Netherlands** ▾ | Open 3D city models (CityGML, CityJSON) from anywhere; make the ground transparent to look at cables, pipes and soil columns beneath. The Dutch menu: the whole 3D model in one go, 3D BAG, terrain with aerial photo, BGT in 3D, KLIC in 3D, BRO CPTs in 3D, water level. See below. |
| Corridor | **Corridor** | An assembly swept along the alignment and its design profile (or a level). Assemblies: *weg*, *fietspad*, *snelweg*, *dijk* (road, cycle path, motorway, dike); lane and shoulder widths, cross slopes, fill/cut slopes, crest and superelevation in curves (with runoff) are all adjustable. The corridor builds: daylight to the ground; a design TIN (status N); 3D feature lines (asphalt edge, shoulder, toe and crest of slope); cut and fill per section with average-end-area volumes and a mass balance (CSV); optional section views with the design. |
| 3D-model (3D model) | **3D BAG** · Terrein + luchtfoto (terrain + aerial photo) · BGT naar 3D (BGT to 3D) | See below. |

The window page **Civiel — instellingen** (settings) holds all settings, and they also apply to the ribbon buttons. Checked on a real road near Dordrecht: the corridor's average-end-area volumes and the independent TIN-to-TIN grid volume agree within 2 %.

## 3D city and landscape model

| | |
|---|---|
| **Terrein + luchtfoto** (`CIV_FOTOTERREIN`) | The AHN ground of the project area as a 3D surface in cells of 1–5 m. Each cell takes the colour of the PDOK aerial photo (8 cm), quantised to a palette, with one mesh per colour. Gaps (water, under buildings) are filled from their lowest neighbour. |
| **3D BAG** (`CIV_BAG3D`) | Every building from the [3D BAG](https://3dbag.nl) (TU Delft, CC BY 4.0) in LoD 1.2, 1.3 or 2.2, with BAG id, year of construction and height as object data. `STIJL=mooi` ("nice", the default): crisp planes, each roof in its colour on the aerial photo, walls by year of construction, monuments in stone, windows per storey and a plinth into the ground. `STIJL=maquette`: a white architectural model. `STIJL=realistisch` or `nlcs`: two plain colours, or the layer colours. |
| **Waterpeil** (water level, `CIV_WATERPEIL PEIL=<m NAP>`) | A water surface at a level over the project area: what floods, and which share of the area lies lower (AHN ground). Again with another level moves the same surface. |
| **KLIC in 3D** (`CIV_KLIC3D [<delivery>] [VERGROOT=1]`) | The cables and pipes of a KLIC delivery as 3D tubes under the AHN ground, in IMKL colours on their NLCS layers: at the depth the delivery gives, otherwise at the usual cover depth per network type (data and low voltage 0.6 m, medium voltage and gas 0.8 m, water 1.0 m, high voltage 1.2 m, gravity sewer 2.0 m). The diameter comes from the delivery where present. Ducts and cable beds are grey sleeves; appurtenances are short columns. The object data says whether the depth came from the delivery or was assumed. Without a file, the delivery imported last on the GIS tab is used. `VERGROOT=` thickens the tubes for overview pictures. |
| **Maaiveld doorzichtig** (see-through ground, `CIV_DOORZICHT [PCT=70] [ALLES] [UIT]`) | Makes every ground surface (terrain, BGT in 3D, water, TIN) transparent, so cables, pipes and soil columns can be seen beneath it from any angle. Buildings stay solid unless `ALLES` is given; `UIT` makes the ground opaque again. |
| **CityGML/JSON** (`CIV_CITYGML <file> [STIJL=]`) | A 3D city model from a file: CityGML 1.0, 2.0 or 3.0 (`.gml`, `.xml`), IMGeo/BGT GML, CityJSON 1.x/2.0 (`.city.json`) or CityJSONSeq (`.city.jsonl`). The file's coordinate system (EPSG codes, RD + NAP 7415, the German AdV names such as `ETRS89_UTM32*DE_DHHN2016_NH`) is transformed to the project location, and only objects in the project area are kept while reading. Buildings get roof and wall meshes on the NLCS building layers, in the same styles as the 3D BAG (walls by year of construction when the file has it); the highest LoD per object is used. Roads, green, water, bridges, tunnels and land use go to the NLCS layer of their BGT counterpart, relief (TIN) to the triangulation layer. 2D objects (IMGeo) are made 3D like `CIV_BGT3D`: draped on the AHN, water at its level, buildings extruded. Tested with the NRW LoD2 tiles (Cologne, 56–97 MB, 1,600–4,500 buildings in 1–2 s) and 3D BAG CityJSON. |
| **BGT naar 3D** (`CIV_BGT3D`) | The whole BGT of the project area in 3D, on its NLCS layers. Areas are draped on the ground: the edges every 3 m, a grid inside, and holes respected. Buildings and walls are extruded to the AHN surface, water lies at its level, bridges at deck height, lines are draped, and posts and trees become 3D symbols (trees with a round crown). Colours are natural by default (asphalt, paving, grass, water…); `KLEUR=nlcs` uses the layer colours. |

![3D BAG styles: current, mooi and maquette, Grote Kerk and a block by the harbour](docs/images/3d-bag-stijlen.png)

![Dordrecht city centre in 3D: AHN terrain with the aerial photo, the BGT in 3D and 3D BAG LoD 2.2, with a ring road and a dike as corridors](docs/images/dordrecht-3d-model.png)

| The Grote Kerk (3D BAG LoD 2.2) and the harbours | Ring road corridor with its cross sections |
|---|---|
| ![Grote Kerk](docs/images/dordrecht-3d-grote-kerk.png) | ![Corridor](docs/images/dordrecht-corridor.png) |

Dordrecht city centre (1.2 × 1.0 km), built by an agent through the MCP server (`tests/e2e/mcp_model.py`, 20 steps in about 110 s):
- 299,000 photo-coloured terrain cells;
- 18,560 BGT objects (about 480,000 triangles);
- 2,864 buildings in LoD 2.2 (204,000 triangles);
- a 1.2 km ring road and a 0.9 km dike as corridors, with 178 cross sections. Their earthworks from the sections and from the design TIN against the existing ground agree within 1 %.

**Large drawings:**
- Open CAD Studio sends a plugin at most 64 MB per message, and shares a document view of 16 MB by default.
- Open Infra Studio sends entities in batches under that limit and draws 3D objects as light MESH entities.
- It keeps an object index in the drawing (`OPENINFRA/INDEX`: terrains, alignments, profiles, corridors, survey points), so the civil commands fetch what they need by handle at any drawing size.
- Whole-drawing tools (NLCS check, quantities) need the shared view. For drawings over 16 MB, start Open CAD Studio with `OCS_V4_SNAPSHOT_SEGMENT_SIZE=268435456` (256 MB; larger values exceed the 1 GiB the plugin side maps).

## Subsurface: CPTs and boreholes from the BRO

The BRO (Basisregistratie Ondergrond, the successor of DINOloket) holds the Dutch cone penetration tests and borehole descriptions as open data (CC0). Open Infra Studio reads them from the public REST services at `publiek.broservices.nl`:
- **CPT** (`sr/cpt/v1`): cone resistance qc, local friction fs and friction ratio Rf per depth, the surface level in m NAP and the end depth.
- **BHR-GT** (`sr/bhrgt/v2`): geotechnical boreholes, with the soil layers described by NEN-EN-ISO 14688 (or NEN 5104 for archive data).

| | |
|---|---|
| **Sonderingen 3D** (`CIV_SONDERINGEN3D`, Civiel → 3D-model) | Every CPT and borehole in the project area becomes a vertical column from its surface level (m NAP) down: 12 sides for a CPT, 8 for a borehole, radius 0.4 m. Each soil layer is its own mesh in a conventional colour: sand yellow, silty sand pale yellow, silt/loam grey-green, clay green, peat brown, gravel orange, made ground grey. Beside each CPT, qc (red, east, 0.1 m per MPa) and Rf (blue, west, 0.25 m per %) are drawn as 3D lines. A plan symbol with the BRO id, surface level and end depth goes with each column. |
| **Sonderingen en boringen (BRO)** (`GIS_Q_SONDERINGEN`, GIS → BGT · BRK · KLIC) | Only the plan symbols and labels, without fetching the measurements: a quick overview of what has been tested. |

- **Soil type from a CPT:** each sample is classified with Robertson's (2010) soil behaviour type index, Isbt = √((3.47 − log₁₀(qc/pa))² + (log₁₀ Rf + 1.22)²), with pa = 0.1 MPa:

  | Isbt | SBT zone | Soil |
  |---|---|---|
  | > 3.60 | 2 | peat / organic soil |
  | 2.95–3.60 | 3 | clay |
  | 2.60–2.95 | 4 | silt mixtures (silt/loam) |
  | 2.05–2.60 | 5 | sand mixtures (silty sand) |
  | 1.31–2.05 | 6 | sand |
  | < 1.31 | 7 | gravelly to dense sand |

  Consecutive samples of the same class form one layer. Layers thinner than 0.2 m (`MINLAAG=`) join their thicker neighbour. Gaps without measurements (a predrilled top, missing data) stay as layers "onbekend".
- **Boreholes:** the described soil name decides the class by its main noun. For example `zwakSiltigZand` is sand, `kleiigZand` silty sand, `zwakZandigeKleiMetGrind` clay, `mineraalarmVeen` peat. Made ground (`puin`, `beton`…) is grey.
- **Object data:** every layer carries the BRO id, date, surface level (m NAP), end depth, soil type and description, top and bottom of the layer in m NAP, and for CPTs the mean qc and Rf.
- **Layers:** NLCS `MO-ONDERZOEK_SONDERING` and `MO-ONDERZOEK_BORING`: G for the columns and graphs, S for the plan symbol, T for the labels. These are the NLCS objects for site investigation.
- **Arguments:** `BRON=sonderingen|boringen|alles`, `MAX=100` (of each kind, nearest to the centre), `STRAAL=0.4`, `MINLAAG=0.2`, `QC=nee` (no graphs), `LABELS=nee` (no plan symbols). With `ID=` the result lists every column with its layers. The MCP tool is `civ_subsurface`.
- **Large areas:** the BRO answers a search with at most 2000 objects. A larger area is split in four automatically.

## Scripting, API and MCP server

Every `CIV_` command and the GIS data commands take arguments:
- `KEY=value`;
- points `x,y` in project coordinates;
- lists separated by `;`.

With `ID=<name>` the outcome is written to `<config>/OpenCADStudio/open-infra/api/<name>.json`, as `{ok, message, data}` with handles, lengths, volumes and so on. This makes Open Infra Studio drivable from scripts and from Open CAD Studio's REST/automation API. `CIV_HELP` and `GIS_HELP` list the arguments.

```
GIS_SITE RD 104900 425350 1200 1000 Dordrecht binnenstad ID=s1
CIV_AS PI=104350,424900;104800,424950;105300,424900 R=200 NAAM=Rondweg ID=a1     → data.as = handle
CIV_ONTWERPPROFIEL AS=<handle> PVI=0:1.5;400:3.0;800:2.0 BOOG=120
CIV_CORRIDOR AS=<handle> SJABLOON=weg RIJSTROOK=3.5 TALUD=3 DWARSPROFIELEN=ja ID=c1
```

The [MCP server](mcp/README.md) (`mcp/open_infra_mcp.py`, Python standard library only) offers about 30 tools on top of this, from `oi_site` to `civ_corridor`, `civ_bag3d` and `oi_capture`. It finds every Open CAD Studio window by itself.

## The workbench window

Every page follows the FreeCAD GIS dialogs:

1. A **map** at the top shows the project location and the bounding box.
2. Grouped settings follow.
3. A fixed action bar holds **OK en importeren**, **Test** and **Annuleren** (Cancel).

You choose the map background in a **Leaflet-style layer switcher** at the top right. Click the map to set the location; Shift+drag draws the bounding box.

| Geographic location | WMS preview (PDOK aerial photo, 8 cm) |
|---|---|
| ![Location page](docs/images/workbench-location.png) | ![WMS page with preview](docs/images/workbench-wms-preview.png) |
| **Server presets**, Netherlands, Flanders and worldwide | **BGT · Kadaster · KLIC** with the BGT preview in NLCS colours |
| ![Server list](docs/images/workbench-servers.png) | ![NLCS page](docs/images/workbench-nlcs.png) |
| **AHN-hoogtes** (AHN heights) with the contour preview (coloured by height) | **AHN5 point cloud** in Open CAD Studio (3D, by class) |
| ![AHN page](docs/images/workbench-ahn.png) | ![AHN5 point cloud](docs/images/ahn-pointcloud.png) |

## Dutch base data to NLCS

The NLCS tables and the official NLCS-BGT 5.0 mapping are built in from [`assets/nlcs`](assets/nlcs): objects, colours, linetypes, statuses, disciplines and main groups. The source is DigiGO / Stichting NLCS, under CC BY 4.0. `tools/nlcs_tables.py` regenerates them from a new NLCS publication.

- **Layers:** every object goes on its NLCS layer `STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT`, e.g. `B-XX-VH-GESLOTENVERHARDING_ASFALT-G` or `B-OI-KL-GAS_LD-G`.
  - The layer gets the NLCS colour, lineweight and linetype for the chosen drawing status (B/N/T/V).
  - Linetypes are defined in paper millimetres, and entities get the linetype scale of the plot scale.
  - Elements: G for lines and areas, S for points, T for texts.
- **BGT:** current objects from the PDOK OGC API (`eind_registratie IS NULL`). Each object is mapped to the most specific NLCS-BGT rule, using function, physical appearance and plus attributes. Street names and house numbers are placed at their BGT rotation.
- **Cadastral map (BRK):** parcel boundaries on `KG-GRENS_PERCEEL`, and parcel numbers at their BRK rotation.
- **KLIC:**
  - IMKL 2.0 deliveries are read with OpenAEC's [`klic2geo`](https://github.com/OpenAEC-Foundation/crates-warehouse).
  - Each theme goes on its KL or RI object.
  - Cables and pipes get labels such as `G-LD Stedin 160 mm PVC`, with duplicates on parallel cables left out.
  - IMKL theme colours can be used instead of the NLCS colours.
- **Object data:** each entity carries its source attributes as XDATA `OPENINFRA`, for example BGT `lokaal_id`, function and material, or the KLIC operator, diameter and material.
- **BGT mutation (`GIS_BGT_MUTATIE`):** the design on NLCS layers back to the BGT, as IMGeo-GML 2.1.1 with a report beside the drawing.
  - Layers N become plan topography (IMGeo plus-status `plan`); R (or N with `GEREALISEERD`) is as built; V ends the BGT object. BGT objects keep their `lokaal_id`; new ones get `<bronhouder>.<uuid>`.
  - The NLCS-BGT mapping is used backwards. Where a layer stands for several BGT types or functions (asphalt: the BGT classes wegdeel, ondersteunend wegdeel or onbegroeid terreindeel, i.e. road section, auxiliary road section or unvegetated terrain), the type and function come from the object data, else from the BGT object the new surface covers most. What stays ambiguous is listed in the report, not exported.
  - Checks: self-intersections, at most 5000 points, coordinates to the millimetre, ring orientation, overlaps on ground level among the mutation and with BGT objects that stay.
  - The file is valid against the IMGeo 2.1.1 schema. The bronhouder's (the responsible authority's) BGT package reads it and delivers to the LV-BGT as StUF-Geo IMGeo through BRAVO.
- **Drawing sheet:**
  - The sheet format is chosen automatically (A3–A0 landscape).
  - The title block lists the data sources of the imports.
  - The legend shows the NLCS layers used, grouped by main group.
  - Sheet texts follow the NLCS text classes (T1.8, T2.5, …).

## AHN heights, profiles and point clouds

- **Heights:** AHN4 from the PDOK WCS: DTM (ground level) and DSM (surface), 0.5 m float grids in m NAP. Larger areas are fetched in tiles or at a coarser cell size.
- **Spot heights** go on `OG-HOOGTEPUNT`, with labels such as `+2,43`. **Contour lines** go on `OG-HOOGTELIJN_MINOR/MAJOR` (marching squares on a lightly smoothed grid). Both sit at their real height (Z).
- **Length profile (`GIS_PROFIEL`):**
  - Select a polyline or line, or pick points and press Enter. Then pick the lower-left corner of the profile.
  - The profile draws the ground line on `GW-MAAIVELD`, optionally the surface line, and a height grid with NAP labels.
  - The profile band (`AL-PROFIELBALK`) lists distances and heights. The NAP line is drawn when it is in range, and the section line A–A' in plan.
  - The profile breaks where AHN has no ground level (buildings, water).
- **Point cloud (`GIS_PUNTENWOLK`):**
  - AHN5 or AHN4 is read as COPC (Cloud Optimized Point Cloud) from Het Waterschapshuis' 1 km tiles. Only the octree nodes in the import area are downloaded, at the chosen point spacing.
  - The cloud is coloured by class (ground, buildings, water, structures, other), by height or by RGB.
  - It is saved as `.rcs` next to the drawing and attached with POINTCLOUDATTACH.
  - Example: 300 × 250 m at 0.25 m spacing is about 1.4 million points and a 29 MB file.
- **Punt info** also reports the AHN ground and surface height of the picked point.

Length profile across the Grotekerksplein: the ground line breaks at the harbour water and the buildings.

![Length profile](docs/images/ahn-profile.png)

## Example drawing

[`examples/dordrecht-3d`](examples/dordrecht-3d) is the city centre of Dordrecht in one drawing: the plan (BGT, cadastre, KLIC) and the 3D model (BGT in 3D, 3D BAG, KLIC in 3D, BRO CPTs, a street design with corridor). It comes as DWG and as IFC 4.3, with the script that builds it.

| Plan | 3D | Beneath the ground |
|---|---|---|
| ![Plan](examples/dordrecht-3d/dordrecht-3d-plattegrond.png) | ![3D model](examples/dordrecht-3d/dordrecht-3d-3d.png) | ![KLIC and CPTs](examples/dordrecht-3d/dordrecht-3d-ondergrond-grote-kerk.png) |

[`examples/dordrecht-nlcs`](examples/dordrecht-nlcs) is the existing situation around the Grote Kerk in Dordrecht. It is 300 × 250 m in RD, on an A1 sheet at 1:500.

| | |
|---|---|
| Entities | 3,439: 1,242 BGT, 1,328 cadastre, 632 KLIC, 237 drawing sheet |
| NLCS layers | 68, with 8 NLCS linetypes |
| With object data (XDATA) | 3,202 |
| Files | DWG (0.7 MB), PDF A1 (1 MB), PNG |

| KLIC delivery around the church, labelled per operator | Legend and title block |
|---|---|
| ![KLIC detail](docs/images/example-klic-detail.png) | ![Legend](docs/images/example-legend.png) ![Title block](docs/images/example-title-block.png) |

It was made in an empty metric drawing with:

```
GIS_SITE RD 104885 425320 300 250 Dordrecht, Grote Kerk
GIS_Q_BGT
GIS_Q_BRK
GIS_KLIC_IMPORT <path>/Levering_21O072584_1.zip
GIS_TEKENBLAD Situatie bestaand - BGT, kadaster en kabels en leidingen
```

## IFC 4.3 export

**GIS → Output → IFC 4x3**, **Civiel → 3D model → IFC 4x3**, or `GIS_IFC` / `CIV_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee]` writes the drawing as IFC 4.3 (IFC4X3_ADD2) beside it, as `<drawing>_IFC4X3.ifc`. The MCP tool is `ifc_export`. Each object becomes the IFC entity of what it is, read from its NLCS layer and its object data:

| In the drawing | In IFC 4.3 |
|---|---|
| Axis (`AM-AS`) with its design profile | `IfcAlignment` with `IfcAlignmentHorizontal` (lines, circular arcs) and `IfcAlignmentVertical` (grades, parabolic curves), the geometry as `IfcCompositeCurve` and `IfcGradientCurve`, a station referent (`Pset_Stationing`) |
| TIN (`OG-TRIANGULATIE`), terrain with aerial photo (`OG-DTM`), water level | `IfcGeographicElement` TERRAIN: one triangulated surface per layer, the photo colours per triangle (`IfcIndexedColourMap`) |
| 3D BAG, CityGML and BGT buildings (`OG-BEBOUWING`) | `IfcBuilding` per building id, roof and walls as coloured meshes |
| Pavements (`VH`) | `IfcPavement` (asphalt FLEXIBLE, concrete RIGID), kerbs `IfcKerb` |
| Green (`GR`), water (`OG-WATER`, `WH`), unpaved terrain | `IfcGeographicElement` VEGETATION, USERDEFINED, TERRAIN |
| Structures (`KW`), street furniture (`IE`) | `IfcCivilElement`; fences and guard rails `IfcRailing`, walls and retaining walls `IfcWall` |
| Road markings (`VW`) | `IfcSurfaceFeature` on the pavement beneath it |
| KLIC cables and pipes (`KL`) | `IfcCableSegment`, `IfcPipeSegment`; casings `IfcCableCarrierSegment`; appurtenances `IfcCableFitting`, `IfcPipeFitting` |
| Sewers (`RI`) | `IfcPipeSegment`; manholes `IfcDistributionChamberElement` MANHOLE |
| CPTs and boreholes (`MO-ONDERZOEK`) | `IfcBorehole` per BRO id, with an `IfcGeotechnicalStratum` per soil layer |
| Earthworks (`GW`) | excavation `IfcEarthworksCut` (it cuts the existing ground), fill `IfcEarthworksFill` |
| Corridor (feature lines, design surface) | in an `IfcRoad` per corridor |
| Contours, spot heights, survey points, parcels, labels, other lines | `IfcAnnotation` CONTOURLINE, SURVEY, TEXT, USERDEFINED |

- **Georeference:** `IfcProjectedCRS` (the project's EPSG code, NAP heights in RD) and `IfcMapConversion` with the base point. Coordinates are in metres from the base point; the site carries latitude and longitude.
- **Data:**
  - every object gets the property sets `OpenInfra_NLCS` (layer, status, discipline, main group, object, element) and `OpenInfra_ObjectData` (its object data, such as BAG id, year of construction or BRO id);
  - the DWG layers become `IfcPresentationLayerAssignment`, the colours surface and curve styles;
  - the DWG handle goes into `Tag`.
- **Stable ids:** GlobalIds follow from the drawing and the object, so a second export of the same drawing gives the same ids.
- **Not exported:** map images, point clouds, hatches, block references and dimensions. The drawing sheet and the length-profile and cross-section views are drawings, not objects, so they stay out too.

## File formats

Open CAD Studio itself opens and saves the drawing (DWG and DXF, R13 to 2018) and plots it to PDF. Open Infra Studio adds the formats below. Imported files are reprojected to the project location. The coordinate system comes from the file where it has one (GeoJSON `crs`, a `.prj` beside the file, GML `srsName`, LAS/LAZ headers); otherwise the project's is used.

### Files you open

| Kind | Formats | Command |
|---|---|---|
| **Vector GIS** | GeoJSON (`.geojson`, `.json`) · Shapefile (`.shp` with `.dbf`, `.shx`, `.prj`) · GeoPackage (`.gpkg`, pick a layer) · KML (`.kml`) · KMZ (`.kmz`) · GPX (`.gpx`) · GML (`.gml`, `.xml`) · CSV, TXT and TSV with coordinate columns (`x`/`y`, `lon`/`lat`) | `GIS_BESTAND <file>`, or *Maps & data → Files*; attributes become object data |
| **Rasters and height grids** | GeoTIFF (`.tif`, `.tiff`), both images and float32 height grids · ASCII grid (`.asc`) · JPEG and PNG (`.jpg`, `.jpeg`, `.png`) with a world file (`.jgw`, `.pgw`, `.wld`) and `.prj` | `GIS_BESTAND <file>`; images go behind the vectors |
| **Point clouds** | LAS and LAZ (`.las`, `.laz`), CRS from the file | `GIS_BESTAND <file>`; written to `.rcs` and attached |
| **Survey points** | PNEZD, PENZD, NEZD and ENZD in `.csv`, `.txt`, `.xyz`, `.pnt` or `.asc`; separator, decimal comma and north/east order are detected | `CIV_PUNTEN_IMPORT` |
| **LandXML** | LandXML 1.x (`.xml`, `.landxml`): points (CgPoints), surfaces (TIN), alignments with lines, arcs and clothoids | `CIV_LANDXML` |
| **3D city models** | CityGML 1.0, 2.0 and 3.0 (`.gml`, `.xml`, `.citygml`) · IMGeo/BGT GML · CityJSON 1.x and 2.0 (`.city.json`, `.json`) · CityJSONSeq (`.city.jsonl`, `.jsonl`) | `CIV_CITYGML <file>` |
| **KLIC delivery** | IMKL 2.0: the delivery as `.zip`, or its GI `.xml` | `GIS_KLIC_IMPORT <file>`, or drop it on the workbench window |

### Files it writes

Beside the drawing, unless the command is given a path.

| Output | Format and name | Command |
|---|---|---|
| **Map images** | PNG, in the folder `<drawing>_GIS` | every WMS, tile and raster import |
| **Point cloud** | `.rcs`, attached to the drawing | `GIS_PUNTENWOLK`, LAS/LAZ import |
| **Survey points** | CSV as PENZD (default), PNEZD, NEZD or ENZD: `punten.csv` | `CIV_PUNTEN_EXPORT` |
| **Mass balance** | CSV per corridor: `corridor_<name>.csv` | `CIV_CORRIDOR` |
| **Quantities** | CSV: `hoeveelheden.csv`, or a table in the drawing | `GIS_HOEVEELHEDEN_CSV`, `GIS_HOEVEELHEDEN_TABEL` |
| **NLCS check** | Markdown report: `nlcs-controle.md` | `GIS_NLCS_RAPPORT` |
| **BGT mutation** | IMGeo-GML 2.1.1 (`bgt-mutatie.gml`) with a Markdown report (`bgt-mutatie.md`) | `GIS_BGT_MUTATIE` |
| **IFC 4.3** | IFC4X3_ADD2: `<drawing>_IFC4X3.ifc` (see [IFC 4.3 export](#ifc-43-export)) | `GIS_IFC`, `CIV_IFC` |
| **Command results** | JSON per call: `<config>/OpenCADStudio/open-infra/api/<ID>.json` | any `CIV_` or GIS data command with `ID=<name>` |

### Web services and data APIs

| Protocol | Formats | Used for |
|---|---|---|
| **WMS 1.1.1 and 1.3.0** | PNG and JPEG maps, XML capabilities | map images from any server in the catalogue |
| **XYZ / TMS tiles** | PNG and JPEG; WMTS REST endpoints as a tile template | OpenStreetMap, aerial photos, HisGIS |
| **Map tile caches** | `…/MapServer` with `tileInfo` | Topotijdreis |
| **WFS 1.1.0 and 2.0.0** | GML and GeoJSON, with paging | vector layers from any server in the catalogue |
| **OGC API Features** | GeoJSON | BGT and GWSW sewers (PDOK) |
| **WCS** | GeoTIFF float32 | AHN height grids |
| **COPC over HTTP** | LAZ 1.4 (COPC), only the octree nodes of the area | AHN4/AHN5 point clouds |
| **3D BAG API** | CityJSON 2.0 | 3D buildings |
| **BRO REST** | IMBRO XML | CPTs (CPT) and boreholes (BHR-GT) |
| **Overpass API** | OSM JSON | OpenStreetMap themes |
| **Geocoding** | PDOK Locatieserver and Nominatim (JSON) | address and place search |
| **epsg.io** | PROJ strings | coordinate systems that are not built in |

### Not yet

The file dialog recognises JPEG 2000 (`.jp2`), E57 and PLY, but cannot read them yet. There is no export to LandXML, GeoJSON, Shapefile or GeoPackage, and no IFC import. Convert such files first, or [open an issue](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/issues).

## Commands

For the command line, scripts and automation:

| Purpose | Commands |
|---|---|
| Workbench pages | `GIS_LOCATIE`, `GIS_KAART`, `GIS_WFS`, `GIS_SERVICES`, `GIS_OSM`, `GIS_PDOK`, `GIS_BGT`, `GIS_COORD`, `GIS_SERVERS` |
| Service catalogue | `GIS_SERVICES [ZOEK=<text>] [CATEGORIE=<c>] [TYPE=WMS\|WFS\|XYZ] [HIER \| LON= LAT=] [LIMIET=50]`, `GIS_SERVICE_LOAD DIENST=<id> [LAAG=<layer>]` |
| In the drawing | `GIS_GEBIED` (pick the import area), `GIS_PUNT` (point info) |
| Location | `GIS_SITE RD <x> <y> [width] [height] [name]`, `GIS_SITE <lon> <lat> [width] [height] [EPSG:code] [name]` |
| Map images | `GIS_Q_LUCHTFOTO`, `GIS_Q_KADASTER`, `GIS_Q_AHN`, `GIS_Q_OSMKAART`, `GIS_Q_SATELLIET`, `GIS_Q_NATURA2000`, `GIS_Q_GEOMORFOLOGIE`, `GIS_Q_BODEMKAART`, `GIS_Q_HISGIS`, `GIS_Q_TIJDREIS`; any server of the catalogue: `GIS_Q_SERVER <id> [LAAG=] [RES=m/px]`, Topotijdreis of any year: `GIS_Q_SERVER JAAR=1850` |
| Vector data | `GIS_Q_BAG`, `GIS_Q_PERCELEN`, `GIS_Q_NWB`, `GIS_Q_HECTO`, `GIS_Q_SPOOR`, `GIS_Q_OSM` |
| NLCS base data | `GIS_Q_BGT`, `GIS_Q_BRK`, `GIS_Q_SONDERINGEN [BRON=]` (BRO CPTs and boreholes in plan), `GIS_KLIC [file]` (opens the page), `GIS_KLIC_IMPORT <file>`, `GIS_TEKENBLAD [subject]` |
| AHN | `GIS_AHN` (page), `GIS_HOOGTEPUNTEN`, `GIS_HOOGTELIJNEN`, `GIS_PROFIEL`, `GIS_PUNTENWOLK` |
| NLCS | `GIS_NLCS_CONTROLE`, `GIS_NLCS_RAPPORT`, `GIS_NLCS_HERSTEL`, `GIS_HOEVEELHEDEN`, `GIS_HOEVEELHEDEN_CSV`, `GIS_HOEVEELHEDEN_TABEL`, `GIS_BGT_MUTATIE [BRONHOUDER=G0505] [GEREALISEERD] [SELECTIE]` |
| IFC 4.3 | `GIS_IFC`, `CIV_IFC` `[BESTAND=<pad>] [SELECTIE] [TEKST=nee]` |
| Civiel tab | `CIV_PUNTEN_IMPORT`, `CIV_PUNT`, `CIV_PUNTEN_EXPORT`, `CIV_LANDXML`, `CIV_TIN`, `CIV_HOOGTELIJNEN`, `CIV_ANALYSE`, `CIV_VOLUME`, `CIV_AS`, `CIV_AS_OFFSET`, `CIV_STATION`, `CIV_LENGTEPROFIEL`, `CIV_ONTWERPPROFIEL`, `CIV_DWARSPROFIELEN`, `CIV_CORRIDOR`, `CIV_BAG3D`, `CIV_FOTOTERREIN`, `CIV_BGT3D`, `CIV_WATERPEIL`, `CIV_CITYGML`, `CIV_KLIC3D`, `CIV_DOORZICHT`, `CIV_SONDERINGEN3D`, `CIV_3DMODEL`, `CIV_INFO`, `CIV_INSTELLINGEN` |
| Help | `GIS_HELP`, `CIV_HELP` |

## Coordinate systems and units

- **Built in:**
  - RD New (EPSG:28992)
  - ETRS89 / UTM 31–33N
  - Belgian Lambert 72 and 2008
  - British National Grid
  - CH1903+ / LV95
  - Lambert-93
  - WGS 84 and Web Mercator
- **Automatic UTM** anywhere in the world.
- **Any other EPSG code** is fetched once from epsg.io and stored with the drawing.
- The address search proposes the national grid for the country it finds.
- **RD accuracy:** RD uses the 7-parameter transformation, about 0.2 m from RDNAPTRANS. That is fine for underlays and context. For survey work, use data that is already in RD; PDOK, BGT, BRK and KLIC all supply it that way.
- **Units:** `INSUNITS` sets the drawing units: metres (default for civil drawings), mm or cm.
  - **Absolute:** CRS coordinates go straight into the drawing, as is usual in Dutch infra work.
  - **Relative:** the base point becomes 0,0.
- **Map images** are saved next to the drawing in `<name>_GIS/`.

## Architecture

```
src/gis/          host-free engine: crs (proj4rs), site, servers, catalog (service search), caps (WMS/WFS GetCapabilities),
                  geocode, raster (GetMap/XYZ + reprojection), wfs, osm (Overpass), vector (GeoJSON + clipping),
                  ogcapi (OGC API Features), bgt (PDOK BGT), klic (IMKL via klic2geo),
                  ahn (WCS height grids, contours, profiles), pointcloud (COPC over HTTP, .rcs writer),
                  bro (BRO CPTs and boreholes, soil classification)
src/nlcs.rs       NLCS 5.02 tables, layer names and styles, BGT/KLIC → NLCS mapping
src/basisdata.rs  BGT, cadastre and KLIC drawn conform NLCS, with object data
src/tekenblad.rs  NLCS drawing sheet: frame, title block, legend, north arrow, scale bar
src/hoogte.rs     AHN spot heights, contour lines, length profiles and point clouds in the drawing
src/emit.rs       engine output → DWG entities (LWPOLYLINE, TEXT, CIRCLE, IMAGE)
src/bridge.rs     host requests from worker/window threads, NLCS layers and linetypes, site in the DWG
src/import.rs     import flows shared by the window and the ribbon quick buttons
src/ui/           workbench window (egui), runs in the plugin process beside the CAD window
src/lib.rs        manifest, ribbon, commands (GIS_*), click interaction (Gebied, Punt info)
src/civiel/       civil engine: landmeten (points), terrein (TIN, contours, analysis, earthwork),
                  as_profiel (alignments, profiles, sections), corridor, landxml, model3d (3D BAG,
                  photo terrain, BGT in 3D), ondergrond (BRO columns), api (arguments, JSON results)
src/gis/bag3d.rs  3D BAG client (CityJSON 2.0), rasterbestand.rs (GeoTIFF, ASC, world files, LAS/LAZ)
civiel/           the Civiel plugin: its ribbon tab and CIV_ commands, on the shared engine
mcp/              MCP server for AI agents
```

The repository is a Cargo workspace with two plugins. The GIS plugin is the root crate, with its entry points behind the default feature `plugin`. The Civiel plugin links the same crate without that feature. Build them with separate `cargo build -p …` runs; `scripts/install.sh` does both.

Open CAD Studio runs plugins out-of-process and has no dialog API for plugins, so the plugin opens its own window. Network work runs on worker threads, so the CAD window stays responsive and the plugin call timeout is never reached.

## Installing

Use Open CAD Studio's **Plugin Manager** (`PLUGINS`): Open Infra Studio is in its list of plugins. Pick it and install the [latest release](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/releases/latest); in an older Open CAD Studio, choose *Add a repository* and enter `OpenAEC-Foundation/Open-Infra-Studio`. This installs the GIS plugin.

Or install by hand:
1. Download `openaec.open-infra-<platform>` and `plugin.toml` from the release.
2. Put them in `<config>/OpenCADStudio/plugins/openaec.open-infra/`.
3. Restart Open CAD Studio.

The Civiel plugin comes as `open-infra-civiel-<platform>.zip` in the same release: unzip it into `<config>/OpenCADStudio/plugins/` (it holds the folder `openaec.open-infra-civiel`) and restart. `<config>` is `%APPDATA%` on Windows, `~/.config` on Linux and `~/Library/Application Support` on macOS.

Release numbers follow Open CAD Studio's calendar numbering. The current code is built for **Open CAD Studio v2026.43** (plugin API 13, opencadcodec `24b037a`, rustc 1.99.1), from release 2026.44.3 on; 2026.44.2 and earlier were for v2026.42. The host only loads a plugin built against its own opencadcodec revision and rustc, so update the plugin together with Open CAD Studio.

## Building

```bash
./scripts/install.sh      # build both plugins and install them into ~/.config/OpenCADStudio/plugins/
./scripts/version.sh      # set both plugins to JJJJ.W.N: this ISO week, the next free number (the release refuses a tag that differs)
cargo test --lib -p open-infra   # engine tests (CRS, clipping, NLCS, TIN, alignments, corridor, 3D BAG, …)
cargo test --test locaties                          # location test set, offline
cargo test --test locaties -- --ignored --nocapture # fetch real data at 16 locations, report in target/locatietest
python3 tests/e2e/ocs_e2e.py                        # end-to-end in Open CAD Studio: import, edit, save, reopen
```

The test set ([`tests/`](tests/README.md)) covers 16 locations: Dutch cities, a polder, the Vaalserberg, the Wadden, and cities across the border and worldwide. It also covers 30 places around the Dutch border, real BGT attributes and a KLIC delivery. It found and fixed several problems:
- the coordinate system for Antwerp and Aachen;
- search results in the wrong town;
- units of mm drawings;
- AHN5 gaps;
- one IMKL colour.

It also lists the host issues still open in Open CAD Studio.

`Cargo.toml` and `civiel/Cargo.toml` pin `ocs_plugin_api` to the Open CAD Studio v2026.43 release. For a newer Open CAD Studio, move the `rev` to the commit of that release, and update the rustc version in `.github/workflows/release.yml` to match.

For development against a local Open CAD Studio checkout, add a `.cargo/config.toml` (not committed):

```toml
[patch."https://github.com/HakanSeven12/OpenCADStudio"]
ocs_plugin_api = { path = "../OpenCADStudio/crates/ocs_plugin_api" }
```

## Platforms

- **Linux** (X11/Wayland) and **Windows**: everything works.
- **macOS**: the ribbon quick buttons and commands work. The workbench window does not yet, because winit needs the main thread there.

## Data sources and licences

- PDOK, Kadaster, Rijkswaterstaat and ProRail: open data (BGT CC0, BRK CC BY 4.0). Credit the source; the title block does this for the imports.
- AHN (Actueel Hoogtebestand Nederland): open data (CC0). Height grids come via the PDOK WCS, point clouds from Het Waterschapshuis' object storage.
- NLCS tables and the NLCS-BGT mapping: DigiGO / Stichting NLCS, CC BY 4.0 (see [`assets/nlcs/NOTICE.md`](assets/nlcs/NOTICE.md)).
- OpenStreetMap: © OpenStreetMap contributors (ODbL), via Overpass, Nominatim and the OSM tile server. Follow their usage policies (light, interactive use).
- Sentinel-2 cloudless by EOX: CC BY-NC-SA 4.0 (non-commercial).
- 3D BAG: 3D geoinformation group, TU Delft, CC BY 4.0 (api.3dbag.nl).
- Aerial photos: Beeldmateriaal.nl via PDOK, CC BY 4.0.
- BRO cone penetration tests and boreholes (Basisregistratie Ondergrond, TNO): open data (CC0), via publiek.broservices.nl.

Licence: GPL-3.0-only ([LICENSE](LICENSE)), the same as the Open CAD Studio plugin API.
