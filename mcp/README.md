# Open Infra MCP-server

Met deze MCP-server kan een AI-agent (Claude Code, Claude Desktop of een andere MCP-client) in Open CAD Studio infra- en civiele modellen bouwen. Hij stuurt de twee Open Infra-plugins aan:

- het tabblad **GIS**, met de `GIS_`-commando's;
- het tabblad **Civiel**, met de `CIV_`-commando's.

De server leest het resultaat van elk commando terug. Het is één Python-bestand (`open_infra_mcp.py`, alleen de standaardbibliotheek, Python ≥ 3.8). Het praat via stdio JSON-RPC volgens MCP 2025-06-18.

## Hoe het werkt

- **Vensters:** elk Open CAD Studio-venster meldt zich in `<config>/OpenCADStudio/automation/<sessie>.json` met poort, token en pid. De server vindt die vensters zelf; je hoeft niets in te stellen.
- **Een venster kiezen:** `oi_sessions` met `select` (sessie-id, pid of tekeningnaam), of de omgevingsvariabele `OI_SESSION`.
- **Commando's:** de server stuurt Open Infra-commando's mét `ID=<naam>`. De plugin schrijft het resultaat naar `<config>/OpenCADStudio/open-infra/api/<naam>.json`, als `{ "ok", "message", "data" }`. De server wacht daarop en geeft `data` terug als `structuredContent` (handles, lengtes, volumes enzovoort).
- **Alternatief via REST:** zet `OI_REST_PORT` en `OCS_API_TOKEN` voor een venster dat met `--http <poort>` is gestart.
- **Coördinaten:** projectcoördinaten in meters, in het CRS van de projectlocatie. In Nederland is dat RD.

## Installeren

Claude Code (`.mcp.json` in je project, of `claude mcp add`):

```json
{
  "mcpServers": {
    "open-infra": {
      "command": "python3",
      "args": ["/pad/naar/Open-Infra/mcp/open_infra_mcp.py"]
    }
  }
}
```

Claude Desktop (`claude_desktop_config.json`): dezelfde regel onder `mcpServers`. Gebruik op Windows `python` en een Windows-pad.

## Tools

De infra-bibliotheek biedt daarnaast `infra_catalog`, `infra_place`, `infra_inspect` en `infra_update`. Lees [de bibliotheekreferentie](../docs/infra-bibliotheek.md) voor aansluitpunten, parameters, bronconflicten en voorbeelden. `parameters` bevat numerieke waarden met de sleutels uit de catalogus; `path` bevat absolute projectpunten in meters.

| Groep | Tools |
|---|---|
| Sessie | `oi_sessions`, `oi_status` (assen, profielen, corridors, terreinen met handles), `oi_command` (elk `GIS_`/`CIV_`-commando of hostcommando) |
| Project en data | `oi_site` (projectlocatie en gebied), `gis_services` (WMS/WFS/tegeldiensten wereldwijd zoeken), `gis_service_load` (een laag daarvan laden), `gis_import` (BGT, BRK, luchtfoto, BAG, NWB, spoor, OSM, AHN, tekenblad …), `nlcs_check`, `quantities`, `bgt_mutation` (NLCS-ontwerp als BGT-mutatie in IMGeo-GML), `ifc_export` (de tekening als IFC 4.3) |
| Landmeten | `civ_points_import` (PNEZD/CSV), `civ_points`, `civ_points_export`, `civ_landxml` |
| Terrein | `civ_tin` (AHN, meetpunten, selectie of laag; status B of N), `civ_contours`, `civ_analysis`, `civ_volume` |
| As en profielen | `civ_alignment` (PI's met boogstralen), `civ_offsets`, `civ_station`, `civ_length_profile`, `civ_design_profile` (PVI's met verticale bogen), `civ_cross_sections` |
| Corridor | `civ_corridor`: sjabloon `weg`, `fietspad`, `snelweg` of `dijk`, met rijstrook, berm, helling, taluds, kruin, verkanting, aansluiting op maaiveld, ontwerp-TIN, kantlijnen, grondverzet per dwarsprofiel |
| 3D-model | `civ_photo_terrain` (AHN met luchtfoto), `civ_bag3d` (3D BAG LoD 1.2/1.3/2.2, stijl mooi of maquette), `civ_bgt3d` (BGT in 3D), `civ_water_level` (waterpeil), `civ_subsurface` (BRO-sonderingen en -boringen als 3D-kolommen met grondsoorten), `civ_klic3d` (KLIC als 3D-buizen), `civ_see_through` (maaiveld doorzichtig), `civ_citygml` (CityGML/CityJSON) |
| Beeld | `oi_view` (aanzicht en visuele stijl), `oi_capture` (PNG terug als afbeelding), `oi_save`, `oi_open` |

## Voorbeeld

```text
oi_site            x=104900 y=425350 width=1200 height=1000 name="Dordrecht binnenstad"
civ_photo_terrain  cell=2
civ_bgt3d          buildings=false
civ_bag3d          lod=2.2
civ_alignment      pis=[[104350,424900],[104800,424950],[105300,424900],[105480,425100]] radii=[200,150] name="Rondweg Zuid"
civ_length_profile axis=<handle>
civ_design_profile axis=<handle> pvis=[[0,1.5],[400,3.0],[800,2.0],[1180,2.5]] curve_length=120
civ_corridor       axis=<handle> assembly=weg sections=true
civ_volume
oi_view            preset=SWISO style=GOURAUDSHADED
oi_capture
```

## Zonder MCP

Elk commando werkt ook rechtstreeks, op de opdrachtregel of via de REST-API van Open CAD Studio. Gebruik argumenten `SLEUTEL=waarde`, punten als `x,y` en lijsten met `;`. Met `ID=<naam>` komt het resultaat in het JSON-bestand. Zie `CIV_HELP` en `GIS_HELP`.
