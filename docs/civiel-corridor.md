# Corridors: parts, assemblies, junctions and roundabouts

The Civiel tab's corridor sweeps an assembly (profieltype) along an axis and its design profile. This page describes:

- the parts library, with pavement layers;
- assemblies as JSON files, with conditions and targets;
- junctions (`CIV_KRUISPUNT`) and roundabouts (`CIV_ROTONDE`);
- where every design value comes from.

All commands also run from scripts and agents: `KEY=value` arguments, coordinates `x,y` in project coordinates, and `ID=<name>` for a JSON result in `<config>/OpenCADStudio/open-infra/api/<name>.json`.

## Commands

| Command | Arguments | What it does |
|---|---|---|
| `CIV_CORRIDOR` | `AS=<handle>` `PROFIELTYPE=<name>` or `<file.json>` (`SJABLOON=` still works), `NIVEAU=` or `BOVEN=`, `VAN=` and `TOT=` (stations), `DOELEN=name:handle;…`, `FREQUENTIE=`, `DWARSPROFIELEN=ja` or `x,y`, `KLEUR=nlcs`, and the overrides `RIJSTROOK=`, `BERM=`, `HELLING=`, `BERMHELLING=`, `TALUD=`, `INGRAVING=`, `KRUIN=`, `VERKANTING=`, `REIKWIJDTE=` | Builds the corridor (see [Output](#what-a-corridor-builds)). `VAN`/`TOT` limit it to a station range, so a junction or roundabout can fill the rest. |
| `CIV_PROFIELTYPEN` | – | Lists every assembly (presets and your own), with its parts and any error in your files. With `ID=`: the full definitions and the named build-ups. |
| `CIV_PROFIELTYPE` | `NAAM=<name>` `[KOPIE=<new name>]` `[PLAATS=x,y]` | Shows one assembly. `KOPIE` copies it into your folder as a start for your own. `PLAATS` draws it as a template section at that point: in fill and, with conditions, also in cut, with its layers and a label per part. |
| `CIV_KRUISPUNT` | `HOOFD=<handle>` `ZIJ=<handle>` `[R=8;10;…]` `[PROFIELTYPE=]` `[PROFIELTYPE_HOOFD=]` `[PROFIELTYPE_ZIJ=]` `[NIVEAU=]` `[NAAM=]` | A T or X junction between two axes with design profiles. From the ribbon: select both axes; the one that ends at the crossing is the side road. |
| `CIV_ROTONDE` | `ASSEN=<handle>;<handle>…` `[MIDDEN=x,y]` `[TYPE=binnen|buiten]` `[D=]` `[RIJBAAN=]` `[OVERRIJDBAAR=]` `[IN=]` `[UIT=]` `[INBREEDTE=]` `[UITBREEDTE=]` `[MIDDENGELEIDER=]` `[LENGTE=]` `[AANSLUITING=]` `[HELLING=]` `[PROFIELTYPE=]` `[NIVEAU=]` `[NAAM=]` | A single-lane roundabout on the given axes. Without `MIDDEN`, its centre is where the first two axes cross. From the ribbon: select the axes. |

The ribbon's Corridor group has the buttons **Corridor**, **Profieltypen / Assemblies**, **Kruispunt / Junction** and **Rotonde / Roundabout**.

## The parts

An assembly lists its parts per side, outward from the axis. Slopes are in %, and a negative slope runs down away from the axis. The table gives the defaults a JSON file may leave out, and where each comes from.

| `type` | Part | Point code | Default | Source |
|---|---|---|---|---|
| `rijstrook` | lane; tilts in curves | RS | width required, −2,5 % | slope: the 2026.44 `weg` template (estimate) |
| `goot` | gutter strip | GO | 0,30 m, −2,5 % | estimate (two rows of 10 cm paving plus joints) |
| `band` | kerb: `opstand` (upstand above the surface before it), `hoogte` (element height), `schuinte` (run of the sloping front face), `breedte` | BV (front top), BA (back top) | trottoirband 13/15×25: 0,15 m wide, 0,25 m high, 0,02 m front face, upstand 0,12 m | 13/15×25 cm: NLCS object `KANTOPSLUITING_TROTTOIRBAND_130X150` and the product range under NEN-EN 1340; the upstand of 0,12 m is an estimate |
| `trottoir` | footway | TR | 1,80 m, +2 % (towards the road) | estimate; check against CROW ASVV 2021 |
| `fietspad` | cycle path | FP | 2,00 m, −2 % | width: CROW publication 351 *Ontwerpwijzer fietsverkeer*, one-way path at 0–150 cyclists per peak hour; slope: estimate |
| `parkeerstrook` | parking lane | PS | 2,00 m, −2,5 % | estimate; check against CROW ASVV 2021 |
| `middenberm` | central reserve; in `beide` its width is the whole reserve | MB | 4,00 m, +4 % (a shallow trough) | estimate |
| `scheidingsberm` | separating verge (between road and cycle path) | SB | 1,50 m, −2 % | estimate |
| `berm` | shoulder | BE | 1,50 m, −4 % | the 2026.44 `weg` template (estimate) |
| `kruin` | dike crest | KR | 2,50 m, −2 % | the 2026.44 `dijk` template (estimate) |
| `sloot` | ditch: `bodembreedte`, `diepte` or `bodempeil` (m NAP), `talud_binnen`, `talud_buiten` (1:n) | SL (bottom), SI (outer edge) | 0,50 m bottom, 0,80 m deep, 1:1,5 both sides | estimate: ditches follow the waterschap's *legger* |
| (daylight) | the fill or cut slope to the ground, `talud` and `ingraving` of the assembly | TL | 1:3 and 1:2 | the 2026.44 `weg` template |

A ditch that is the last part of a side daylights with its back slope: it rises at `talud_buiten` until it meets the ground. Followed by another part, its back slope rises to the level of its inner edge.

Each part's edge becomes a 3D feature line on the NLCS layer of its surface:
- asphalt `VH-GESLOTENVERHARDING_ASFALT`, red asphalt `…_ASFALT_ROOD`;
- paving `VH-OPENVERHARDING_BETONSTRAATSTEEN`, tiles `VH-OPENVERHARDING_TEGEL`;
- kerbs `VH-KANTOPSLUITING_TROTTOIRBAND`, `…_OPSLUITBAND` and so on;
- gutters `VH-GOOT_BSS`, grass `GR-GRAS_BERM`;
- ditches `GW-WATERGANG_SLOOT_BODEM` and `…_INSTEEK`;
- daylight `GW-TALUD_TEEN` (fill) and `GW-TALUD_KRUIN` (cut).

A part's `materiaal` (for example `"tegels"`) or `nlcs` (`"VH-OPENVERHARDING_NATUURSTEEN"`) sets that layer.

## Pavement build-up

Each part may have `lagen`: a list of layers from the top down, or the name of a build-up from `assets/assemblies/opbouw.json`.

```json
"lagen": [
  {"materiaal": "AC 11 surf (deklaag)", "dikte": 0.04},
  {"materiaal": "menggranulaat (fundering)", "dikte": 0.25, "rol": "fundering"}
]
```

`rol` is `verharding` (surfacing: asphalt, paving and its bedding) or `fundering` (foundation). Without it, the material name decides: granulate, AGRAC, sand-cement and sand beds are foundation, everything else surfacing. The material name also picks the NLCS layer of the layer in the section views (`AC … surf` → `…_ASFALT_DEKLAAG`, `menggranulaat` → `VH-FUNDERING_GRANULAAT`, `zand` → `VH-FUNDERING_ZAND`, …).

The named build-ups are **estimates**: common build-ups, not a pavement design. Material names follow the Standaard RAW 2020 and NEN-EN 13108-1 (asphalt concrete AC … surf/bin/base), NEN-EN 13242 (unbound granulates), NEN-EN 1338 (concrete paving blocks) and NEN-EN 1339 (concrete flags). Design the pavement per project, for its traffic and subsoil.

| Name | Layers (thickness in m) | Total |
|---|---|---|
| `asfalt-gow` | AC 11 surf 0,04 · AC 16 bin 0,06 · AC 22 base 0,08 · menggranulaat 0,30 · zand 0,50 | 0,98 |
| `asfalt-stad` | AC 11 surf 0,04 · AC 16 bin 0,06 · menggranulaat 0,25 · zand 0,30 | 0,65 |
| `asfalt-etw` | AC 11 surf 0,03 · AC 16 base 0,06 · menggranulaat 0,25 · zand 0,30 | 0,64 |
| `fietspad-asfalt` | AC 8 surf rood 0,03 · AC 16 base 0,05 · menggranulaat 0,20 · zand 0,20 | 0,48 |
| `snelweg-zoab` | ZOAB 16 0,05 · AC 22 bin 0,08 · AC 22 base 0,12 · AGRAC 0,25 · zand 1,00 | 1,50 |
| `klinkers` | betonstraatsteen 0,08 · straatlaag 0,05 · menggranulaat 0,25 · zand 0,30 | 0,68 |
| `goot` | betonstraatsteen (BSS) 0,08 · straatlaag 0,05 · menggranulaat 0,25 · zand 0,30 | 0,68 |
| `tegels` | betontegels 30×30×5 0,05 · straatlaag 0,05 · zand 0,20 | 0,30 |
| `parkeren` | betonstraatsteen 0,08 · straatlaag 0,05 · menggranulaat 0,20 · zand 0,30 | 0,63 |
| `band` | beton stelbed (foundation of the kerb) 0,10 | 0,10 |

An `opbouw.json` beside your assemblies (`{"opbouw": {"naam": {"omschrijving": "…", "lagen": [ … ]}}}`) adds build-ups or replaces these.

## What a corridor builds

- **The design surface** (top) as a TIN of meshes on `N-…-OG-TRIANGULATIE-G`: strips between the sections. Where the sections differ (a condition swaps a part, a width follows a target), the strip is triangulated by offset.
  Each part's strip has a natural colour in 3D (asphalt grey, red asphalt, tiles, paving, kerbs, grass); `KLEUR=nlcs` keeps the layer colour. The junction and roundabout surfaces take `KLEUR=nlcs` too.
- **Formation and subgrade**, when parts have layers:
  - the formation (`VH-FUNDERING`) is the top of the foundation, under the surfacing of each part;
  - the subgrade (`GW-CUNET_BODEM`) is the underside of the whole build-up, and the design surface where a part has no layers (slopes, verges, ditches).
- **Feature lines** along every part edge (see above), the axis on `AM-AS` with the corridor's object data.
- **Earthwork** per section, by average end areas:
  - cut and fill against the top (as before; this matches a TIN-to-TIN volume of the design surface);
  - cut and fill against the subgrade: the earthwork for the build-up (`afgraven_onderkant_m3`, `ophogen_onderkant_m3`).
  The list `corridor_<axis>.csv` beside the drawing has both.
- **Material quantities** in `corridor_<axis>_materialen.csv` and in the JSON result:
  - per layer material its volume (m³);
  - per surface its area (m²);
  - per kerb its length (m) and volume (m³).
  All by average end areas between the sections.
- **Section views** (`DWARSPROFIELEN=ja`): the ground, the design and the outline of every layer and kerb.

## Your own assemblies (JSON)

Put a file `<name>.json` in `<config>/OpenCADStudio/open-infra/assemblies/`. On Linux that is `~/.config/OpenCADStudio/open-infra/assemblies/`; on Windows `%APPDATA%\OpenCADStudio\open-infra\assemblies\`. A file with a preset's name replaces that preset. `CIV_PROFIELTYPE NAAM=weg KOPIE=mijnweg` writes a copy to start from.

```json
{
  "naam": "polderweg",
  "omschrijving": "Polderweg; in ingraving een sloot, in ophoging een bredere berm",
  "bronnen": ["…"],
  "talud": 3, "ingraving": 2, "reikwijdte": 40, "verkanting": 0, "uitrol": 30,
  "beide": [
    {"type": "rijstrook", "breedte": 2.50, "helling": -2.5, "lagen": "asfalt-etw"},
    {"type": "berm", "breedte": 1.50, "helling": -4.0},
    {"type": "sloot", "als": "ingraving", "bodembreedte": 0.50, "diepte": 0.80, "talud_binnen": 1.5, "talud_buiten": 1.5},
    {"type": "berm", "als": "ophoging", "breedte": 1.00, "helling": -4.0}
  ]
}
```

- **Sides:** `beide` (both), or `links` and `rechts` for an asymmetric profile.
- **Assembly keys:**
  - `talud` and `ingraving`: daylight slopes 1:n;
  - `reikwijdte`: how far a slope looks for the ground (m);
  - `verkanting`: superelevation in curves (%);
  - `uitrol`: its runoff (m).
- **Part keys:**
  - `type`, `naam`, `breedte`, `helling`, `lagen`, `materiaal`, `nlcs`;
  - for kerbs: `opstand`, `hoogte`, `schuinte`;
  - for ditches: `bodembreedte`, `diepte`, `bodempeil`, `talud_binnen`, `talud_buiten`;
  - `als`, `breedte_doel`, `hoogte_doel`, `helling_min`, `helling_max`, and `bron` (free text).
- **Conditions** (`als`): `"ingraving"` or `"ophoging"`, or `{"type": "ingraving", "min": 0.3, "max": 5}`. The part is used only when the ground at its start lies above the design (cut) or below it (fill) by between `min` and `max` metres. Parts whose condition fails are skipped, so "in cut a ditch, otherwise a shoulder" is two parts with opposite conditions.
- **Targets:**
  - `breedte_doel`: a number is a fixed distance from the axis for the part's outer edge; a name is a line in the drawing, given with `CIV_CORRIDOR … DOELEN=name:handle`. The part ends where the section crosses that line.
  - `hoogte_doel`: a number is a level (m), `"maaiveld"` the existing ground at the outer edge, and a name a line (a 3D polyline gives its height). The part's slope changes to reach it, within `helling_min` … `helling_max` (%).
  - The kerb's upstand never changes: a footway with a level target behind a kerb starts at the kerb's top, and only its slope adapts ("keep the kerb upstand").
- **Errors:** they name the file, the side and the part number, in the interface language. `CIV_PROFIELTYPEN` shows them per file.

### Presets

| Name | What |
|---|---|
| `weg`, `fietspad`, `snelweg`, `dijk` | the 2026.44 templates, unchanged in geometry, now with build-ups (`weg`, `fietspad`, `snelweg`) |
| `stadsstraat` | 2 × 3,00 m asphalt with gutters and kerbs; left a footway, right a separating verge, a cycle path, a shoulder and a ditch |
| `woonstraat` | 2 × 2,50 m paving, parking lanes, kerbs and footways |
| `polderweg` | 2 × 2,50 m asphalt and shoulders; in cut a ditch, in fill a wider shoulder |
| `2x2-middenberm` | a 4,00 m central reserve, 2 × 2 lanes of 3,50 m and shoulders |

## Junctions (`CIV_KRUISPUNT`)

Input: a main road and a side road, each an axis with a design profile, and an assembly per road. Without `PROFIELTYPE`, the assembly comes from the road's newest corridor, else `weg`.

1. **The crossing.** The axes are intersected; the side road may stop short of the main road (it is extended 60 m). Arms exist where an axis runs on for more than the other road's width plus 5 m: three arms make a T junction, four an X junction.
2. **Corners.** Between every two neighbouring arms of different roads, an arc of radius `R` touches both carriageway edges. The edge is the kerb front where the assembly has a kerb, else the edge of the last lane, gutter or parking lane. The centre lies at `edge + R` from both axes, so curved axes work too. Corners are counted counter-clockwise from the main road's forward arm; `R=8;10` gives the first corner 8 m and the rest 10 m. Default `R` = 8 m: an **estimate** for two distributor roads inside a built-up area. Check the radius with the swept path of the design vehicle (the vehicle library, `GIS_VEHICLE`).
3. **Levels.** The main road keeps its own cross section through the junction. The side road is tied in to it: where the side road meets the main road's edge it takes the main road's height there (its cross slope and grade). From there to the arm's mouth its grade and cross section warp linearly to its own design section. The corners take a straight grade along their kerb return between the heights at their tangent points.
4. **Surface.** A TIN patch on `N-…-OG-TRIANGULATIE-G` with the junction's object data covers the paved area: the main road between its mouths, the side arms, and the corners up to the kerb returns. At each mouth it is the road's own section, so a corridor built from that station on (`TOT=` or `VAN=`, given per arm in the JSON result) fits it.
5. **Plan.** In 2D: the kerb returns (front and back of the kerb) as arcs, and the carriageway edges of the arms up to the mouths. In 3D: the kerb returns' foot and top. Give-way triangles (`VW-MARKERING_DWARS_DRIEHOEK`) cross the side road's approach lane: 0,50 m base, 0,60 m high, 0,25 m apart, an **estimate**.

## Roundabouts (`CIV_ROTONDE`)

A single-lane roundabout after CROW publication 126, *Eenheid in rotondes* (1998). The dimensions come from its chapter *Maatvoering ontwerpelementen* (CROW Kennisbank):

| | Inside built-up areas (`TYPE=binnen`, default) | Outside (`TYPE=buiten`) |
|---|---|---|
| Outer radius | 16,00 m (D = 32 m) | 18,00 m (D = 36 m) |
| Inner radius (central island with the overrunnable strip) | 10,50 m | 12,75 m |
| Carriageway width | 5,50 m | 5,25 m |
| Entry / exit radius, with splitter islands | 12,00 / 15,00 m | 12,00 / 15,00 m |
| Entry / exit lane width | 4,00 / 4,50 m | 4,00 / 4,50 m |
| Splitter island width | at least 2,50 m | at least 3,00 m |
| Splitter island length | about 14–15 m (14 m used) | about 14–15 m |
| Overrunnable strip | 3,00 m (22 m vehicle; 4,00 m for a 27 m vehicle) | 3,00 m |

CROW gives the overrunnable strip's width with the outside-built-up radii. Its use inside built-up areas is an **estimate**. Without splitter islands, CROW gives an entry radius of 8,00 m and an exit radius of 12,00 m; this tool always draws splitter islands.

These values are **estimates**:
- the carriageway's outward cross slope, 2,5 %;
- the strip's upstand, 0,05 m;
- the island's kerb, 0,15 m, and its mound, 0,35 m;
- the 15 m (`AANSLUITING`) over which the arm narrows from the entry and exit lanes to its road.

What it builds:

- **Arms:** every axis that runs through the centre gives two arms; an axis that ends there gives one. A roundabout has 3 to 6 arms.
- **Plan:** each arm has an exit lane (right, looking outward) and an entry lane (left) beside a splitter island. Arcs of the exit and entry radii join the lane edges to the outer circle. In 2D:
  - the outer kerb line, arcs included, between the mouths;
  - the circles of the overrunnable strip and the island (`VH-KANTOPSLUITING_ROTONDEBLOK`);
  - the splitter islands;
  - give-way triangles across every entry.
- **Levels:** the carriageway is a cone, highest at its inner edge, falling outward. Without `NIVEAU`, the inner edge lies so that the outer edge has the mean design height of the arms there. Each arm warps from the circle to its road's own section at its mouth.
- **3D:**
  - the carriageway and the arms as a TIN patch with the overrunnable strip (raised, with a 2 cm front face) on `N-…-OG-TRIANGULATIE-G`;
  - the island as a grass mound on `N-…-GR-GRAS-G`.
- **Result:** the JSON result lists each arm's mouth station and its corridor argument (`VAN=` / `TOT=`).

## Limits

- **Layers stop at their part's edge.** The foundation does not run on under the kerb or beyond the edge, as a wider foundation strip would.
- **Superelevation** tilts only the lanes. Gutters, parking lanes and the other parts keep their own slope.
- **Junction surfaces** cover the carriageway, not the footways, verges or cycle paths around the corners.
  - A main-road corridor built through the junction keeps its kerb and footway across the side road's mouth. Build it in two parts with `TOT=` and `VAN=` around the junction if that matters; the patch then covers the carriageway between them.
  - Junctions are drawn without superelevation (on straights).
- **Roundabouts:**
  - single-lane only, without bypasses or turbo lanes;
  - an axis must run through (or end at) the centre;
  - the cone is level all round, so a roundabout on a slope ties in over the arms' length.
- **Section views** take their height range from the ground and the design. A deep build-up in cut may run out of the frame.
- **No dialog.** There is no assembly window; assemblies are JSON files, and `CIV_PROFIELTYPE … PLAATS=` draws one to check it.

## Sources

- CROW publication 126, *Eenheid in rotondes* (1998); CROW Kennisbank, chapter *Maatvoering ontwerpelementen*: <https://kennisbank.crow.nl/public/gastgebruiker/WOBU/Eenheid_in_rotondes/Maatvoering_ontwerpelementen/7085>.
- CROW publication 351, *Ontwerpwijzer fietsverkeer* (2016): cycle path widths by peak-hour intensity.
- CROW, *ASVV 2021* (Aanbevelingen voor verkeersvoorzieningen binnen de bebouwde kom), and CROW, *Handboek wegontwerp*: the references for the estimated lane, gutter, footway, parking and verge widths and the corner radius; check them there.
- CROW, *Standaard RAW-bepalingen 2020*: material names of the build-ups.
- NEN-EN 13108-1 (asphalt concrete), NEN-EN 13242 (unbound granulates), NEN-EN 1338 (concrete paving blocks), NEN-EN 1339 (concrete flags), NEN-EN 1340 (concrete kerbs).
- NLCS 5.02 (DigiGO / Stichting NLCS, CC BY 4.0): the layers, and the kerb sizes 13/15 (`TROTTOIRBAND_130X150`) and 10/20 (`OPSLUITBAND_100X200`).
- The waterschap's *legger*: ditch dimensions (the defaults here are estimates).
