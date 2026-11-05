# Grading: feature lines, plateaus, slopes, exact earthwork and mass haul

The **Grondwerk / Grading** group on the Civiel tab adds the tools to design a site's ground: 3D feature lines, building plot plateaus and slopes that run out to the existing ground, one design surface for all of them ("bouwrijp maken"), exact cut and fill between two TINs, and the mass haul diagram of a corridor.

Everything works on drawings in metres, millimetres or centimetres, at any location: the existing ground is the TIN of status B in the drawing (`CIV_TIN`), otherwise the height source of the project (AHN in the Netherlands, worldwide terrain elsewhere). Every command takes `KEY=value` arguments and `ID=<name>` (see the main README); `CIV_HELP GRADING` lists them on the command line.

## Ribbon

| Button | Command | From the ribbon |
|---|---|---|
| **Kantlijn / Feature line** | `CIV_KANTLIJN` | The selected (poly)line becomes a feature line on the ground, with points where the grade of the ground changes. |
| **Plateau** | `CIV_PLATEAU` | The selected closed polyline becomes a plateau at the level of the settings (Civiel settings, *peil*), or at the heights of a selected feature line. |
| **Talud / Slope** | `CIV_TALUD` | Select a line, then click the side: a slope to the ground. |
| **Hoogtepunt / Height point** | `CIV_KANTLIJN_PUNT` | Select a feature line, then click points on it: each becomes a height point (height interpolated). Enter ends. |
| **Grondverzet TIN / Volume TIN** | `CIV_VOLUME_TIN` | Exact cut and fill of the design against the existing ground, with the map. |
| **Grondbalans / Mass haul** | `CIV_GRONDBALANS` | The mass haul diagram of the selected axis. |

There is no dialog API for plugins, so the values that need typing (levels, grades, falls) go in as command arguments.

The MCP server offers the same as `civ_feature_line`, `civ_feature_line_edit`, `civ_plateau`, `civ_slope`, `civ_volume_tin` and `civ_mass_haul`.

## Feature lines (`CIV_KANTLIJN`)

A 3D polyline on `N-…-OG-BREUKLIJN-G` (status N unless `STATUS=` is given), with object data `kantlijn`, `bron`, `punten`, `lengte_m`, `z_min`, `z_max` and `hellingen` (grade per segment, %). Labels on `…-OG-BREUKLIJN-T`: the level at each corner and the grade of each segment with an arrow down the slope (on lines of more than 30 points only the ends get a level).

| Argument | Meaning |
|---|---|
| `LIJN=<handle>` / `PUNTEN=x,y[,z];…` | The line: one in the drawing (line, polyline with arcs, 3D polyline), or points in project coordinates. Without either: the selection. |
| `HOOGTE=MAAIVELD` (default) / `ONTWERP` / `EIGEN` | Heights from the existing ground, from the design surface (N), or the line's own heights. |
| `TUSSENPUNTEN=ja` or `=<m>` | With a surface: sample every `STAP` (or the given) metres and keep the points where the grade changes by more than 5 mm (corners always stay). |
| `START=<m> HELLING=<%>;<%>;…` | A start level and a grade per segment (rising along the line > 0; the last grade repeats). |
| `PEILEN=<m>;;<m>` or `PEILEN=<n>:<m>;…` | Fixed levels at corners (empty = interpolate; or corner number from 1 and level). Corners in between follow linearly by length; before the first and after the last fixed level the line stays level. |
| `GESLOTEN=ja`, `NAAM=` | Close the line; its name (default `Kantlijn n`). |

### Edits (the line keeps its handle)

| Command | What it does |
|---|---|
| `CIV_KANTLIJN_PEIL KANTLIJN=<h> PUNT=<n>` or `BIJ=x,y` `Z=<m>` | One corner (by number from 1, or the corner nearest to a point) to a level. |
| `CIV_KANTLIJN_VERHOOG KANTLIJN=<h> DZ=<m>` | Everything up (or down with a negative value). |
| `CIV_KANTLIJN_PUNT KANTLIJN=<h> BIJ=x,y [Z=<m>]` | A height point on the line at the foot of `x,y`, at `Z` or interpolated. On a corner it sets that corner's level. |
| `CIV_KANTLIJN_HELLING KANTLIJN=<h> VAN=<n> TOT=<n> [HELLING=<%>]` | Constant grade between two corners: from the first at the given grade, or straight between the two levels. |

The labels are redrawn after each edit, and the object data gets `bewerkt=` with the last change. If the host refuses the change in place, the line is drawn anew and the JSON returns its new handle.

## Plateaus (`CIV_PLATEAU`)

A plot at a level with an optional fall, sloping to the existing ground: a cut slope where the plateau edge lies below the ground, a fill slope where it lies above.

| Argument | Meaning |
|---|---|
| `LIJN=` / `PUNTEN=` / selection | The closed outline (a polyline, a closed feature line, or three or more points). |
| `PEIL=<m>` | The level at the reference point. Without it a feature line keeps its own heights. |
| `PEIL=AUTO [MIN=] [MAX=] [FACTOR=]` | Find the level where cut and fill balance: cut = `FACTOR` × fill, where `FACTOR` is the m³ of in-situ cut needed per m³ of compacted fill (default 1). The search uses Brent's method between `MIN` and `MAX` (default: the ground under the outline ± 1 m) to 5 mm. Each trial builds the same slopes to the ground and measures cut and fill as the plateau itself does. If there is no balance within the bounds, the error gives the best bound and its surplus. The calculation is in `rekenkern::balans` (issue #68). |
| `AFSCHOT=<%> RICHTING=<N…NW or degrees>` (or `AFSCHOT=<%>;<dir>`) | Fall of the plateau, downhill towards the compass direction (N, NO/NE, O/E, ZO/SE, Z/S, ZW/SW, W, NW, or degrees clockwise from north). |
| `PEILPUNT=x,y` | Where `PEIL` applies (default: the centroid of the outline). |
| `TALUD_AF=1:n`, `TALUD_OP=1:n`, `TALUD=1:n` | Cut and fill slopes (`TALUD` sets both); `1:2,5`, `2.5` and `1:2.5` all work. |
| `REIKWIJDTE=<m>` | How far a slope may search for the ground (default 50 m). |
| `STAP=<m>` | Spacing of the slope lines along the edge (default 1 m). |
| `NAAM=`, `SAMENVOEGEN=nee` | Name (default `Plateau n`); keep the surface apart instead of joining the design surface. |

What it draws:
- the plateau edge as a feature line on `OG-BREUKLIJN` (not again when the outline already was a feature line);
- the daylight line: the toe of fill slopes on `GW-TALUD_TEEN`, the crest of cut slopes on `GW-TALUD_KRUIN`;
- slope hatching from the top of the slope down, alternating long (`GW-TALUDARCERING_LANG`) and one-third (`GW-TALUDARCERING_KORT`) lines every 2.5 mm on paper;
- the name and level at the reference point, and an arrow with the fall on `GW-HOOGTE_AFWERKHOOGTE-T`;
- its triangles in the combined design surface (below).

The JSON gives the plateau and slope areas, the daylight length, the number of slope lines that found no ground (`zonder_aansluiting`), and the plateau's own cut and fill against the ground (exact on a TIN, on a 0.5 m grid over AHN).

### How the slopes are built

Every point of a slope belongs to the nearest point of the edge, and the slope runs square to the edge from there. At an outside corner the slopes form a cone (a fan of slope lines every 10°); at an inside corner the slopes of both sides meet on the line where both edges are equally far away, so nothing overlaps and nothing needs cleaning up. Each slope line runs at the cut or fill slope (chosen at its start) until it meets the ground, found every 0.25 m and interpolated in between. The plateau inside the edge is triangulated with the same edge points, so plateau and slopes share their edges.

## Slopes (`CIV_TALUD`)

A slope from an open (or closed) line on one side:

| Argument | Meaning |
|---|---|
| `LIJN=` / `PUNTEN=` / selection | The edge. A feature line or 3D line keeps its heights; a 2D line gets `RANDPEIL=<m>`, or the ground's height at its corners. |
| `ZIJDE=RECHTS` (default) / `LINKS` | Looking along the line. |
| `NAAR=MAAIVELD` (default) | To the ground, with `TALUD_AF` / `TALUD_OP` (or `TALUD`). |
| `NAAR=AFSTAND BREEDTE=<m> TALUD=1:n [RICHTING=OP]` | Over a fixed width, down (default) or up. |
| `NAAR=PEIL PEIL=<m> TALUD=1:n` | To a level, up or down as needed. |

## One design surface ("bouwrijp maken")

All gradings join one design surface: meshes on `N-…-OG-TRIANGULATIE-G` with object data `terrein=Bouwrijp`, `deel=kern|ophoging|ontgraving` (plateau, fill slope, cut slope) and the names of the gradings in it. Every new grading is laid over the existing surface exactly (both triangulations are cut into convex pieces); where they overlap:

| New \ existing | Plateau | Fill slope | Cut slope |
|---|---|---|---|
| **Plateau** | the new one | the highest | the lowest |
| **Fill slope** | the highest | the highest | the new one |
| **Cut slope** | the lowest | the new one | the lowest |

So a fill slope only raises what it runs over and a cut slope only lowers it: the fill slope of a higher plot runs onto a lower neighbour until it is below that plot's level. The result has no T-junctions, so its contours (`CIV_HOOGTELIJNEN OPPERVLAK=N`), height analysis (`CIV_ANALYSE OPPERVLAK=N`) and volumes work as on any TIN. The object index keeps the surface under `bouwrijp` and `tin`/`N`. With `SAMENVOEGEN=nee` a grading becomes a surface of its own.

## Exact earthwork between two TINs (`CIV_VOLUME BASIS=TIN`, `CIV_VOLUME_TIN`)

Inside one triangle of each TIN both surfaces are planes, so the height difference is linear over every convex piece where two triangles overlap. The command overlays the design and the existing ground (status B), splits each piece where the design crosses the ground, and integrates exactly (area × difference at the centroid): the prismoidal volume, whatever the two triangulations look like. Only the part where both surfaces exist counts.

| Argument | Meaning |
|---|---|
| `ONTWERP=` | Default: the selected faces, else the whole design surface (N), else the level. `BOUWRIJP`: only the combined grading surface. `PEIL` (or `PEIL=<m>`): a level. |
| `LOSFACTOR=` | Bulking: loose m³ per m³ in situ (default 1). |
| `VERDICHTING=` | Compaction: loose m³ needed per m³ of compacted fill (default 1). |
| `BANDEN=`, `KAART=nee` | Colour bands of the map (half cut, half fill; default from the settings), or no map. |
| `VERGELIJK=ja` | Also the grid volume at `CEL` metres, for comparison. |

The balance is in loose m³: `cut × LOSFACTOR − fill × VERDICHTING` (surplus > 0, shortage < 0). Both factors are project values from the soil investigation; Open Infra has no defaults other than 1.

The map lies in plan (z = 0): cut bands from yellow to red on `OI-GRONDVERZET-AF-nn`, fill bands from cyan to blue on `OI-GRONDVERZET-OP-nn`, split exactly at the band limits (a nice step: 1, 2 or 5 × 10ⁿ m); differences under 1 mm stay uncoloured. The legend beside it gives each band's area and volume; the summary above it the totals, factors and balance. JSON: `afgraven_m3`, `ophogen_m3`, `afgraven_m2`, `ophogen_m2`, `overlap_m2`, `afgraven_los_m3`, `ophogen_nodig_m3`, `balans_m3`, `banden` and, with `VERGELIJK`, `raster_afgraven_m3` / `raster_ophogen_m3`.

If several design surfaces overlap in plan (for example a corridor running through the grading surface), `ONTWERP=N` counts both; use `ONTWERP=BOUWRIJP` or select the faces.

## Mass haul (`CIV_GRONDBALANS`)

The cumulative volume along an axis: every interval adds the reused cut times the factor and subtracts the fill. The diagram follows the classical mass diagram method (see Sources):
- the **mass line** rises in cut and falls in fill;
- **balance points** are where it crosses the balance line (zero): between two of them cut and fill balance;
- the area between the mass line and the balance line is the **haul** (m³ × distance), shaded forward (cut moved ahead) and back;
- **average haul** = haul / volume moved;
- with a **free haul distance** the chord of that length is drawn in every loop (the loop is exactly that wide there). Material above the chord moves within the free haul distance; the rest moves further, and its **overhaul** is volume × the distance beyond the free haul distance;
- what remains at the end is a **surplus** (to take away, together with cut that cannot be reused) or a **shortage** (to bring in).

Open Infra computes the haul by sweeping the levels of the mass line: at every level the line splits into closed intervals, and each moves that slice of volume over its width. That is exact for the loops and gives the free haul and overhaul of any shape of loop, also loops with several peaks.

| Argument | Meaning |
|---|---|
| `AS=<handle>` / selection | The axis. |
| `BRON=AUTO` (default) / `CORRIDOR` / `TIN`, `CSV=<path>` | The corridor's section list beside the drawing (`<drawing>_corridor_<axis>.csv`, what the corridor stores); or measured again on the TINs: design (N) against the ground every `FREQUENTIE` m, across the run of design next to the axis (at most `BREEDTE` m, default 100), average end areas. `AUTO` uses the list when it exists. |
| `FACTOR=` | m³ of compacted fill from one m³ of reused cut (shrinkage < 1, swell > 1); default 1. |
| `HERGEBRUIK=<%>` | Share of the cut that is suitable as fill (default 100 %); the rest counts as surplus. |
| `VRIJ=<m>` | Free haul distance (default 0: no free haul analysis). |
| `PLAATS=x,y` | Lower-left corner of the graph; default below the project, after earlier profiles. |

The graph is drawn like a length profile: horizontally at the drawing scale with stations, vertically in m³ with a grid, a band with stations and the mass at them, the mass line on `OI-GRONDBALANS-MASSALIJN`, the balance line and points on `OI-GRONDBALANS-BALANS`, free haul chords on `OI-GRONDBALANS-VRIJ`, and the haul shaded on `OI-GRONDBALANS-VOORUIT` / `-TERUG`. The title and three lines of totals sit above it.

JSON (`ID=`): `stations` (per station: cut, fill, reused, waste, mass), `balanspunten`, `lussen` (per loop: direction, volume, free haul chord), `verplaatst_m3`, `transport_m3km`, `gemiddelde_afstand_m`, `binnen_vrij_m3`, `bovenmatig_m3`, `bovenmatig_m3km`, `overschot_m3`, `tekort_m3`, `eindmassa_m3`, `csv`. The same list goes beside the drawing as `<drawing>_grondbalans_<axis>.csv`.

## Values and their sources

| Value | Where it comes from |
|---|---|
| Cut slope 1:2 and fill slope 1:3 when none is given | The defaults of the corridor's `weg` template in Open Infra. **Estimate**: a starting value, not a standard; take the slopes from the geotechnical advice. |
| Bulking, compaction and reuse factors | None built in (1 and 100 %); project values from the soil investigation. |
| Free haul distance | None built in (0); a contract value. |
| Reach 50 m, slope lines every 1 m, fans every 10°, ground searched every 0.25 m, 5 mm grade-break tolerance, hatching every 2.5 mm on paper | Calculation and drawing settings, not design values. |
| Mass diagram method: cumulative ordinate, balance points, haul as the area under the mass line, average haul = haul / volume | NDDOT Construction Conference 2011, *Earthwork and Mass Diagrams* ([UGPTI DOTSC](https://www.ugpti.org/dotsc/engcenter/downloads/2011-03_EarthworkAndMassDiagrams.pdf)). |
| Free haul chord: a chord of the free haul length parallel to the balance line, touching the mass line; material above it is free haul | The usual "gradient point" construction of mass haul diagrams, as described in road design software documentation. |

## Checks

- Unit tests (`cargo test --lib`): exact TIN volumes of a box, a pyramid (a²h/3 on RD-sized coordinates) and tilted planes, also triangulated differently; feature line heights and edits; draping with grade breaks; a square plateau whose volume matches plateau + prisms + corner cone exactly; an L-shaped plateau against a fine numerical integral of the distance field (within 0.2 %); slopes to a width and a level; two plateaus joined without overlaps or T-junctions; mass haul of a triangular loop with free haul and overhaul in closed form; balance points, reuse and factor; reading the corridor list.
- End to end in Open CAD Studio: `tests/e2e/grading_e2e.py` (a polder in the Dordtse Biesbosch with AHN).

## Limits

- Feature lines are 3D polylines: no arcs in 3D (arcs of a 2D polyline become chords of at most 5°).
- A slope line is in cut or fill from its start; where an edge crosses the ground the slope switches between neighbouring lines, so the surface there has a short crease.
- With a fall, the slopes of two edges meet on the line of equal distance; their heights there differ slightly when the edges have different heights, which gives a vertical step of a few millimetres in the TIN.
- Where a fill slope meets a cut slope of another grading, the newest wins, which can leave a step: no feasible slope connects them.
- Removing a grading from the design surface is not built in: undo, or draw the gradings again.
