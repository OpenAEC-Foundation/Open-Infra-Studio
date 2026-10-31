# Survey codes, parcels, water and parking

Four tools that fill gaps in the civil workflow:

- survey point codes become linework and symbols;
- plots are split into parcels;
- water drops, catchments and ponds on the terrain;
- swept paths in reverse, and parking bays to NEN 2443.

The first three are on the **Civiel** tab (groups *Landmeten* and *Water*); swept paths and parking bays are in the **Library** group of the GIS tab, next to the vehicles. Every command takes `KEY=value` arguments and `ID=<name>`, which writes the outcome as JSON to `<config>/OpenCADStudio/open-infra/api/<name>.json`, as everywhere in Open Infra.

## Survey codes to linework and symbols

A surveyor gives every measured point a code: `LTRB1` for the first kerb line, `SBM Eik 30` for an oak with a 30 cm trunk. With codes the drawing makes itself: line codes become 3D polylines through their points, point codes become symbols, each on its NLCS layer.

| Command | What it does |
|---|---|
| `CIV_PUNTEN_IMPORT <file> CODES=JA [CODETABEL=<file>]` | imports the points as before, then draws the linework and symbols of their codes (a second undo step) |
| `CIV_CODES_TOEPASSEN [CODETABEL=<file>]` | does the same for the points already in the drawing (the selected ones, or all); with all points the earlier code output is replaced, not doubled |

### How a code is read

The description field of a point (the D of PNEZD/PENZD) is read word by word:

1. The **first word is the code**. Digits right after it are a **line number**: `LTRB1` and `LTRB2` are two kerb lines measured side by side. When a code itself ends in digits, the longest code wins: `LTR131` is code `LTR13`, line 1.
2. **Control codes** follow as separate words (`LTRB1 B`), or glued on with a separator (`LTRB1/B`).
3. The **other words are parameters**: `SBM Eik 30` gives the tree the attributes *soort = Eik* and *diameter = 30* as object data.

| Control (default) | Meaning |
|---|---|
| `B`, `ST` | begin a new line; an open line with the same code and number ends first |
| `E`, `END` | this point ends the line |
| `CLS`, `CL` | this point ends the line and closes it to its first point |
| `C`, `CONT` | continue (what a line code does anyway) |
| `BC`, `PC` | begin an arc: through this point and the next two, or up to `EC` |
| `EC`, `PT` | end of a longer arc |
| `OC` | a single point on an arc through its neighbours |

A line code without control codes continues the open line of its code and line number, or begins one. Arcs are drawn as short chords (at most 5° and 0.5 m) that pass exactly through the measured points; the height runs linearly between them. A line of one point is left out and counted in the message. Codes that are not in the table are listed in the message and in `onbekende_codes`.

### The code table

The table is `assets/landmeten/codes.json`. The first time codes are used, it is copied to `<config>/OpenCADStudio/open-infra/landmeten/codes.json`; from then on that copy is read, so a survey firm can put its own codes in it. `CODETABEL=<file>` reads another table for one command.

```json
{
  "stuurcodes": {"begin": ["B", "ST"], "einde": ["E", "END"], "sluiten": ["CLS", "CL"], "vervolg": ["C", "CONT"],
                 "boog": ["BC", "PC"], "einde_boog": ["EC", "PT"], "boogpunt": ["OC"], "scheiding": "/", "lijnnummers": true},
  "codes": [
    {"code": "LTRB", "soort": "lijn", "omschrijving": "trottoirband", "nlcs": "VH KANTOPSLUITING_TROTTOIRBAND"},
    {"code": "SBM", "soort": "punt", "omschrijving": "boom", "nlcs": "GR BOOM", "symbool": "boom", "attributen": ["soort", "diameter"]}
  ]
}
```

Per code:

- `soort`: `lijn` (3D polyline), `punt` (symbol), `tekst` (its parameters as a text) or `negeren` (a sighting point and the like);
- `nlcs`: the NLCS main group and object; the element follows from the kind (G for lines, S for symbols, T for texts) and the status from the settings. `laag` gives a plain layer name instead;
- `symbool`: one of `boom`, `put`, `kolk`, `lichtmast`, `paal`, `bord`, `brandkraan`, `afsluiter`, `kast`, `verkeerslicht`, `waterpeil`, `kruis`, `cirkel`, `vierkant`, `driehoek`. Each becomes a block `OI_SYMBOOL_<NAME>` drawn in paper millimetres and inserted at the drawing scale, at the point's height;
- `attributen`: the names of the parameters after the code.

### The default codes

There is no Dutch national standard for field codes, as far as we could find. NLCS prescribes the layers of the delivered drawing, not what a surveyor types in the field, and the surveying specifications of road authorities and municipalities that we found prescribe accuracy and the deliverable, not the codes. Survey firms use their own lists, or the starter list that comes with their instruments.

The defaults follow the starter code list that is shipped with many total stations and GNSS field computers sold in the Netherlands (a published list of about 50 codes):

- line codes begin with `L`, point codes with `S`;
- a line number follows the code directly;
- control codes are separate words.

The 47 codes of that list are in the table, each mapped to an NLCS object by Open Infra:

| Codes | NLCS object |
|---|---|
| `LTRB`, `LTR13`, `LTR18` (kerbs) | VH KANTOPSLUITING_TROTTOIRBAND |
| `LKAS`, `LKBT`, `LKKL`, `LKTG`, `LKPU`, `LKVH`, `LKOV` (pavement edges) | VH GESLOTENVERHARDING_ASFALT, …_BETON, OPENVERHARDING_STRAATBAKSTEEN, …_TEGEL, HALFVERHARDING_PUIN, GESLOTENVERHARDING, OG TERREIN_ONVERHARD |
| `LBGH`, `LBGB` (buildings) | OG BEBOUWING, OG BEBOUWING_BIJGEBOUW |
| `LHK`, `LRST`, `LSCH`, `LMU`, `LHG` (fences, walls, hedges) | IE TERREINAFSCHEIDING_KUNSTMATIG_HEKWERK, …_RASTERHEKWERK, …_SCHUTTING, …_MUUR, GR HAAG |
| `LKWA`, `LBSC`, `LDKR`, `LBRG`, `LDMW`, `LTAL`, `LBRK` | GW WATERGANG_INSTEEK, OB BESCHOEIING, KW KUNSTWERK_DUIKER, KW KUNSTWERK_BRUG, GC DAMWAND_STAAL, GW TALUD_KRUIN, OG BREUKLIJN |
| `LBMR`, `LGRN`, `LPKV`, `LOLO` | GR BOOM_RIJ, GR BEPLANTING, OG WEG_PARKEERPLAATS_INDELING, OG TOPOGRAFIE |
| `SBM` (tree), `SLM` (lamp post), `SVB` (sign), `SVL` (traffic light), `SPL` (post) | GR BOOM, OV LICHTMAST, VW VERKEERSTEKEN_BORD, IE MEUBILAIR_WEGMEUBILAIR_VERKEERSLICHT, IE MEUBILAIR_WEGMEUBILAIR_PAAL |
| `SRIOG`, `SRIOH`, `SRIOV` (manholes), `SSK`, `STK` (gullies) | RI GWA_RIOOLPUT, RI HWA_RIOOLPUT, RI DWA_RIOOLPUT, RI GWA_KOLK |
| `SBKR`, `SGAS`, `SWAT`, `SCAI` | KL WATER_BRANDLEIDING_BRANDHYDRANT, KL GAS_HULPSTUK_STRAATPOT, KL WATER_STRAATPOT, KL DATA_CAI_KAST |
| `SHC`, `SWP`, `SOPO`, `STXT`, `SDMY` | OG HOOGTEPUNT, OG WATER, OG TOPOGRAFIE, a text, ignored |

Choices to check for your project: gullies go to the mixed system (GWA) and the slope line `LTAL` to the crest; change them in your copy of the table.

## Parcels

`CIV_PERCELEN` splits a plot, a closed polyline, into parcels. `CIV_PERCEEL_INFO` reports area, perimeter and frontage.

| Argument | Meaning |
|---|---|
| `KAVEL=<handle>` | the plot; or select it |
| `STRAAT=<handle>` | the street line: dividing lines perpendicular to it; or select an open line with the plot |
| `RICHTING=<degrees>` | instead: dividing lines parallel to this direction (from east, counter-clockwise) |
| `N=<n>` | n parcels of equal area |
| `OPP=<m²>` | parcels of this area |
| `FRONT=<m>` | parcels of this frontage |
| `REST=laatste\|apart\|verdelen` | what is left after the last full parcel: added to it (default), a parcel of its own, or spread over all |
| `PREFIX=P`, `START=1` | parcel numbers P1, P2, … |

Without a street and a direction the dividing lines are perpendicular to the plot's longest side. Without a size: two equal parcels. With `OPP=` and `FRONT=` together both are minimums: each parcel gets at least that area and at least that frontage.

**How.** The dividing lines form a family: for a street, the line at station *s* runs through the street point at *s*, square to the street; its direction turns smoothly from vertex to vertex, so the area before it grows without jumps. For each parcel boundary the station is found by bisection on the exact area of the clipped polygon (to 10⁻¹¹ m), so the areas are exact to well under 0.01 m². This is unit-tested on a trapezoid, in RD coordinates and rotated, on an L-shaped plot and along a curved street. Beyond its ends the street runs on straight.

**Frontage** is the length of the parcel's edges that lie on the plot's front: the plot edges within 2 m of the edge nearest to the street and at most 30° from its direction. Without a street the frontage is the width across the dividing lines.

**Output:**

- the parcels as closed polylines on `KG-GRENS_PERCEEL`, element G. The status is N (new) unless `STATUS=` says otherwise;
- their number and area as labels (T);
- object data on each parcel: *perceel, oppervlakte_m2, omtrek_m, front_m, kavel, methode, richting*;
- a CSV table: `<config>/…/api/<ID>.csv` with `ID=`, otherwise beside the drawing. A Dutch interface writes semicolons and decimal commas, an English one commas and points.

`CIV_PERCEEL_INFO [PERCEEL=<h>;<h>] [STRAAT=<h>]` reports the selected parcels. Their frontage comes from the street when given, otherwise from their object data.

**Limits:**

- A concave plot cut in two by one dividing line gives a parcel in two parts, joined by a zero-width line; its area is still right.
- Along a street that bends sharply within the depth of the plot, the dividing lines may cross inside it.

## Water: drops, catchments and ponds

Neither NLCS 5.02 nor the BGT has objects for flow paths or catchments. These go on tool layers, like the height analysis: `OI-WATERDRUPPEL`, `OI-STROOMGEBIED`, `OI-STROOMGEBIED-AFVOERPUNT` and `OI-PLAS`.

### Water drop

`CIV_WATERDRUPPEL [PUNTEN=x,y;…]` (or click points) traces where a drop runs over the TIN:

- **In a triangle** it runs down the plane's steepest slope to an edge.
- **At an edge** it goes on into the next triangle. Where both triangles slope towards the edge (a gutter or a valley), it follows the edge down.
- **At a vertex** it takes the steepest way out: a triangle's slope or a descending edge.

It stops in a pit (a local minimum: water stands there), at the edge of the TIN, or on a level triangle. The path is a 3D polyline with its length, fall and end as object data, and a label with the end height.

Without a TIN in the drawing, an AHN TIN every `CEL=` m (default 1) is made in memory. It is not drawn.

### Catchments

`CIV_STROOMGEBIED` computes the catchment of each outlet:

- **Outlets:** picked points (`PUNTEN=x,y;…`, or click them and press Enter), or the points, blocks and circles on the layers of `LAAG=` (`*` wildcards). For example `LAAG=*KOLK*` takes the gullies of a coded survey or of GWSW.
- **Ground:** a grid of `CEL=` m (default 1), sampled from the TIN in the drawing, or fetched from the height source with `BRON=AHN`. The height source is AHN in the Netherlands, worldwide terrain tiles elsewhere.
- **Method:**
  - Depressions are filled with Priority-Flood (Barnes, Lehman and Mulla, 2014, *Computers & Geosciences* 62, 117–127).
  - Each cell drains to its steepest lower neighbour of the eight (D8: O'Callaghan and Mark, 1984, *Computer Vision, Graphics and Image Processing* 28, 323–344).
  - Level and filled cells drain along the flooding order, so every cell reaches an outflow.
  - Each cell belongs to the first outlet downstream of it.
  - The grid border and cells without a height are where water leaves the model. In AHN these are water and buildings.
- **Snapping:** an outlet moves to the cell with the most flow within `SNAP=` m (default 2), as a gully catches the gutter's flow.
- **Output:** the outline of each catchment (cell edges, lightly simplified), its area in m² as a label and in the JSON, and the outlet as a circle.

### Ponds

With `PLASSEN=ja` (the default) `CIV_STROOMGEBIED` also draws the ponds; `CIV_PLASSEN` draws only them. The ponds are the filled depressions: where water stays on the street after rain until it seeps away or is pumped off. Each pond gets its outline, its deepest point, depth (m), volume (m³), area and water level, as object data, a label and JSON.

`MINDIEPTE=` (default 0.05 m) and `MINOPP=` (default 2 m²) leave out the puddles. These two defaults are Open Infra's choice, not standard values.

**Limits:**

- This is the surface only. Rainfall, infiltration, the sewer's capacity and time are not modelled; the ponds are the volumes that can stand, not what a given storm leaves behind.
- AHN's ground model has no heights under buildings, so water that reaches a building leaves the model there.
- Kerbs narrower than the cell are smoothed away: use a TIN from survey points with breaklines (`CIV_TIN BRON=PUNTEN`) for streets.

## Swept paths in reverse

`GIS_VEEGPAD <vehicle> [interval] ACHTERUIT [KNIK=<degrees>] [PAD=x,y;x,y;…]` (also `GIS_SWEPT_PATH … REVERSE`) reverses a vehicle along the path. The path is a selected line, picked points, or `PAD=` in project coordinates.

**Kinematics.** The rear axle of the last unit (the trailer's axle group, a tandem acting as one axle) runs along the path backwards; the unit points against the direction of travel. The units ahead follow from it:

- The coupling of the unit ahead sits on the kingpin of the unit behind, which is known.
- That unit's own axle only moves along its heading (no side slip), so it is a tractrix of the coupling.
- The tractrix is stable in one direction of travel only, so it is integrated in that direction:
  - with the coupling ahead of the axle (a fifth wheel), it is integrated from the end of the path to its start;
  - with the coupling behind the axle (a drawbar hitch), from the start to the end.
- Both give the same no-slip motion.

Unit tests check:

- a car's rear axle stays on the path, with steering tan δ = wheelbase / radius;
- a semitrailer reversing round a circle settles at the no-slip articulation asin(wb_t/R_k) − asin(e/R_k), where R_k is the kingpin radius, wb_t the trailer's kingpin-to-axle distance and e the fifth wheel's offset ahead of the tractor's axle;
- the couplings stay together, also with a drawbar trailer.

**Jackknifing.** The message gives the largest articulation (the angle between two units) and where it occurs. Reversing, it is checked against a limit:

- `KNIK=` when given;
- otherwise, for a vehicle with one trailer and a known turning radius, its jackknife angle. That is the steady articulation with the tractor at full lock: the largest the driver can still hold or reduce. Full lock is estimated from the vehicle's outer turning radius, assuming the outer front corner of the tractor sets it. This is an estimate.
- otherwise 70°, also an estimate (no standard gives one value).

Above the limit the message warns *SCHAREN / JACKKNIFE*, and the JSON has `scharen: true`.

Use a smooth path for reversing (arcs): a sharp corner turns the last unit over about a metre.

## Parking bays

`GIS_PARKEERVAKKEN` (also `GIS_PARKING`) lays out rows of bays along a line, the centre line of the aisle:

| Argument | Meaning |
|---|---|
| `HOEK=90\|80\|70\|65\|60\|45\|30\|LANGS` | parking angle; LANGS (0) is parallel parking. Default 90 |
| `KLASSE=openbaar\|intensief\|nietopenbaar\|stalling` | NEN 2443 facility class: table 4A (public), 4A with b ≥ 2.50 m (intensive public use), 4B (non-public), 4C (storage). Default openbaar |
| `BREEDTE=<m>` | bay width: a column of the table (2.30 … 2.55). Default 2.50, parallel 2.00 |
| `ZIJDE=beide\|links\|rechts` | rows on both sides (default) or one |
| `LIJN=x,y;…` | the line in project coordinates; or select a line, or pick points |
| `START=<n>` | first bay number |
| `MET_VEEGPAD=<vehicle>` | check one bay with the swept path of a vehicle from the library (`ja` = the library car closest to 4.5 m long) |
| `VAK=<n>` | which bay to check (default the middle one of the first row) |

**Output:**

- the bays as closed polylines on `OI-PARKEREN` / `OI-PARKING`, with number, angle, width, depth, aisle, class and source as object data;
- the numbers on `OI-PARKEREN-TEKST` / `OI-PARKING-TEXT`;
- the aisle edges on `OI-PARKEREN-RIJBAAN` / `OI-PARKING-AISLE`.

Inside a bend, bays that would overlap earlier ones are left out.

### Dimensions (NEN 2443:2013)

From NEN 2443:2013 *Parkeren en stallen van personenauto's op terreinen en in garages*, normtekst 5.3.8 and tables 4A, 4B and 4C. The values are those of the reproduction in annex 1 of the municipality of Maastricht's *Beleidsregel parkeren en duurzaamheid Belvédère* (lokaleregelgeving.overheid.nl, CVDR654663). We could not check them against the standard itself, which is not public.

Per angle and bay width b: P1 is the depth of a row square to the aisle, W the aisle width.

| Angle | P1 (m) | W, table 4A (public), b = 2.40 / 2.45 / 2.50 / 2.55 | W, table 4B (non-public), b = 2.30 / 2.35 / 2.40 / 2.45 / 2.50 | W, table 4C (storage), b = 2.30 … 2.50 |
|---|---|---|---|---|
| 30° | 4.19 | 3.80 (b ≤ 2.50) | 3.00 | 2.75 |
| 45° | 4.95 | 3.80 (b ≤ 2.50) | 3.00 | 2.75 |
| 60° | 5.38 | 3.80 | 3.70 / 3.40 / 3.10 / 3.00 / 3.00 | 3.40 / 3.10 / 2.90 / 2.75 / 2.75 |
| 65° | 5.44 | 3.87 / 3.80 / 3.80 / 3.80 | 4.17 / 3.80 / 3.60 / 3.20 / 3.00 | 3.86 / 3.60 / 3.30 / 3.00 / 2.75 |
| 70° | 5.46 | 4.43 / 4.08 / 3.82 / 3.80 | 4.66 / 4.36 / 4.07 / 3.80 / 3.50 | 4.41 / 4.11 / 3.80 / 3.50 / 3.20 |
| 80° | 5.37 (5.38 for b < 2.40) | 5.53 / 5.23 / 4.87 / 4.47 | 5.86 / 5.47 / 5.16 / 4.85 / 4.55 | 5.50 / 5.17 / 4.85 / 4.54 / 4.17 |
| 90° | 5.13 | 6.67 / 6.33 / 6.00 / 5.67 | 7.00 / 6.66 / 6.33 / 6.00 / 5.66 | 6.62 / 6.29 / 5.95 / 5.62 / 5.29 |

- **Minimum bay width** (5.3.8.1, for garages; applied here to every layout): 2.50 m for intensive public use, 2.40 m for public garages, 2.30 m for non-public and storage garages.
- **Parallel parking** (5.3.8.2, 5.3.8.3):
  - bay width at least 1.80 m with exit space beside the row, 2.00 m along walls;
  - bay length at least 6.25 m on car parks and in public garages, 5.75 m in non-public and storage garages, plus 0.50 m for bays entered forwards;
  - aisle at least 4 m, 3.50 m in storage and non-public facilities.
- An angled bay is drawn as a parallelogram: its sides along the bay, its front and back parallel to the aisle, b / sin α long along the aisle and P1 deep.

### The swept-path check

`MET_VEEGPAD=` drives the vehicle into one bay. The rear axle of its last unit is guided, so the vehicle ends square in its bay. The manoeuvre is Open Infra's choice, not part of NEN 2443:

- **90° bays:** driven past, then reversed in on a quarter circle. The radius is as large as lets the front's swing stay 0.1 m off the far side of the aisle, and never tighter than the vehicle's full lock. It ends nose out, 0.3 m off the back of the bay.
- **Angled bays:** forward in on an arc from the far side of the aisle, 0.25 m clear of its edge. The radius is the largest the aisle allows, at most 15 m. It ends 0.3 m off the slanted back of the bay.
- **Parallel bays:** reversed in along an S of two arcs 5 % wider than full lock, ending 0.3 m from the rear of the bay.
- **Full lock:** estimated from the vehicle's outer turning radius, else a rear-axle radius of 3 m (an estimate).

The other bays hold the same vehicle, parked the same way; in parallel bays they stand in the middle of theirs. The check reports:

- the deepest overlap of the swept vehicle with a parked one (`indringing_m`) and in which bay;
- how far it crosses other bays' lines (`over_vaklijnen_m`);
- whether it ends inside its bay.

*Past* (fits) means no overlap with a parked vehicle and the final position inside the bay. The swept path itself is drawn on the swept-path layers.

With the defaults a mid-size car from the library (4.28 m long, 2.62 m wheelbase) fits a 2.50 m bay at 90° with a 6.00 m aisle in reverse, and a 6.25 m parallel bay along the S. These cases are unit-tested.

## Tests

- `cargo test --lib`:
  - code reading, figures, arcs and the default table against NLCS;
  - parcel areas to the hundredth (equal, by area with the three remainder options, by frontage, both minimums, parallel to a direction, an L-shaped plot, a curved street);
  - water drops on a plane, a bowl and a valley;
  - Priority-Flood draining every cell, a filled pit's depth and volume, two catchments on a ridge, outlines with a hole and a pinch;
  - reverse kinematics (car, semitrailer steady state, jackknife, drawbar trailer);
  - NEN 2443 dimensions, layouts and the parking check.
- `tests/e2e/overig_e2e.py` (port 18818, own configuration folder) runs it all in Open CAD Studio around the Grote Kerk in Dordrecht and takes captures in `target/e2e-overig/`.
