# Plan production: sheets and labels

The Civiel tab's group **Bladen** turns a design into contract drawings (bestektekeningen):

| Button | Command | What it does |
|---|---|---|
| **Bladen maken** | `CIV_BLADEN` | A sheet set along an alignment: one paper-space layout per sheet with the plan strip and the length profile of that stretch. |
| Bladen naar PDF | `CIV_BLADEN_PDF` | Every sheet to its own PDF. |
| Labels | `CIV_LABELS` | Stationing, curve data, PVIs, grades, spot heights, slope arrows and point labels that know their source. |
| Labels bijwerken | `CIV_LABELS_BIJWERKEN` | Redraws the labels whose source changed or moved, and removes those of deleted sources. |

## Sheets along an alignment (`CIV_BLADEN`)

```
CIV_BLADEN [AS=<handle>] [FORMAAT=A0|A1|A2|A3] [SCHAAL=1:1000] [VSCHAAL=10|1:100]
           [OVERLAP=<m>] [PROJECT=<text>] [ONDERWERP=<text>] [NUMMER=<text>] [STATUS=B|N|T|V] [ID=<name>]
```

- **AS**: the alignment; from the ribbon, select one first.
- **FORMAAT**: landscape ISO 216 A-format, A1 by default.
- **SCHAAL**: plan scale (`1:1000`, `1000` or `1/1000`); the settings' drawing scale by default.
- **VSCHAAL**: vertical exaggeration of the profile (`10`), or the vertical scale (`1:100`). The settings' exaggeration by default.
- **OVERLAP**: how much of the neighbouring sheets each plan shows beyond its match lines, in metres. Default: 20 mm on paper (20 m at 1:1000, 10 m at 1:500).
- **PROJECT**, **ONDERWERP**, **NUMMER**: title block texts. Sheet *n* gets drawing number `<NUMMER>-0n`; without `NUMMER` it is `OI-BL-0n`.

A sheet set is an alignment plus a paper size. Run the command again with the same alignment and paper size, and the set is replaced (layouts and model-space parts). An A1 set and an A3 set of the same alignment live side by side.

### What a sheet holds

Each sheet is a real paper-space layout, named `<alignment>_<paper>_blad_01`, `…_02` (no spaces, so the plot prompts can take them). Its page setup is the paper size in millimetres at 1:1. It holds:

- **Frame:** the trim line and the drawing frame with a 20 mm binding margin (as on `GIS_TEKENBLAD`).
- **Plan (SITUATIE):** a viewport over the full width of the drawing area, at the plan scale. It is turned (view twist) so that the stretch of the alignment runs from left to right.
- **Length profile (LENGTEPROFIEL):** a second viewport below, as wide, at the same horizontal scale.
- **Match-line references:** "ZIE BLAD n" inside the plan, beside each match line, on the side of the other sheet.
- **Title column on the right:** 185 mm wide (125 mm on A3). It holds:
  - the NLCS title block of `GIS_TEKENBLAD` (drawing number, status, coordinate system, format, scale, date, subject, project, sources);
  - "BLAD n VAN N" and the stretch's stations;
  - the scale bar, and the profile's horizontal and vertical scales;
  - the north arrow, turned as the plan is;
  - the legend of the NLCS layers drawn in this session.

In model space, the command draws:

- **View frames:** the outline of every sheet's plan along the alignment, with "BLAD n VAN N". They sit on `AL-TEKENBLADGRENS`, a layer that does not plot.
- **Match lines (aansluitlijnen):** across the alignment at every sheet boundary, with "AANSLUITLIJN 0+500", on `AL-BLADVERWIJZING_AS`.
- **One profile view per sheet:** in a column below the project area, at the plan scale. Each holds the band rows (Stationering, Maaiveld, Ontwerp, Helling / boog, Alignement) and the height grid with NAP labels. In the graph:
  - the ground line (`GW-MAAIVELD`);
  - the design line (`GW-HOOGTE_AFWERKHOOGTE`);
  - the PVIs with their tangents (`ZZ-CONSTRUCTIELIJN`) and heights;
  - the tangent points (`AM-TANGENTPUNT`).

  The band shows the grades between the vertical curves, and each curve as a box with crest or sag, R and L. The ground comes from the TIN in the drawing, or from AHN without one, as for `CIV_LENGTEPROFIEL`. The design comes from the newest `CIV_ONTWERPPROFIEL` of the alignment.

The parts carry object data (`bladen`, `blad`, `van`, `tot`, `aansluitlijn`), and the model-space parts are in the drawing's object index (kind `bladen`).

### How the alignment is divided

1. Each sheet's plan window is the plan viewport at the plot scale.
2. A stretch of the alignment, plus half the overlap at both ends, must fit inside that window, turned along the stretch's chord. Along the alignment 2 % of the window stays free at each end, across it 12 % on each side.
3. A stretch is also no longer than the profile graph is wide.
4. The command first takes each stretch as long as it fits, ending on a round station (10 m at 1:1000, 5 m at 1:500). It then tries to spread the same number of sheets evenly; when every even stretch fits, it uses those.
5. A tight curve gets shorter sheets. A hairpin is split at its apex.

The vertical exaggeration you ask for is lowered when a sheet's profile would not fit its viewport. It drops to the next of 50, 25, 20, 10, 5, 4, 2 or 1, so the vertical scale stays a round one. The message says so. When even 1× does not fit, the profile is cut off at the top, and the message warns about it.

### PDF (`CIV_BLADEN_PDF`)

```
CIV_BLADEN_PDF [AS=<handle> | NAAM=<alignment name>] [MAP=<folder>] [ID=<name>]
```

Plots every sheet of the alignment (or of all sheet sets) to `<layout>.pdf`, through Open CAD Studio's `-PLOT`. The settings are: the layout at 1:1, centred, with lineweights, on the set's paper size, landscape. The files go to `<drawing>_bladen/` beside the drawing, or to MAP.

`-PLOT` takes its answers as words separated by spaces, so the folder may not contain a space. When it does, the plugin's own folder (`<config>/OpenCADStudio/open-infra/bladen`) is used instead. If that one also contains a space, the command asks for `MAP=`.

## Labels (`CIV_LABELS`)

```
CIV_LABELS [AS=<handle>] [SOORT=stationering;bogen;pvi;hellingen;hoogtes;afschot;punten|alles]
           [PUNTEN=x,y;x,y] [RASTER=<m>] [OPPERVLAK=B|N] [SCHAAL=…] [ID=<name>]
```

| SOORT | Source | What is drawn | Default layer |
|---|---|---|---|
| `stationering` | alignment | Ticks every 20 m; every 100 m a long tick with the station (`0+100`, or hectometres `0,1`) | `AM-KILOMETRERING` |
| `bogen` | alignment | BB/EB/HP with their station; R and L in the middle of each curve, on the inside | `AM-TANGENTPUNT`, `AM-AS` |
| `pvi` | design profile | PVI mark and line; station and height; crest or sag curve with R and L | `AM-AS` |
| `hellingen` | design profile | The grade (%) along each straight, above the line | `AM-AS` |
| `hoogtes` | TIN | Spot heights at `PUNTEN` (project coordinates), or on a grid of `RASTER` m | `ZZ-HOOGTELABEL` |
| `afschot` | TIN | Slope arrows pointing downhill on a grid of `RASTER` m, with the slope in % | `ZZ-AANPIJLING` |
| `punten` | survey points | Number, height and code beside each point (the selected ones, else all) | `OG-HOOGTEPUNT` |

- **Defaults without SOORT:** a selected alignment gets `stationering` and `bogen`, plus `pvi` and `hellingen` when it has a design profile. Selected survey points get `punten`.
- **Terrain:** `OPPERVLAK=N` labels the design TIN instead of the existing ground.
- **Repeated runs:** labelling the same source again replaces its earlier labels. Spot heights at picked points are added, never replaced.

### Style (`labels.json`)

The style is `<config>/OpenCADStudio/open-infra/labels.json`. The first `CIV_LABELS` writes it with the defaults, so you can edit it:

```json
{
  "tekst_mm": 1.8,
  "decimalen": { "hoogte": 2, "station": 2, "helling": 2, "straal": 2, "lengte": 2 },
  "stationering": { "klein_m": 20, "groot_m": 100, "formaat": "station", "voorvoegsel": "", "laag": "AM KILOMETRERING" },
  "bogen": { "voorvoegsel_straal": "R = ", "voorvoegsel_lengte": "L = ", "laag": "AM AS", "laag_punten": "AM TANGENTPUNT" },
  "pvi": { "voorvoegsel": "PVI ", "laag": "AM AS" },
  "hellingen": { "laag": "AM AS" },
  "hoogtes": { "voorvoegsel": "", "raster_m": 20, "laag": "ZZ HOOGTELABEL" },
  "afschot": { "raster_m": 20, "pijl_mm": 8, "min_procent": 0.1, "laag": "ZZ AANPIJLING" },
  "punten": { "naam": true, "hoogte": true, "code": true, "laag": "OG HOOGTEPUNT" }
}
```

- **Sizes:** text heights and arrow lengths are millimetres on paper. The plot scale (`SCHAAL`, or the settings' drawing scale) turns them into drawing units. Make labels at the scale of the sheets they go on.
- **Layers:** a layer is an NLCS main group and object (`"AM KILOMETRERING"`). The status and discipline come from the settings.
- **Missing keys** keep their default.

### Labels that follow their source (`CIV_LABELS_BIJWERKEN`)

Open CAD Studio has no reactors, so a label cannot follow its source by itself. Every label entity carries object data (`OPENINFRA`) instead:

- `label`: its kind;
- `bron`: the handle of its source, or `tin:B` / `tin:N` for a terrain;
- `vingerafdruk`: a fingerprint of the source's geometry and object data;
- `plek`: where a spot height sits.

The labels are listed per source in the drawing's object index (`OPENINFRA/INDEX`, kind `labels`), so updating works at any drawing size. `CIV_LABELS_BIJWERKEN` compares each source with its fingerprint:

- **Changed or moved:** the labels are drawn again, with the same spots or grid. PVI and grade labels follow the design line, wherever it has been moved to.
- **Deleted:** the labels are removed.
- **Unchanged:** the labels are left alone.
- **Labels you deleted by hand:** they are forgotten.

The result (`ID=`) reports `bijgewerkt`, `verwijderd` and `ongewijzigd`.

## Limits

- **Viewports cannot be clipped.** The plugin API has no clipped (polygonal) viewports, so a sheet's plan is the rectangular window, overlap included. The match lines and "ZIE BLAD" references mark where the next sheet takes over.
- **Plot styles are not set.** `CIV_BLADEN_PDF` plots without them, with the layer lineweights. The plot runs in Open CAD Studio's window: it must be the active drawing.
- **The title block is plain lines and texts,** as on `GIS_TEKENBLAD`, not a block with attributes: edit its texts directly, or run the command again with other `PROJECT`, `ONDERWERP` or `NUMMER`.
- **Model-space texts do not turn with the plan.** In a turned plan view, texts drawn in model space with a fixed rotation (street names, spot heights) appear turned on the sheet. The stationing and curve labels run along the alignment, so they read well.
- **Labels follow only through `CIV_LABELS_BIJWERKEN`.** They do not move while you edit. Labels made by the other Civiel commands (the fixed texts of `CIV_AS`, `CIV_ONTWERPPROFIEL`) do not follow at all.

## Sources

- NLCS 5.02 (DigiGO / Stichting NLCS, CC BY 4.0), the objects used:
  - `AL TEKENBLADGRENS`, `AL BLADVERWIJZING_AS`, `AL TEKENBLAD_*` (frame, title block, viewport, scale bar, explanation);
  - `AL ORIENTATIE`, `AL PROFIELBALK`, `AL LEGENDA`;
  - `AM KILOMETRERING`, `AM TANGENTPUNT`, `AM AS`;
  - `GW MAAIVELD`, `GW HOOGTE_AFWERKHOOGTE`;
  - `ZZ HOOGTELABEL`, `ZZ AANPIJLING`, `ZZ CONSTRUCTIELIJN`, `OG HOOGTEPUNT`.

  NLCS has no dedicated object for slope arrows; `ZZ AANPIJLING` (pointer arrow) is the closest.
- **Sheet sizes:** ISO 216 A-series (A0 1189 × 841 mm to A3 420 × 297 mm).
- **Title block and frame:** the same as `GIS_TEKENBLAD` (185 mm title column, 20 mm binding margin); NLCS text classes 1.8 / 2.5 / 3.5 / 5 mm.
- **Open Infra's own choices (no standard behind them):**
  - stationing ticks every 20 m with labels every 100 m;
  - an overlap of 20 mm on paper;
  - free margins of 12 % across and 2 % along the plan window;
  - round match-line stations;
  - the 125 mm title column on A3.
