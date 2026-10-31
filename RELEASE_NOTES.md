Open Infra Studio — open-source GIS and civil design for **Open CAD Studio v2026.43** (plugin API 13), for infrastructure and civil engineering anywhere in the world. It is a free alternative to commercial civil-CAD and desktop-GIS suites: project location and coordinate systems worldwide, web map services and OpenStreetMap, and civil design from survey points and TIN to alignments, corridors and earthworks, all scriptable and agent-ready. Plus deep integration with Dutch base data (BGT, BRK, KLIC, AHN, 3D BAG) and the NLCS drawing standard. Two plugins: the **GIS** ribbon tab and the **Civiel** (civil) ribbon tab. New in this release: the whole drawing as **IFC 4.3**.

**Note:** this release is built for Open CAD Studio **v2026.43**. A plugin for v2026.42 does not load in v2026.43, and the other way round. So update Open Infra together with Open CAD Studio.

**Installing**

1. Update Open CAD Studio to [v2026.43](https://github.com/HakanSeven12/OpenCADStudio/releases/tag/v2026.43).
2. **GIS plugin:** in Open CAD Studio, run `PLUGINS` (Plugin Manager), pick **Open Infra Studio** from the list (or choose *Add a repository* and enter `OpenAEC-Foundation/Open-Infra-Studio`), pick this release and click *Install*. By hand: put `openaec.open-infra-<platform>` and `plugin.toml` from this release in `<config>/OpenCADStudio/plugins/openaec.open-infra/`.
3. **Civiel plugin:** unzip `open-infra-civiel-<platform>.zip` into `<config>/OpenCADStudio/plugins/`. That gives the folder `openaec.open-infra-civiel` with its library and `plugin.toml`.
4. Restart Open CAD Studio. The ribbon shows the tabs **GIS** and **Civiel**.

`<config>` is `%APPDATA%` on Windows, `~/.config` on Linux and `~/Library/Application Support` on macOS.

The libraries are signed: Authenticode with a timestamp on Windows (publisher Impertio Studio B.V.), Developer ID and notarized by Apple on macOS, and a detached OpenPGP signature of the OpenAEC Foundation (`.asc`) per Linux file. Check those with `gpg --verify <file>.asc <file>` against `openaec-signing-public-key.asc` (fingerprint `13F6D414ADEE4ED4CD09AC60601D86C93C504D29`).

**New — IFC 4.3 export**

- **GIS → Output → IFC 4x3** and **Civiel → 3D model → IFC 4x3** (`GIS_IFC`, `CIV_IFC [BESTAND=] [SELECTIE] [TEKST=nee]`, MCP tool `ifc_export`) write the whole drawing as IFC 4.3 (IFC4X3_ADD2) beside it.
- Every object becomes the IFC entity of what it is, from its NLCS layer and object data:
  - terrain and photo terrain as `IfcGeographicElement`;
  - 3D BAG and CityGML buildings as `IfcBuilding`;
  - pavements, kerbs, markings, walls, fences, green and water;
  - KLIC cables, pipes, casings and appurtenances, sewers and manholes;
  - CPTs and boreholes as `IfcBorehole` with a stratum per soil layer;
  - excavation and fill, and a corridor as `IfcRoad`.
- **Axes become IfcAlignments** with their horizontal layout (lines and arcs) and, from the design profile, the vertical one (grades and parabolic curves), with a station referent.
- **Georeferenced:** `IfcProjectedCRS` and `IfcMapConversion` from the project location.
- **Data:** object data and the NLCS layer as property sets, DWG layers and colours as IFC layers and styles.
- **Checked with IfcOpenShell:** schema and EXPRESS rules, and the alignment geometry against its layout. The example drawings export without errors.

**Fixed**

- **IFC export (2026.44.5):** the contents of the cross-section and length-profile views (axis, ground and design lines, texts) went into the IFC, with an extra alignment per cross section. They now stay out; a corridor's 3D axis line is part of the IfcRoad.
- **KLIC in 3D:** appurtenances (joints) and manholes were empty meshes, without faces, so they did not show. They are now closed columns with a lid and a base, also visible from above. Run `CIV_KLIC3D` again for an existing model.

**Other changes**

- **Plugin Manager:** Open Infra Studio is now in the Plugin Manager's own list of plugins.
- **Releases:** `scripts/version.sh` sets both plugins to the `JJJJ.W.N` version of this week, and the release stops when tag and version differ. Every release is signed (see above).

**From 2026.44.3 — international first**

- The ribbons put the worldwide tools first: location, maps & data (services worldwide, WMS/TMS, WFS, OpenStreetMap, files), elevation, output, library. Everything Dutch sits in one compact **Netherlands** group of menus on the GIS tab and one **3D Netherlands** menu on the Civiel tab.
- **Dutch modules on/off** next to the language choice: off hides those menus for users elsewhere.
- **Language:** Open Infra follows Open CAD Studio's interface language and falls back to English; every ribbon label is in English and Dutch.
- The workbench windows have their own taskbar icon instead of a generic one.

**From 2026.44.3 — historical and thematic maps**

- **Topotijdreis:** every Dutch topographic map from 1815 to 2014 (Kadaster), read from ArcGIS tile caches in RD: `GIS_Q_TIJDREIS` or `GIS_Q_SERVER JAAR=1850`.
- **HisGIS:** the cadastral maps (minuutplans) of 1832: `GIS_Q_HISGIS`.
- **Natura 2000, the BRO geomorphological map and soil map** as one-click imports.
- `GIS_Q_SERVER <id> [LAAG=] [RES=]` loads any server of the catalogue by id (MCP tool `gis_server`).
- `CIV_BAG3D RAMEN=nee`: buildings with plain facades, without windows.

**From 2026.44.3 — fixed**

- `GIS_SITE` with a coordinate system that comes from epsg.io kept the previous location's coordinates.
- Services that answer in ISO-8859-1 (e.g. the Spanish cadastre) were refused.
- A new map image went behind the older ones; now the newest map lies on top (a cadastre over the photo), behind the vectors.
- Transparent PNG maps showed a white background.
- A WMS default layer given as a list (`a,b`) took only the first layer.
- A quick import over a large area (tens of km) asked for billions of pixels and hung; the resolution now follows the area.

**From 2026.44.3 — Civiel tab (after Hakan's FreeCAD Road workbench)**

- **Points:**
  - survey points from PNEZD/PENZD/CSV; the separator, decimal comma and N/E order are detected automatically;
  - place points with their height from the TIN or AHN;
  - export;
  - LandXML (points, surfaces, alignments with curves and clothoids).
- **Surface:**
  - TIN from points, 3D lines, contour lines or AHN, with status B (existing) or N (design);
  - contour lines on the TIN;
  - height analysis in colour bands;
  - earthworks: design against existing ground, or against a level.
- **Alignment:**
  - from a polyline, or from PI points with curve radii;
  - stationing (0+100), tangent points BB/EB/HP and curve radii;
  - offset lines;
  - station and offset of points.
- **Profiles:**
  - length profile with an alignment band;
  - design profile from PVIs, with parabolic crest and sag curves (L and R) and grades;
  - cross sections in a grid.
- **Corridor:**
  - templates weg, fietspad, snelweg and dijk (road, cycle path, motorway, dike); lane, shoulder, cross slopes, side slopes, crest and superelevation with runoff are adjustable;
  - daylight to the ground;
  - design TIN and 3D feature lines;
  - cut and fill per cross section (average-end-area method) with a mass balance (CSV);
  - cross sections with the design drawn in.
- **3D model:**
  - the AHN terrain coloured with the PDOK aerial photo;
  - 3D BAG buildings in LoD 1.2, 1.3 or 2.2;
  - the whole BGT in 3D: areas draped, buildings and walls extruded, water at its level, bridges at deck height, posts and trees as 3D symbols, in natural colours.

**From 2026.44.3 — API and MCP server**

- **Arguments:** every `CIV_` command and the GIS data commands also take arguments (`KEY=value`, points `x,y`). With `ID=<name>` the result is saved as JSON in `<config>/OpenCADStudio/open-infra/api/` (handles, lengths, volumes).
- **MCP server** (`mcp/open_infra_mcp.py`): about 30 tools that let an AI agent (Claude Code, Claude Desktop) build whole models. It finds every Open CAD Studio window by itself.
- **Tested with a mega-model of Dordrecht city centre** (1.2 × 1 km), built through the MCP server:
  - 299,000 photo cells;
  - 18,560 BGT objects in 3D;
  - 2,864 buildings in LoD 2.2;
  - a ring road and a dike with corridor and cross sections.

**From 2026.44.3 — nicer 3D and water level**

- **3D BAG in the "mooi" ("nice") style** (now the default of `CIV_BAG3D`):
  - crisp roof and wall planes;
  - each roof in its colour from the aerial photo;
  - walls by year of construction;
  - monuments in natural stone;
  - windows per storey;
  - a plinth down into the ground.
- `STIJL=maquette` gives a white architectural model; `STIJL=realistisch` and `STIJL=nlcs` remain.
- Trees in BGT 3D get a round crown.
- `CIV_WATERPEIL PEIL=<m NAP>` (Civiel → 3D-model → Waterpeil) lays a water surface over the project area and reports which share lies lower. With another level, the same surface moves along: climate adaptation in one picture. Also available as the MCP tool `civ_water_level`.

**From 2026.44.3 — the subsurface from the BRO: CPTs and boreholes in 3D**

- `CIV_SONDERINGEN3D` (Civiel → 3D-model → Sonderingen 3D) fetches the cone penetration tests (BRO CPT) and geotechnical boreholes (BRO BHR-GT) of the project area from the Basisregistratie Ondergrond, the successor of DINOloket.
- Each one becomes a 3D column from its surface level (m NAP) down, one mesh per soil layer in conventional colours: sand yellow, clay green, peat brown, silt/loam grey-green, gravel orange, made ground grey.
- CPT layers come from cone resistance and friction ratio with Robertson's (2010) soil behaviour type index. Samples of the same class merge, and layers under 0.2 m join their neighbour. Boreholes use their described NEN-EN-ISO 14688 or NEN 5104 soil names.
- qc and Rf are drawn as 3D graphs beside each CPT, with a plan symbol and BRO id for each column. Every layer carries its BRO id, date, surface level, end depth, soil type, and top and bottom in m NAP as object data. They go on the NLCS layers `MO-ONDERZOEK_SONDERING` and `MO-ONDERZOEK_BORING`.
- `GIS_Q_SONDERINGEN` (GIS → BGT · BRK · KLIC) draws only the plan symbols, as a quick overview. The MCP tool is `civ_subsurface`.

**From 2026.44.3 — service browser: WMS, WFS and tiles worldwide**

- A new workbench page, **Services wereldwijd / Services worldwide**, opens from the ribbon (*Kaarten & data*) or with `GIS_SERVICES`. It searches the 19 built-in presets, 600 checked public services from GIS2BIM's worldwide list (45 countries, Europe, Africa, worldwide) and your own servers.
- Free-text search in Dutch and English, with theme words in several languages (*kadaster*, *parcels*, *Flurstücke*, *cadastro*; *luchtfoto*, *aerial*; *hoogte*, *DEM*; *bodem*, *geology* …). Place names work too (*Bayern*, *Vlaanderen*, *Wenen*). Filter chips cover category and type.
- **At this location:** only the services whose coverage contains the centre of the workbench map (or the project location). Zoom the map in on a place to see what is available there; local services come first.
- Choosing a service fetches its GetCapabilities. The page lists the layers with their extent and coordinate systems, and shows whether the location lies inside. The project CRS and the service's CRS are shown side by side, with a warning when no service CRS can be transformed.
- **Load into the drawing** uses the existing WMS, tile and WFS imports. WFS features keep their attributes. **Add to my servers** and **Open on the WMS/WFS page** are there too, and any other service can be opened by URL.
- For scripts and agents: `GIS_SERVICES ZOEK=… [CATEGORIE=] [TYPE=] [HIER | LON= LAT=]` returns the matches as JSON. `GIS_SERVICE_LOAD DIENST=<id> [LAAG=]` loads one layer. The MCP server gains the tools `gis_services` and `gis_service_load`.
- `assets/servers/world.json` now carries each service's coordinate systems, provider, licence, coverage and a WGS 84 coverage box. `tools/import_gis2bim_servers.py` derives the box from the state, province or city the coverage names. Service ids are unchanged.
- Reprojection: a WMS layer without the project CRS, Web Mercator or WGS 84 is now requested in a system that transforms, and an EPSG code that is not built in is loaded once from epsg.io. A WFS feature type that lists neither the project CRS nor WGS 84 is requested in a system it does list. Layer extents are now read from GetCapabilities, and WMS 1.1.1 capabilities with a DOCTYPE are no longer refused.

**From 2026.44.3 — KLIC in 3D under a see-through ground**

- `CIV_KLIC3D` (Civiel → 3D-model → KLIC in 3D) turns a KLIC delivery into 3D tubes under the AHN ground, in IMKL colours on the NLCS layers. The depth comes from the delivery where given, else the usual cover depth per network type; the object data records which. Ducts are grey sleeves, appurtenances short columns. `VERGROOT=` thickens the tubes for overview pictures.
- `CIV_DOORZICHT` (Maaiveld doorzichtig) makes the ground surfaces transparent, so cables, pipes and CPT columns show beneath the street. `UIT` makes it opaque again.
- MCP tools `civ_klic3d` and `civ_see_through`.

**From 2026.44.3 — CityGML and CityJSON import**

- `CIV_CITYGML <file>` (Civiel → 3D-model → CityGML/JSON) imports a 3D city model: CityGML 1.0, 2.0 and 3.0, IMGeo/BGT GML, CityJSON 1.x/2.0 and CityJSONSeq.
- The file's coordinate system is transformed to the project location: EPSG codes, RD + NAP (7415) and the German AdV names (`ETRS89_UTM32*…`, `DHDN_3GK*`).
- Large files are read object by object and filtered to the project area while reading. A 97 MB NRW LoD2 tile (4,500 buildings) loads in under 2 seconds.
- Buildings: roof and wall meshes on the NLCS building layers, in the 3D BAG styles (`STIJL=mooi`, `maquette`, `realistisch`, `nlcs`), with the highest LoD per object and walls by year of construction. Other themes (roads, green, water, bridges, relief) go to the NLCS layers of their BGT counterpart; 2D IMGeo objects are draped on the AHN.
- MCP tool `civ_citygml`.

**From 2026.44.3 — NLCS design as BGT mutation (#10)**

- `GIS_BGT_MUTATIE` (ribbon *NLCS → BGT-mutatie*, or the NLCS-controle page) exports the design back to the BGT, as IMGeo-GML 2.1.1 with a report beside the drawing. The GML is valid against the IMGeo 2.1.1 schema. The municipality's BGT package reads it and delivers it to the LV-BGT.
- Layers N become plan topography, R (or N with *Gerealiseerd*, "as built") becomes as built, and V ends the BGT object. Existing objects keep their `lokaal_id`; new ones get `<bronhouder>.<uuid>`.
- Type and function come from the BGT data, from the BGT object the new area covers, or from the NLCS layer if that is unambiguous. Anything ambiguous is listed in the report.
- Checks: self-intersection, at most 5000 points, millimetres, ring orientation, overlap at ground level.

**From 2026.44.3 — other changes**

- **Button test:** `tests/e2e/ui_knoppen.py` clicks every button of both windows in a real Open CAD Studio. Found and fixed: *Annuleren* (Cancel) on the location page now closes the window, *Zoeken* (Search) is disabled while the search field is empty, and `GIS_SITE` rejects RD coordinates without the word `RD`.
- **NLCS check and quantities** (also in the 2026.44.x betas):
  - the check on its own page, with repair (can be undone) and a report;
  - quantities per layer or object, as CSV or as a schedule in the drawing.
- **GIS ribbon:** the Dutch PDOK items sit compactly in one group *Nederland (PDOK)*, as menus. The worldwide map and data tools get the room.
- **Large drawings:**
  - the plugin reads them through Open CAD Studio's shared memory view;
  - entities are sent in batches under the 64 MB message limit;
  - 3D objects are light meshes.

**From 2026.44.2 — clearer search in Geo-locatie (geolocation)**

- **Suggestions while you type:** after a short pause, a list of Dutch addresses appears (PDOK). Enter or **Zoeken** also searches worldwide (OpenStreetMap).
- **Each hit on two lines:** the name in bold, with the place below ("3311 CJ Dordrecht"). A coloured label shows the kind of hit: address, street, place, parcel or coordinate. When hits come from both sources, they are grouped under *Nederland · PDOK* and *Wereldwijd · OpenStreetMap*.
- **No clutter:**
  - No more duplicate hits (the same street as a street, a postcode and an OSM hit).
  - Without a house number, one line per street.
  - Stray hits that do not match are dropped.
  - An exact hit comes first.
  - Worldwide names are short and in Dutch.
- **Controls:** arrow keys and Enter to choose, Esc to close. "Zoeken…" (searching) and "Niets gevonden" (nothing found) appear in the list itself.
- **RD coordinates** (`104885 425320`) are recognised, as well as latitude and longitude.
- **The list never covers the search field:** it opens above the field when there is no room below.

**From 2026.44.1 — test set and fixes**

A new test set fetches data at 16 locations, from Dordrecht and the Vaalserberg to Antwerp, London, New York and the North Sea. It also edits the drawing in Open CAD Studio. This is what it found, and it is now fixed:

- **Coordinate system on a map click:** Antwerp, Ghent, Bruges, Aachen, Kleve, Emmerich and Gronau got RD, Lille got Lambert 72 and the North Sea got RD. Now rough national borders of the Netherlands and Belgium are used, and 30 border towns are tested.
- **Search:** "Grote Kerk Dordrecht" gave a street in Hoorn, and "Utrecht Centraal" a bus station in Breda. Now hits with all search words come first; otherwise the search looks in the last place named.
- **mm and cm drawings** got everything in metres, because the plugin did not read INSUNITS. Now the unit comes from the drawing.
- **AHN point cloud:** where AHN5 is still missing (for example in Groningen), the tile comes from AHN4.
- **AHN outside the Netherlands** now gives a clear message instead of a server error.
- **KLIC IMKL theme colours** now follow the PMKL-handreiking 3.0 (PMKL guideline) exactly: buisleiding gevaarlijke inhoud (pipelines with hazardous contents) #ff7f00, plus wees (orphaned) and overig (other).
- **`GIS_SITE … RELATIEF`** sets the project location with the base point at 0,0.

Test set ([tests/README.md](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/blob/main/tests/README.md)):
- **Offline:** on every push to GitHub.
- **Online:** weekly, with search, map image, BGT, BRK, OSM, AHN, profile and point cloud for each location.
- **End-to-end in Open CAD Studio:** import, move, change layer, erase, undo, save and reopen, mm drawing, relative coordinates.

Known issues, in Open CAD Studio v2026.42 itself:
- A plugin import cannot be undone. What older release notes promised ("every import is one undo step") is therefore not true.
- Importing again in the same area adds everything a second time.

**From 2026.44 — AHN heights, profiles and point cloud**

![AHN5 point cloud in Open CAD Studio](https://raw.githubusercontent.com/OpenAEC-Foundation/Open-Infra-Studio/v2026.44.2/docs/images/ahn-pointcloud.png)

- **Spot heights:** AHN4 ground level (DTM) or surface (DSM) in a grid of 2 to 100 m.
  - They go on `OG-HOOGTEPUNT`, with heights such as `+2,43` m NAP.
- **Contour lines:** from the AHN4 ground level.
  - Interval of your choice (0.1–5 m), with labelled major lines on `OG-HOOGTELIJN_MINOR/MAJOR`.
  - Lightly smoothed, without fragments.
  - Lines and points sit at their height (Z).
- **Length profile** (`GIS_PROFIEL`):
  - Select a polyline or pick points, press Enter and pick where the profile goes.
  - The profile shows the ground line (`GW-MAAIVELD`) and optionally the surface line (DSM), with a height grid and NAP labels.
  - A profile band gives distances and heights. The NAP line appears when it is in range.
  - Section line A–A' is drawn in plan.
  - The vertical exaggeration is adjustable.
- **Point cloud** (`GIS_PUNTENWOLK`):
  - AHN5 or AHN4 as COPC. Only the needed part of the 1 km² tiles is downloaded, at a point spacing of your choice.
  - Colour by class, height or RGB, and a filter on classes (ground, buildings, water, structures, other).
  - The point cloud is saved as `.rcs` next to the drawing and attached. You see it in a 3D visual style.
- **Punt info** (point info) now also gives the AHN ground and surface height.
- **Workbench:** page *AHN-hoogtes* (AHN heights) with **Test** (contour lines on the map). In the ribbon, the group *Hoogte (AHN)* with AHN, Hoogtepunten, Hoogtelijnen, Profiel and Puntenwolk (spot heights, contour lines, profile, point cloud).
- **New screenshots** in the README.

Data:
- AHN4 height grids via the PDOK WCS.
- AHN4/AHN5 point clouds from Het Waterschapshuis.
- AHN is open data (CC0).

**From 2026.43 — Dutch base data to NLCS**

From this release on, version numbers follow Open CAD Studio's calendar numbering. Release 2026.43 is built for Open CAD Studio v2026.42.

- **NLCS 5.02 built in:**
  - 3,659 objects, 256 colours and 543 linetypes.
  - All statuses and disciplines.
  - The official NLCS-BGT 5.0 mapping with 719 rules (DigiGO / Stichting NLCS, CC BY 4.0).
- **Layers:** every object goes on its NLCS layer `STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT`.
  - The layer gets the NLCS colour, lineweight and linetype.
  - Linetypes are defined in paper mm; the linetype scale follows the drawing scale (1:100–1:5000).
- **BGT:** the current objects via the PDOK OGC API, in 42 collections and 11 groups. Street names and house numbers get their rotation from the BGT.
- **Cadastral map (BRK):** parcel boundaries and parcel numbers, on `KG-GRENS_PERCEEL`.
- **KLIC:**
  - The delivery is read as a zip or GI xml (IMKL 2.0, with `klic2geo`).
  - 13 themes on the NLCS objects KL and RI.
  - Labels per cable or pipe, such as `G-LD Stedin 160 mm PVC`.
  - Optionally the IMKL theme colours.
- **Object data:** every object gets its source attributes as XDATA `OPENINFRA`.
- **Drawing sheet:**
  - Frame and title block, with the sources of the imports.
  - A legend of the NLCS layers used.
  - North arrow, scale bar and RD coordinate crosses.
  - The sheet format is chosen automatically (A3–A0).
- **Workbench:** new page *BGT · Kadaster · KLIC* with **Test** (preview on the map) and **OK en importeren** (OK and import). You can also drop a KLIC delivery on the window.
- **Ribbon:** new group *Basisdata NL (NLCS)* with BGT · BRK · KLIC, BGT, Kadaster, KLIC (file picker) and Tekenblad (drawing sheet).
- **New commands:** `GIS_SITE`, `GIS_BGT`, `GIS_Q_BGT`, `GIS_Q_BRK`, `GIS_KLIC`, `GIS_KLIC_IMPORT` and `GIS_TEKENBLAD`.
- **Example drawing** in `examples/dordrecht-nlcs`: the Grote Kerk in Dordrecht with BGT, cadastre and KLIC, A1 1:500, as DWG, PDF and PNG.

Limitations:
- Linetypes with symbols or text are only included as dash patterns.
- Point objects are circles instead of NLCS symbols.
- Areas get no hatching yet.

**From 0.2.1**
- **Larger location window:** the workbench window opens at 1280 × 1000, and the map on the *Geografische locatie* (geographic location) page fills most of the window, like the map in the FreeCAD dialog "Set Geographic Location".

**From 0.2.0**
- **Ribbon tab *Open Infra — GIS***, organised like the toolbars of the FreeCAD GIS workbench:
  - *Webservices*: Geo-locatie, WMS/TMS, WFS, OpenStreetMap, PDOK (NL).
  - *Snel importeren* (quick import): map image and vector data in one click.
  - *Hulpmiddelen* (tools): Gebied, Punt info, Coördinaten, Servers.
- **Workbench window** modelled on the FreeCAD GIS dialogs:
  - At the top, a **map** with the project location and the bounding box.
  - You choose the background map at the top right, with a Leaflet layer switcher: OSM, PDOK BRT, PDOK aerial photo or Sentinel-2.
  - Click the map to set the location; draw the area with Shift+drag.
  - **Test** shows a preview of the import on the map; **OK en importeren** places it in the drawing.
- **PDOK (NL)** in one go: aerial photo, cadastral map, AHN, BAG, parcels, NWB road sections and hectometre markers, ProRail track axes and kilometre markers, waterways.
- **Coordinate systems:**
  - Built in: RD New, ETRS89/UTM, Belgium, the UK, Switzerland and France.
  - Automatic UTM worldwide, plus any EPSG code via epsg.io.
  - The address search proposes the national grid.
- **Imports:**
  - WMS with tiling and reprojection, and XYZ tiles.
  - WFS with paging.
  - OpenStreetMap via Overpass.
  - Everything can be clipped to the import area.
- **In the drawing:**
  - The project location is stored in the DWG; the default is the Grote Kerk in Dordrecht.
  - Optionally a base-point marker (GeoOrigin).
  - Map images are saved next to the drawing and sit behind the vectors.
  - Every import is one undo step.

The window works on Linux and Windows. On macOS the ribbon buttons and commands work, but the window does not yet.

Data: PDOK, Kadaster, Rijkswaterstaat and ProRail (open data). © OpenStreetMap contributors (ODbL). Sentinel-2 cloudless by EOX (CC BY-NC-SA 4.0).
