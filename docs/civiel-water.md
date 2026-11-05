# Water: watercourses, storage, pressure networks and storms on the ground

Three Civiel commands calculate water in the design: the water surface along a watercourse (`CIV_WATERLOOP`, issue #74), a storage facility with a design storm routed through it (`CIV_BERGING`, #64), and a drinking-water or pressure network with a fire-flow check (`CIV_DRUKNET`, #67). They sit in the ribbon under **Terrein → Water** (*Waterlijn watergang*, *Berging*) and **Riolering → Riool** (*Drukleidingnet*).

The calculations are the host-independent kernels in `rekenkern` (`waterloop.rs`, `berging.rs` with `buien.rs`, `druknet.rs`), tested there on analytical cases. The commands in `src/civiel/waterloop.rs`, `berging.rs` and `druknet.rs` read the drawing, call the kernel, draw the results and write a report.

All three take `KEY=value` arguments and `ID=<name>` for a JSON result in `<config>/OpenCADStudio/open-infra/api/<name>.json`. Coordinates are project coordinates (RD in the Netherlands). Lists are separated by `;`; inside one list item the values are separated by `,`, so decimals take a point there (`DUIKER=120,0.8,12,0.10,0.05`). Single numbers may use a decimal comma (`Q=1,5`). Each command also writes a Markdown report and a JSON file beside the drawing. The MCP tools are `civ_water_profile`, `civ_storage` and `civ_pressure_network`.

Ground levels come from the TIN in the drawing, else AHN (in the Netherlands) or the worldwide terrain, the same way `CIV_DWARSPROFIELEN` and `CIV_LENGTEPROFIEL` sample them.

## `CIV_WATERLOOP`: water surface profile of a watercourse

`CIV_WATERLOOP AS=<handle> Q=<m³/s> [RUWHEID=] [BENEDENSTROOMS=] [INTERVAL=] [BREEDTE=] [DUIKER=…] [STUW=…] [RICHTING=]`

Cross sections are cut across the axis from the ground, and the steady water surface at discharge `Q` is computed from the downstream end upstream.

| Argument | Meaning | Default |
|---|---|---|
| `AS=` | the axis of the watercourse (hex handle); without it, the one selected axis | selection |
| `Q=` | discharge in m³/s, constant along the reach | required |
| `RUWHEID=` | Manning n (a value below 1, e.g. `0.035`), or the Manning coefficient k_M = 1/n in m^⅓/s (1 and up, e.g. `30`) | n = 0.035 |
| `BENEDENSTROOMS=` | downstream boundary: a water level in m (e.g. the target level of the receiving water), `NORMAAL` (normal depth on the mean bed slope) or `KRITIEK` (critical depth, a free fall) | `NORMAAL` |
| `INTERVAL=` | cross section spacing in m | the section distance of the settings (20 m) |
| `BREEDTE=` | section width on each side of the axis, m | the larger of the settings' left and right widths (20 m) |
| `DUIKER=` | culverts: `station,diameter,length,invert_in,invert_out;…`. Station in the axis stationing; diameter in m, or in mm from 20 up (`800`); a box culvert as `width x height` (`2x1.5`, `2000x1500`); `invert_in` is the upstream invert, `invert_out` the downstream one | none |
| `STUW=` | weirs: `station,crest,width;…` (m) | none |
| `RICHTING=` | flow direction: `MEE` (along the axis), `TEGEN` (against it), `AUTO` (from the bed: the water runs to the lower end; a flat bed counts as along) | `AUTO` |

**Cross sections.** Every `INTERVAL` m along the axis, at both ends, and on both sides of each structure. A culvert takes its length plus 1 m on each side, a weir 1 m plus 1 m on each side; the sections there stand in the open channel, not on the embankment over the culvert, and regular sections inside that stretch are left out. The ground is sampled every 0.5 m across (finer for narrow sections: `BREEDTE`/40, at least 0.1 m). A section needs at least three ground heights. At most 400 sections per run.

**Method** (`rekenkern::waterloop`, after the HEC-RAS *Hydraulic Reference Manual*, USACE, version 4.1, 2010, "HRM"):
- the energy equation between sections by the standard step method (HRM eq. 2-1, 2-2), the friction slope from the average conveyance (HRM eq. 2-13, the HEC-RAS default);
- Manning conveyance K = A·R^⅔/n per wet sub-area (HRM eq. 2-5), the velocity coefficient α from those conveyances (eq. 2-7);
- contraction and expansion losses on the difference in velocity heads: 0.1 and 0.3 for gradual transitions (HRM table 3-3);
- critical depth as the minimum of the specific energy (HRM ch. 2). Where no subcritical solution exists the level is set to critical depth and flagged, as HEC-RAS does;
- normal depth (for `BENEDENSTROOMS=NORMAAL`) on the least-squares slope of the lowest bed point of the sections;
- culverts (HRM ch. 6, after FHWA 1985): outlet control with entrance loss 0.5 (headwall, square edge, HRM table 6-3), exit loss 1.0 (sudden expansion, HRM eq. 6-12) and barrel friction by Manning, n = 0.013 (concrete pipe, 0.012–0.013 in FHWA HDS-5, 2012); inlet control simply as critical depth at the inlet plus the entrance loss. The larger governs;
- weirs: broad crested, Q = C·b·H^1.5 with C = 1.70 m^½/s (critical flow on the crest, (2/3)^1.5·√g; HRM table 8-1), reduced for submergence after Villemonte (1947), exponents 1.5 and 0.385.

**Manning n default.** 0.035: Chow, *Open-Channel Hydraulics* (1959), table 5-6, excavated earth channel, winding and sluggish, dense weeds, normal value. A maintained ditch with grass is nearer 0.030, a clean one 0.025. Water boards often give k_M = 1/n (20–35); `RUWHEID=30` is taken that way.

**Drawn** (NLCS layers with the drawing status of the settings):
- `GW WATERGANG_WATERLIJN` (G): the inundation boundary in plan, two polylines through the outer waterlines left and right of each section (object data `waterloop`, `as`, `q`, `zijde=links|rechts`);
- `GW WATERGANG_WATERLIJN` (G) in the newest length profile of the axis (`CIV_LENGTEPROFIEL`): the water line, with a tick and the levels upstream / downstream at each structure. Without a length profile the message says so; draw one and run again;
- `GW WATERGANG_WATERLIJN` (T): the water level at both ends of the axis and the levels at each culvert and weir, in plan.

A new run on the same axis replaces the previous drawing (the index kind `waterloop`).

**Report.** `<drawing>-waterloop-<axis>.md` (per section: station, water level, depth, velocity, Froude number, top width, flags; per structure: levels and regime) and `<drawing>-waterloop-<axis>.json` (the kernel input and result). `ID=` returns `secties` (station, `peil`, `diepte`, `snelheid`, `froude`, `breedte`, `links`, `rechts` as axis offsets, `energie`, `kritiek`, `boven_rand`), `kunstwerken`, `stroomt_met_as`, `lengteprofiel`, `rapport`, `json`.

**Limitations.**
- Steady, one-dimensional, subcritical flow only: no hydraulic jumps, no mixed regime, no unsteady flow.
- One Manning n for the whole section and reach; every wet part of a section conveys (no levees, no ineffective areas, no separate floodplain n).
- When the water rises above an end of a section, the end is taken as a vertical wall and the section is flagged (`boven de rand`): widen `BREEDTE=`.
- The reach length between sections is the station difference along the axis (no separate left/right bank lengths).
- Structures lie between their two bounding sections without friction over that stretch; inlet control of culverts is the simple unsubmerged check (no HDS-5 nomographs); Villemonte is applied to broad-crested weirs too.
- Bridges, piers and lateral inflow are not modelled. The cross sections are the existing ground; a ditch designed in a corridor is not read yet (draw the design TIN first).

## `CIV_BERGING`: wadi, pond or crate storage with a design storm

`CIV_BERGING [LIJN=<handle>] SOORT=wadi|vijver|krat BODEM=<m> DIEPTE=<m> [TALUD=1:n] [KNIJP=] [DREMPEL=] [INFILTRATIE=] [BUI=] [OPPERVLAK=] [NAAM=] [TIN=ja] [PLAATS=x,y]`

The closed polyline is the **bottom outline** of a wadi or pond, or the **footprint** of a crate system.

| Argument | Meaning | Default |
|---|---|---|
| `LIJN=` | the closed polyline (hex handle) | the one selected entity |
| `SOORT=` | `wadi`, `vijver` (pond) or `krat` (crates) | required |
| `BODEM=` | bottom level, m | required |
| `DIEPTE=` | depth from the bottom to the top of the facility, m | required |
| `TALUD=` | side slope `1:n` (horizontal n per vertical 1); crates have vertical sides | 1:3 |
| `KNIJP=` | orifice (throttle): `diameter[,invert]`, diameter in m or in mm from 20 up (`110`); the invert defaults to the bottom | none |
| `DREMPEL=` | overflow weir: `crest level,width` (m) | none |
| `INFILTRATIE=` | infiltration rate in mm/h over the wetted area (a design value from a soil test, safety factor applied) | none |
| `BUI=` | the design storm, see below | `08` |
| `OPPERVLAK=` | connected paved area draining into it, m² or ha (`0.5ha`) | none: only the rain on the facility |
| `NAAM=` | name; the same name again replaces the earlier drawing | `Wadi n`, `Vijver n`, `Kratten n` |
| `TIN=ja` | also cut the design TIN to the basin, as `CIV_PLATEAU` does: the bottom at `BODEM` with slopes 1:n to the existing ground, joined into the design surface (N) | no |
| `PLAATS=x,y` | lower-left corner of the graph | right of the facility |

`STATUS=` sets the NLCS drawing status; as a design it defaults to **N** (new work).

**Storms** (`rekenkern::buien`):
- `01`–`10`: the standard storms of Stichting RIONED, *Leidraad Riolering* module C2100 (2004), appendix 3, table B3.2, in 5-minute blocks. Bui 08 (T = 2 years, peak late, 19.8 mm in 60 min) is the usual design check;
- `T100`, `T250`, `T1000`, `48H`, `R100`, `R250`, `R1000`: the standard events of the DPRA stress test *wateroverlast* (notitie standaarden stresstest wateroverlast, November 2024, table 4, climate 2050): local 70 mm / 1 h (T = 100), 90 mm / 1 h (T = 250), 155 mm / 2 h (T = 1000), 200 mm / 48 h; regional 130, 150 and 185 mm / 48 h. The note prescribes constant intensity, so these are single blocks;
- `<mm>:<min>`: a uniform block, e.g. `BUI=40:60`;
- a file path: one block per line, `minutes,value`, the minutes at the end of each block. The value is mm/h, unless a header line names mm per block (`minuten;mm`). Separators: comma, semicolon, tab or spaces; with semicolon or tab a decimal comma is fine.

**Stage–storage curve** (`rekenkern::berging::StageStorage`):
- wadi and pond: the plan area at height z above the bottom is the mitred outward offset of the bottom polygon by r = n·z, A(z) = A₀ + P₀·r + K·r² (K = 4 for a rectangle), sampled in 100 steps with average end areas. Exact for convex bottoms; concave ones as long as the offset does not cross itself;
- crates: footprint × depth × void ratio 0.95 (manufacturer data: 95–96 %, Wavin Q-Bic Plus; 95.5 %, Pipelife Stormbox; ≥ 95 %, DYKA Rainbox). Adjust to the product;
- above the top the curve continues with vertical walls; the run then reports that the facility overtops.

**Inflow.** Q = A·i with A the connected area plus, for a wadi or pond, the plan area at the top (rain on the open water), and runoff coefficient 1 (all rain on the connected paved area arrives). No delay over the surface, which is conservative for storage.

**Outlets** (Leidraad Riolering C2100, appendix 2, g = 9.81 m/s²):
- orifice (*knijp*): Q = μ·a·√(2g(h − h_c)) with μ = 0.6 for a round opening (§2.5), h_c the centre of the opening; partly submerged it behaves as a weir;
- weir (*drempel*): Q = m·b·⅔·√(⅔g)·H^1.5 with m = 0.8 (§2.4, a free short weir); its volume counts as overflow;
- infiltration: the rate over the plan area at the water level.

**Routing.** Level-pool routing by the storage-indication (modified Puls) equation, Chow, Maidment & Mays, *Applied Hydrology* (1988), §8.2, with a time step of 60 s, until the facility is empty (storage below 1 % of the peak) or 7 days have passed. The new level is found by bisection, so any mix of outlets works; the volume balance closes to rounding (reported).

**Drawn:**
- on the NLCS object of the facility — `RI ITR_RESERVOIR_INFILTRATIERESERVOIR_WADI`, `RI ITR_RESERVOIR_BERGINGSVIJVER` or `RI ITR_RESERVOIR_INFILTRATIERESERVOIR` (crates) — (G): the bottom outline as a 3D polyline at `BODEM` (with the object data `berging`, `soort`, `bodem`, `diepte`, `talud`, `berging_m3`, `max_peil`), the top outline at `BODEM + DIEPTE`, and the corner lines of the side slopes; (T): the name, bottom level, storage and the highest level;
- `GW TALUDARCERING_LANG` and `GW TALUDARCERING_KORT`: slope hatching, alternately long and short, every 4 paper mm;
- `GW WATERGANG_WATERLIJN`: the waterline at the highest level in plan, and the level line in the graph;
- the graph (100 × 50 paper mm, right of the facility): frame, grid and axis texts on `AL PROFIELBALK` (G, T); level against time in hours; the top of the facility and the weir crest as horizontal lines on the facility's layer.

**Report.** `<drawing>-berging-<name>.md` (the facility, the storm, highest level, storage, inflow, volume per outlet, overflow, drain time, mass balance, the curve) and `.json` (summary, curve and the series every 5 minutes). `ID=` returns `berging` (the summary: `max_peil`, `max_berging_m3`, `overloop`, `overloop_m3`, `boven_rand`, `ledigingstijd_s`, …), `kromme`, `reeks`, `rapport`, `json`.

**Defaults that are choices, not norms.** The side slope 1:3 is a common choice for slopes that are mown and safe; set `TALUD=` to the design. The drain time is measured until 1 % of the peak storage remains (an adjustable choice in the kernel). Municipal requirements for drain time (often 24–72 h for a wadi) are not checked: compare the reported value with the local rule.

**Limitations.**
- The stage–storage curve comes from the designed shape (bottom polygon, depth, slope), not from the TIN; where the ground around a wadi is lower than its top, the real storage is smaller. `TIN=ja` cuts the design TIN, but the curve does not read it back.
- `TIN=ja` draws the plateau's own edge and daylight lines on the grading layers too (`CIV_PLATEAU`), and slopes to the existing ground, not to `BODEM + DIEPTE`.
- One orifice, one weir and one infiltration rate; no backwater from the receiving water on the orifice; the infiltration rate is constant (no groundwater rise).
- No surface runoff delay; the connected area drains with coefficient 1.
- No IFC export of the storage object yet.

## `CIV_DRUKNET`: pressure network with fire flow

`CIV_DRUKNET [LAAG=<layer>] BRON=x,y,head[;…] [VRAAG=] [DIAMETER=mm] [RUWHEID=] [BRANDKRAAN=x,y;…] [MINDRUK=kPa] [BLUSDEBIET=m³/h]`

| Argument | Meaning | Default |
|---|---|---|
| `LAAG=` | layer pattern(s) with `*` wildcards, `;`-separated: the lines (and hydrant points) on them | — |
| (none) | without `LAAG=`: the selected lines; without a selection: the infra library's water mains (`waterleiding`, `persleiding-*`) and hydrants, and the lines on `*-KL-WATER_TRANSPORTLEIDING*` | |
| `BRON=` | sources: `x,y,head` with the head (hydraulic grade) in m, e.g. the level of a water tower or the pressure of a supply point as head; connected to the nearest node or pipe within 10 m | required |
| `VRAAG=` | demand: l/s per node (`0.05`), or a total `TOTAAL:2.5` spread over the nodes by half the length of the pipes at each node | 0 |
| `DIAMETER=` | internal diameter in mm for pipes without one in their object data | required when a pipe has none |
| `RUWHEID=` | Hazen-Williams C (20 and up, e.g. `130`), or Darcy-Weisbach wall roughness k in mm (below 20, e.g. `0.1`) | C = 130 |
| `BRANDKRAAN=` | extra hydrants as points (next to the library's hydrants and points, circles or blocks on `*BRANDHYDRANT*` layers) | none |
| `MINDRUK=` | minimum hydrant pressure while drawing the fire flow, kPa | 150 |
| `BLUSDEBIET=` | required fire flow per hydrant, m³/h | 60 |

**The network from the drawing.** Every line, polyline (arcs as chords) or 3D polyline is a pipe; ends within 0.5 m become one node; a node lying on another line splits that line (a tee); a closed ring gets a node halfway. Vertices in between are bends, not nodes. A hydrant or source within 10 m of the network is connected at the nearest node, or the nearest pipe is split there. Node elevations come from the ground (the pressure is the pressure at ground level); a node without ground takes the mean of the others (noted).

**Diameters.** From the object data: the infra library's `diameter` parameter (mm; the catalogue default when the object does not set it), or a `binnendiameter_mm`, `diameter` or `dn` value on the entity (mm). It is used as the **internal** diameter. The library's value is the connection (nominal, outside) size, so for PE SDR 11 the real internal diameter is about 0.82 × the outside: give the internal diameter in the object data or `DIAMETER=` when that matters.

**Method** (`rekenkern::druknet`): the global gradient algorithm of Todini & Pilati (1988), as in EPANET (Rossman et al., EPANET 2.2 user manual, "Analysis algorithms"), solving heads and flows together with Newton steps; loops need no loop definitions. Friction:
- Hazen-Williams h = 10.667·C^-1.852·D^-4.871·L·Q^1.852;
- Darcy-Weisbach h = 8fL/(gπ²D⁵)·Q² with f from Swamee & Jain (1976), laminar 64/Re below Re 2000, interpolated between 2000 and 4000;
- water at 20 °C (ν = 1.004·10⁻⁶ m²/s); no minor losses;
- converged when Σ|ΔQ|/Σ|Q| < 10⁻⁶.

**Defaults and sources.**
- Hazen-Williams C = 130: EPANET 2 users manual (Rossman 2000), table 3.2 gives 130–140 for new cast iron and 140–150 for plastic; 130 allows for some ageing.
- Fire flow 60 m³/h (1000 l/min) at one hydrant: Brandweer Nederland, *Handreiking Bluswatervoorziening en Bereikbaarheid* (2003); the 2019 edition allows 30 m³/h in many cases, and regional policy decides (e.g. Veiligheidsregio Groningen 2025 prefers 60 m³/h, 30 m³/h minimum).
- Minimum hydrant pressure 150 kPa: regional policy (Veiligheidsregio Groningen, *Beleid bluswatervoorziening en bereikbaarheid*, 2025) asks 1 bar at the pump of the fire engine, which by rule of thumb needs 1.5–2 bar at a pressurised source; the lower end is used.

**Fire-flow check** per hydrant: the largest extra draw at the hydrant's node that keeps its pressure at `MINDRUK`, by bisection (up to 0.5 m³/s), plus the hydrant pressure and the lowest pressure anywhere while drawing `BLUSDEBIET`. A hydrant meets the requirement when the available flow reaches `BLUSDEBIET`.

**Drawn** (NLCS layers with the drawing status of the settings):
- `KL WATER_TRANSPORTLEIDING` (T): at each node its id and pressure (`K3 312 kPa`; a source shows its head), along each pipe its id, flow, velocity and diameter (`L4 · 1,25 l/s · 0,32 m/s · Ø110`);
- `KL WATER_BRANDLEIDING_BRANDHYDRANT` (G): a circle at each checked hydrant; (T): `BK1 72 m³/h (150 kPa) voldoet|onvoldoende`.

A new run replaces the labels of the previous one (the index kind `druknet`).

**Report.** `<drawing>-druknet.md` (nodes, pipes, fire flow) and `.json` (the kernel network, nodes, pipes, hydrants). `ID=` returns `knopen` (id, x, y, `maaiveld`, `stijghoogte`, `druk_kpa`, `vraag_ls`, `bron`), `leidingen` (id, from, to, length, `diameter_mm`, `debiet_ls`, `snelheid`, `verlies_m`, `verlies_m_per_km`, the source entity), `brandkranen` (`beschikbaar_m3h`, `druk_bij_nodig_kpa`, `laagste_bij_nodig`, `voldoet`), `iteraties`, `rapport`, `json`.

**Limitations.**
- Steady state only: no 24-hour demand pattern (extended period), no tanks with a varying level, no pumps, no pressure-reducing or check valves. Valves can only be modelled by leaving the pipe out.
- No minor losses at bends and fittings; the snap tolerance (0.5 m) and the connection distance (10 m) are fixed.
- Pressure is at ground level, not at the pipe; the demand is the same per node or spread by pipe length, not from connections or dwellings.
- Pipes that cross without a shared end or vertex within 0.5 m are not connected (as on site: a crossing is not a junction).
- Minimum service pressures for customers are reported (lowest and highest) but not checked against a norm.

## `CIV_BUI`: a storm on the ground (stress test)

A design storm falls on the ground grid and is simulated in time (issue #63): water flows to the low spots, fills them and spills over to the next. The scheme is local-inertial overland flow (Bates et al. 2010, as in LISFLOOD-FP, with the θ-weighting of de Almeida et al. 2012) on a regular grid from the TIN in the drawing, or else AHN or the worldwide terrain, the same grid as `CIV_STROOMGEBIED`. The kernel is `rekenkern::bui`. Mass is conserved to round-off, and the report states the water balance.

| Argument | Meaning | Default |
|---|---|---|
| `BUI=` | `T100` (70 mm in 1 h), `T250` (90 mm in 1 h), `T1000` (155 mm in 2 h), `48h` (200 mm in 48 h), `R100`/`R250`/`R1000` (regional 48 h events): the stress test events for climate 2050 of the Deltaplan Ruimtelijke Adaptatie (*Stresstest wateroverlast*, November 2024). Also `01`–`10` (the standard storms of Leidraad Riolering C2100), `<mm>/<minutes>` (a uniform block) or a CSV file (minutes, mm or mm/h). | `T100` |
| `BRON=TIN\|AHN`, `CEL=` | The ground and the cell size (m). | TIN if present, 2 m |
| `INFILTRATIE=` | Infiltration capacity (mm/h) everywhere. | 0 |
| `RUWHEID=` | Manning's n. | 0.030 (Chow 1959, short grass) |
| `RAND=open\|dicht` | Water may leave over the edge of the model, or not. | open |
| `KOLKEN=x,y;…` or `LAAG=*KOLK*`, `KOLK=` | Gullies or sewer inlets and the l/s each takes away. | none, 5 l/s |
| `NA=` | Minutes simulated after the rain stops. | 30 |

What it draws:
- **Maximum water depth** in classes from 5, 10, 15, 20 and 30 cm, as closed outlines on `OI-BUI-DIEPTE-05CM` … `-30CM`. These are the depth classes of the Klimaateffectatlas.
- **Ponds** at the highest water, labelled with their volume and depth on `OI-BUI-PLAS`.
- **Overflow points:** for each pond, where it overflows, marked with the overflow level on `OI-BUI-OVERLOOP`.

The report `bui.md` beside the drawing gives the area per depth class and the water balance: rain, storage on the ground, infiltration, over the edge, into gullies, and the error. The JSON (`ID=`) holds the ponds and the balance.

Limits:
- **Grid size:** at most 400,000 cells. A 500 × 500 grid takes about 1.5 minutes of computing for a 1.5 h storm in a release build.
- **Infiltration:** a constant capacity, with no decrease in time.
- **Gullies:** they never surcharge, so no water comes back from the sewer.

It is a stress test indication. It does not replace a calibrated 2D model coupled to the sewer.
