# Clothoids, design criteria, rail and sight distance

What the branch `c3d/as-spoor` adds to the Civiel tab. Commands take `KEY=value` arguments like the other `CIV_` commands; `ID=<name>` writes the result to `<config>/OpenCADStudio/open-infra/api/<name>.json`. Coordinates are project coordinates (RD in the Netherlands).

## Clothoids (overgangsbogen)

Alignments know three kinds of pieces: straights, circular curves and **clothoids**. A clothoid starts at a point with a direction, and its curvature changes linearly from k0 to k1 over its length L (left turns positive). A clothoid from a straight to radius R has the parameter A = √(R·L).

**Exact evaluation.** A point on a clothoid is the integral of (cos θ, sin θ) with θ = k0·t + (k1 − k0)·t²/(2L): the generalised Fresnel integrals. They are computed with composite 10-point Gauss–Legendre quadrature, on pieces over which the direction turns at most 0.3 rad. That is exact to rounding (about 10⁻¹² of the length) for any A and L. The unit tests check it against the clothoid tables and the series:

| A | L | x (m) | y (m) |
|---|---|---|---|
| 100 | 50 | 49.92193 | 2.08101 |
| 100 | 100 | 97.52877 | 16.37140 |

and against the series x = l·Σ(−1)ⁿ l⁴ⁿ/((4n+1)(2n)! 2²ⁿ), y = l·Σ(−1)ⁿ l⁴ⁿ⁺²/((4n+3)(2n+1)! 2²ⁿ⁺¹) (l = L/A, scaled by A) for A from 30 to 1200 m and L from 25 to 1000 m, to 10⁻⁶ m.

**Everything that uses an alignment handles them:**
- stationing, and point, direction and curvature at a station;
- station and offset of a point (nearest sample, then Newton on (p − P(s))·T(s) = 0);
- offset lines (`CIV_AS_OFFSET`): the parallel of a clothoid is no clothoid, so it is drawn as arcs through points at the offset;
- length profiles (the alignment band slopes along a clothoid and is labelled with A), cross sections, corridors (the superelevation follows the curvature along the clothoid), LandXML import.

**In the drawing.** Open CAD Studio has no spiral entity. An axis stays one polyline: each clothoid becomes circular arcs through points on it, split until no arc strays more than 1 mm from it (in a metre drawing). The polyline's object data (XDATA `OPENINFRA`) carries one record per clothoid: `spiraal<n>=<first vertex>;<last vertex>;<L>;<k0>;<k1>`. Reading the axis back replaces those arcs by the exact clothoid. The record is relative to the vertices, so moving or rotating the axis keeps it valid. When the polyline was scaled or edited so that the clothoid no longer fits its vertices, the record is ignored and the arcs stay (within 1 mm).

### `CIV_AS` with clothoids

```
CIV_AS PI=x,y;x,y;… R=r;… LS=l;…   (clothoid length per PI)
CIV_AS PI=x,y;x,y;… R=r;… A=a;…    (clothoid parameter per PI)
```

At each inner PI with a radius, a symmetric clothoid–arc–clothoid: τ = L/(2R), shift p = Y − R(1 − cos τ), abscissa k = X − R·sin τ, tangent length T = (R + p)·tan(Δ/2) + k, with (X, Y) the clothoid's end point. When 2τ equals the deflection, the arc drops out (two clothoids meeting at the top). Too long clothoids, or curves that do not fit between the PIs, are refused. The last value of `R=`, `LS=` or `A=` repeats.

**Main points** (on `AM-TANGENTPUNT`, with their station):

| Code | Dutch | English code | Meaning |
|---|---|---|---|
| BA / EA | begin / eind as | BOA / EOA | start and end of the axis |
| BB / EB | begin / eind boog | PC / PT | straight to circular curve and back |
| OB | overgang boog-boog | PCC | between two circular curves |
| HP | hoekpunt | PI | corner between straights |
| BO | begin overgangsboog | TS | straight to clothoid |
| BC | begin cirkelboog (eind overgangsboog) | SC | clothoid to circular curve |
| EC | eind cirkelboog (begin overgangsboog) | CS | circular curve to clothoid |
| EO | eind overgangsboog | ST | clothoid to straight |
| BP | buigpunt (S-bocht) | SS | two clothoids at an inflection |
| OO | overgang overgangsbogen | SS | two clothoids, same direction |

Each clothoid is labelled `A = … · L = …` on the inside of the bend; the JSON result lists `overgangsbogen` with A and L.

### LandXML

`<Spiral>` elements of LandXML 1.x become real clothoids: the start point, the direction towards the spiral's PI (or, without a PI, the direction that makes the chord fit), the length and the curvature from `radiusStart` to `radiusEnd` (`INF` is straight), signed by `rot`. Export of clothoids to LandXML is added when the export branch is merged.

### IFC 4.3

The IFC export (`CIV_IFC`, `GIS_IFC`) writes clothoids into the `IfcAlignment`:
- the layout: an `IfcAlignmentHorizontalSegment` with PredefinedType `CLOTHOID`, the start and end radius (0 for straight, negative for clockwise) and the segment length;
- the geometry: an `IfcCurveSegment` of the composite curve whose parent curve is an `IfcClothoid` with constant A = 1/√|dκ/ds| (negative for a clockwise clothoid). A clothoid whose curvature does not start at zero (out of a curve, or between two radii) uses the part of the parent curve from parameter t₀ = κ₀/(dκ/ds), so the curvature along the segment is exactly the clothoid's.

**Cant** of a rail track (`CIV_SPOOR`) is written as `IfcAlignmentCant` (rail head distance 1.5 m) with one `IfcAlignmentCantSegment` per piece: `CONSTANTCANT` on straights (0) and circular curves (the applied cant), `LINEARTRANSITION` along the clothoids. The outer rail is raised: in a left curve the right rail, in a right curve the left rail. The cant has no geometric representation yet (no `IfcSegmentedReferenceCurve`): it is part of the alignment's layout only.

Checked with IfcOpenShell 0.8.4 (`tests/ifc_validate.py`): no schema or EXPRESS rule errors, and the composite curve evaluated by IfcOpenShell's geometry kernel passes through the start of every layout segment (to 1 mm), so each clothoid ends where the next segment starts. The sample drawing of the unit tests (`target/ifc-test/voorbeeld.ifc`) has a road with clothoids and a rail track with cant.

## Design criteria (`CIV_ONTWERPCRITERIA`, `CIV_AS_TOETS`)

```
CIV_ONTWERPCRITERIA                                   lists the tables
CIV_ONTWERPCRITERIA RICHTLIJN=roa CATEGORIE=niethoofdbaan V=70 [AS=<handle>]
CIV_AS_TOETS AS=<handle> [RICHTLIJN= CATEGORIE= V=] [VERKANTING=<%>]
```

`CIV_ONTWERPCRITERIA` shows the values of a guideline, road category and design speed, each with its source. It keeps the choice for the session and, with `AS=`, on the axis (object data `ontwerpcriteria=roa;niethoofdbaan;70`).

`CIV_AS_TOETS` checks the axis and its design profile against the choice (from the arguments, else the axis, else the session):
- every circular curve against the minimum radius;
- every clothoid against the minimum A and the rule A ≥ R/3;
- curves below the radius that needs transitions, when they meet a straight without a clothoid;
- every grade against the maximum (and the minimum, as a point of attention, where a guideline gives one);
- every vertical curve, R = L/Δg, against the minimum crest or sag radius;
- with `VERKANTING=` a design superelevation against the maximum.

Each violation is marked on `AL-AANKLEDING`: the stretch along the axis, a circle and a text with the stations, the value, the limit and the source. Grade and vertical-curve violations are also marked in the length profile of the axis when there is one. The JSON result lists `overtredingen` with `station`, `eindstation`, `toets`, `waarde`, `grens`, `ernst` (`fout` or `let op`), `tekst` and `bron`.

### The tables

The tables are in `assets/criteria/*.json`. Your own tables go in `<config>/OpenCADStudio/open-infra/criteria/*.json`, in the same form; a table with the same `id` replaces a built-in one. Each value is `{"v": …, "bron": "<key>", "schatting": true|false, "opmerking": "…"}`, and every key is cited in the table's `bronnen`. Values marked `schatting` (estimate) could not be checked against the source itself; the command line and the JSON show them as such, and in the text output they carry a `*`.

| Table | `RICHTLIJN=` | Categories (`CATEGORIE=`) and design speeds (`V=`) |
|---|---|---|
| ROA 2017 | `roa` | `hoofdbaan` 120, `stadsautosnelweg` 90, `niethoofdbaan` 120/90/70/50 |
| NOA 2007 (with errata) | `noa` | `autosnelweg` 120/100/80/50 |
| Roads outside built-up areas, after the CROW Handboek wegontwerp (secondary sources) | `crow` | `stroomweg` 100, `gow` 80, `weg50` 50, `etw` 60 |
| AASHTO Green Book 2018, metric | `aashto` | `emax4`, `emax6`, `emax8`: 50–130 |

**ROA 2017** (Rijkswaterstaat, *Richtlijn Ontwerp Autosnelwegen*, 27-11-2017):

| Category, V | Rmin (m) | Amin (m) | Clothoid below R (m) | Max grade | Crest R (m) | Sag R (m) | Max superelevation | Stopping sight (m) |
|---|---|---|---|---|---|---|---|---|
| Hoofdbaan 120 | 1500 (2.5 %) | 270 | 4000 | 3 % | 12 400 | 24 800 | 2.5 % | 260 |
| Stadsautosnelweg 90 | 700 (2.5 %) | 175 | 2000 | 3 % | 6600 | 13 200 | 2.5 % | 135 |
| Niet-hoofdbaan 120 | 750 (5 %) | 270 | 4000 | 3 % | 12 400 | 24 800 | 7 % | 260 |
| Niet-hoofdbaan 90 | 330 (6 %) | 175 | 2000 | 3 % | 6600 | 13 200 | 7 % | 135 |
| Niet-hoofdbaan 70 | 160 (7 %) | 95 | 800 | 4 % | 3000 | 6000 | 7 % | 80 |
| Niet-hoofdbaan 50 | 80 (6.5–7 %) | 60 | 300 | 4 % | 750 | 1500 | 7 % | 40 |

Sources: tables 5.15 and 5.16 (radii), 5.18–5.20 (clothoids, R/3 ≤ A ≤ R), 5.21 (grades), 5.23 (crest), 5.25–5.26 (sag; the comfort minimum in underpasses is 1200/700/400/200 m), 5.45 (superelevation), 5.4 and §5.1.3 (stopping sight: perception–reaction time 2.5/2.0/1.75/1.5 s, longitudinal friction 0.32/0.38/0.44/0.48), 5.22 and p. 55 (eye height 1.10 m; object for stopping sight a rear light 0.2 × 0.2 m with its underside at 0.50 m). ROA 2017 gives no minimum grade.
- The ROA 2017 text was read through a public copy of the official edition (puc.overheid.nl/rijkswaterstaat/doc/PUC_154078_31).
- **ROA 2019** (standaarden.rws.nl/link/standaard/1474) could not be read. According to Vereniging Infra Ontwerp (news of 29-10-2019 and 3-2-2022), ROA 2019 no longer ties the main carriageway to a design speed but to standard minimum dimensions, and ROA 2019 v1.1 changed nothing in substance. The 120 km/h main-carriageway values above are very likely those minimums; this is not verified.

**NOA 2007** (RWS Adviesdienst Verkeer en Vervoer, *NOA – Nieuwe Ontwerprichtlijn Autosnelwegen*, 1-1-2007, with *Errata nr. 1*, 1-12-2007; open.rijkswaterstaat.nl/publish/pages/127502/noa2007.pdf and …/127503/errata.pdf). NOA adds the 100 and 80 km/h values that ROA 2017 dropped.

| V | Rmin (m) | Amin (m) | Clothoid below R (m) | Max grade | Crest R (m)* | Sag R (m)* | Max superelevation | Stopping sight (m) |
|---|---|---|---|---|---|---|---|---|
| 120 | 750 (5 %) | 270 | 4000 | 3 % | 12 400 | 24 800 | 5 % | 260 |
| 100 | 480 (5 %) | 205 | 2500 | 3 % | 8300 | 16 600 | 5 % | 170 |
| 80 | 270 (5 %) | 115 | 1250 | 4 % | 5000 | 10 000 | 5 % | 103 |
| 50 | 85 (5 %) | 60 | 300 | 4 % | 920 | 1840 | 5 % | 41 |

Sources: table 3-3 (errata) and table 6-1 (radii), table 3-4 and §3.4 (clothoids), tables 3-17 to 3-19 (grades), table 5-2 (superelevation, new construction), table 7-3 (errata, stopping sight), §3.6.2 and §7.3 (eye 1.10 m, object 0.20 m). \*NOA gives the crest only as the formula of table 3-13, Rmin = Lz²/(2(√h₀ + √h_h)²), with the road-course sight of table 7-1, and the sag as 2 × Rtop (table 3-14): these radii are computed and marked as estimates. The comfort minimum for sags is 1200 (tunnels)/850/500/200 m (table 3-16).

**Roads outside built-up areas** (`crow`): the CROW *Handboek wegontwerp* (2013) and the ASVV were **not** consulted. CROW states that its content may not be used for AI purposes, including text and data mining (crow.nl, "Gebruik content voor AI-doeleinden niet toegestaan"), and this table was compiled with AI assistance. The values come from two public secondary sources and are all marked as estimates:
- TU Delft CTB1420-14, *Tabellenblad Wegontwerp* (May 2017), tables 3, 5, 6 and 7 (ocw.tudelft.nl/wp-content/uploads/Tabellenblad-tentamen.pdf): regional stroomweg 100, gebiedsontsluitingsweg 80 and 50 km/h: max grade 4–5 / 5–6 / 7 %, crest (road-course sight) 8300 / 5000 / 900 m (stopping sight 4700 / 1800 / 300), sag comfort 850 / 500 / 200 m (road image 16 600 / 10 000 / 2200), stopping sight 170 / 105 / 40 m, eye 1.10 m, object 0.50 m.
- COB, *Handboek Tunnelbouw*, art. 20 "Verticaal alignement" for non-motorways (version 2015.1), tables 20.1–20.2 (handboektunnelbouw.nl): 60 km/h crest 900 m (sight 64 m), sag comfort 550 m (road image 8000).

Minimum radii, clothoid parameters and superelevation for these roads, and every ASVV value for urban roads (GOW 50, ETW 30), are not in the table. Add them from a licensed copy of the CROW publications as your own table (`<config>/…/criteria/crow.json` or `asvv.json`).

**AASHTO** (*A Policy on Geometric Design of Highways and Streets*, 7th ed., 2018, metric). No public copy of the tables was found, so the values are computed from the Green Book's metric equations and marked as estimates:
- stopping sight d = 0.278·V·t + 0.039·V²/a with t = 2.5 s, a = 3.4 m/s², rounded up to 5 m (table 3-1): 65, 85, 105, 130, 160, 185, 220, 250, 285 m for 50–130 km/h;
- crest K = S²/658 (eye 1.08 m, object 0.60 m; table 3-35): 7, 11, 17, 26, 39, 52, 74, 95, 124; sag K = S²/(120 + 3.5·S) (headlight criterion; table 3-37): 13, 18, 23, 30, 38, 45, 55, 63, 73. A radius is R ≈ 100·K (K in m per % of grade change);
- minimum radius Rmin = V²/(127·(0.01·emax + fmax)) (§3.3) for emax 4, 6 and 8 %, with fmax 0.19 … 0.08 (the fmax values are not checked against the table);
- minimum grade 0.5 % on curbed roads (0.30 % in some cases), eye 1.08 m and object 0.60 m: Green Book 2018 pp. 3-164 and 3-167, as quoted in Michigan DOT, *Sight Distance Guidelines According to the 2018 AASHTO*.

## Sight distance (`CIV_ZICHT`)

```
CIV_ZICHT AS=<handle> [RICHTLIJN= CATEGORIE= V=] [NODIG=<m>] [OOG=<m>] [OBJECT=<m>]
          [OM=10] [RIJLIJN=1.75] [VRIJ=5] [MAX=<m>] [RICHTING=heen|terug|beide]
          [LAGEN=-OG-BEBOUWING;…] [PLAATS=x,y]
```

- **Required:** the stopping sight distance of the design criteria (the arguments, else the axis's choice, else the session's), or `NODIG=`. The eye and object heights come from the same guideline (ROA 2017: 1.10 and 0.50 m; NOA: 1.10 and 0.20 m; AASHTO: 1.08 and 0.60 m) unless `OOG=` and `OBJECT=` are given.
- **Available, per station** (every `OM=` m, looking along and/or against the stationing, from the driver's line `RIJLIJN=` m right of the axis in the driving direction):
  - **over the profile:** the eye at the road height plus the eye height, the object at its road height plus the object height. The road is the design profile of the axis, else the ground along the axis (TIN or AHN). A road point blocks the view when it rises more steeply from the eye than the object;
  - **over the terrain:** the straight sight line in 3D against the existing-ground TIN (status B), only where the line leaves the road platform (`VRIJ=` m from the axis), such as the inside of a bend in a cutting;
  - **in plan:** the straight sight line against the edges of obstacles on layers whose name contains one of `LAGEN=` (default `-OG-BEBOUWING`, the BGT buildings).
- **Output:** a graph per direction below the project (frame `AL-PROFIELBALK`): the required distance, the three kinds (thin, labelled) and the governing available distance (`KW-ZICHTRUIMTE`). Deficits are shaded between available and required on `AL-AANKLEDING`, with their stations and cause. In plan, each deficit gets a line beside the axis, the shortest sight line (`KW-ZICHTRUIMTE`) and a text. The JSON result has `stations` (per station and direction: `profiel_m`, `terrein_m`, `plattegrond_m`, `beschikbaar_m`) and `tekorten` (`van`, `tot`, `minimum_m`, `oorzaak`).
- **Checked** against the textbook crest formula S = √(2R)·(√h₁ + √h₂) (sight line within the curve) to within 2 m in the unit tests.

Obstacles are read from Open CAD Studio's shared view of the drawing. For drawings larger than that view (16 MB unless `OCS_V4_SNAPSHOT_SEGMENT_SIZE` is set), select the obstacles before running the command.

## Rail (`CIV_SPOOR`, `CIV_WISSEL`)

### `CIV_SPOOR`

```
CIV_SPOOR AS=<handle> | PI=x,y;… R=r;… [LS=l;…|A=a;…]  V=<km/h> [VMIN=<km/h>]
          [VERKANTING=mm;…] [SPOREN=2] [SPOORAFSTAND=<m>] [NAAM=] [BEGIN=] [PLAATS=x,y]
```

A track axis on `AM-AS_SPOOR_TREIN`, with stationing, main points, radii and clothoids as for `CIV_AS`, and per circular curve:

- **Equilibrium cant** D_eq = 11.8 · V² / R (mm; V in km/h, R in m). It follows from D = s·v²/(g·R) with s ≈ 1500 mm between the rail heads of standard gauge, g = 9.81 m/s² and v = V/3.6: 1500/(9.81 · 3.6²) = 11.8 (Powell & Gräbe 2017, eq. 1, after Esveld, *Modern Railway Track*, 2001).
- **Applied cant** D: given per curve with `VERKANTING=`, or chosen: D_eq, at most the maximum cant, and with `VMIN=` at most the slow train's equilibrium cant plus the allowed excess; rounded to 5 mm.
- **Cant deficiency** I = D_eq − D at V, and **cant excess** E = D − D_eq(VMIN).
- **Cant ramps**, linear along the clothoids: from 0 at the straight to D at the curve (or between two curves' cants). Checked: the ramp gradient (mm/m), dD/dt = gradient · V/3.6 (mm/s); dI/dt is reported. A curve with cant and no clothoid towards a straight is reported (the cant has nowhere to ramp).
- **Parallel tracks** with `SPOREN=n` at `SPOORAFSTAND=` (m; negative to the left).
- **Cant diagram** below the project (frame `AL-PROFIELBALK`): the applied cant (`VH-VERKANTING`) and the equilibrium cant over the stationing, the main points and each curve's D, D_eq and I.

The JSON result lists `bogen` (`r_m`, `deq_mm`, `d_mm`, `tekort_mm`, `overschot_mm`), `overgangen` (ramp gradient, dD/dt, dI/dt) and `overtredingen`.

**Limits** (`assets/criteria/spoor.json`):

| Value | Used | Source |
|---|---|---|
| Max cant | 160 mm | Regulation (EU) No 1299/2014 (TSI Infrastructure), §4.2.4.2, table 7: ballasted track, mixed traffic (180 mm passenger only). A Dutch maximum of 150 mm could not be verified. |
| Max cant deficiency | 130 mm | TSI INF §4.2.4.3, table 8: freight wagons at V ≤ 160 km/h (153 mm for passenger stock). Higher speeds have lower limits there. |
| Max cant excess | 110 mm | EN 13803, not consulted (secondary mentions only): **estimate** |
| Max ramp gradient | 1.67 mm/m (1:600) | a TU Delft repository document seen only as a search result: **estimate**; EN 13803 is quoted elsewhere as 2.25 mm/m |
| Max dD/dt | 50 mm/s | B. Kufver (Ferroplan), *EN standards 13803-1 and 13803-2 for track alignment*, KTH lecture slides: the EN 13803-1 normal limit for linear ramps (55 mm/s for S-shaped ramps). A Dutch 25 mm/s could not be verified. |
| Track centre distance | 4.00 m | no ProRail value found: **estimate**. TSI INF §4.2.3.2 table 4 gives minimums from 160 km/h: 3.80 m (160–200), 4.00 (200–250), 4.20 (250–300), 4.50 m (> 300 km/h). Give `SPOORAFSTAND=` from the structure gauge. |

ProRail's own design rules (OVS/RLN) are not public and EN 13803 is not freely available; check the values above against them.

### `CIV_WISSEL`

```
CIV_WISSEL AS=<track> STATION=<m> [TYPE=1:9] [ZIJDE=links|rechts] [RICHTING=op|af] [LENGTE=50] [NAAM=W1]
```

From the ribbon: select the track, click **Wissel** and click the turnout start on the side the diverging track goes (type 1:9).

The turnout starts (WB, *wisselbegin*) at the station, along (`op`) or against (`af`) the stationing. The diverging track is a circular curve of the turnout's radius from WB, tangent to the main track, turning α = atan(1/n); then straight through the crossing to the turnout end (WE) and `LENGTE=` m beyond. MS (*mathematisch snijpunt*) is where the straight diverging axis crosses the main axis, R·tan(α/2) from WB. The diverging track is drawn and stored as an alignment (`W1 afbuigend`, stationing from the main track's station), with WB, MS and WE marked and the label `W1 1:9 R260 rechts · V 40 km/h`.

**Turnouts** (nl.wikipedia, *Wissel (spoorweg)*, table "Wissels in het Nederlandse spoorwegnet", read 29-10-2026, after ProRail, *Regels voor het functioneel ontwerp van railinfrastructuur*, 2016):

| `TYPE=` | 1:n | Diverging radius | Angle α | Diverging speed |
|---|---|---|---|---|
| `1:9` | 1:9 | 260 m (ProRail standard) | 6.340° | 40 km/h (merged table cell: **estimate**) |
| `1:9-195` | 1:9 | 195 m (only where 260 m does not fit) | 6.340° | 40 km/h (**estimate**) |
| `1:12` | 1:12 | 465 m | 4.764° | 60 km/h |
| `1:15` | 1:15 | 725 m | 3.814° | 80 km/h |
| `1:18.5` | 1:18.5 | 760 m (Betuweroute) | 3.094° | not given |
| `1:34.7` | 1:34.7 | 2300 m (movable point, being phased out) | 1.651° | not given |

Dutch turnout lengths were not found. The model takes the curve from WB to the tangent point, so WB–MS = MS–WE = R·tan(α/2) (1:9 R260: 14.40 m; 1:12: 19.34 m; 1:15: 24.14 m): an **estimate**. Real turnouts have a straight switch part and a straight crossing, so their WB–MS and MS–WE differ. Turnouts are placed on the tangent at WB; turnouts in curved track (with adjusted radii) are not modelled.

## Sources

- Rijkswaterstaat, *Richtlijn Ontwerp Autosnelwegen (ROA) 2017*, 27-11-2017 (PUC_154078_31).
- Rijkswaterstaat, *ROA 2019* and *ROA 2019 v1.1*: standaarden.rws.nl/link/standaard/1474 (not read); Vereniging Infra Ontwerp, news of 29-10-2019 and 3-2-2022.
- RWS Adviesdienst Verkeer en Vervoer, *NOA – Nieuwe Ontwerprichtlijn Autosnelwegen*, 1-1-2007, and *Errata nr. 1*, 1-12-2007 (open.rijkswaterstaat.nl).
- TU Delft CTB1420-14, *Tabellenblad Wegontwerp*, May 2017 (ocw.tudelft.nl).
- COB, *Handboek Tunnelbouw*, art. 20 "Verticaal alignement", non-motorways, version 2015.1 (handboektunnelbouw.nl).
- CROW, "Gebruik content voor AI-doeleinden niet toegestaan" (crow.nl): why CROW publications were not consulted.
- AASHTO, *A Policy on Geometric Design of Highways and Streets*, 7th ed., 2018; *Errata*, October 2019 (downloads.transportation.org/GDHS-7-Errata.pdf).
- Michigan DOT, *Sight Distance Guidelines According to the 2018 AASHTO* (quotes Green Book 2018 pp. 3-164, 3-167, tables 3-1, 3-35, 3-37).
- A.F. Powell & P.J. Gräbe (2017), "Exploring the relationship between vertical and lateral forces, speed and superelevation in railway curves", *J. S. Afr. Inst. Civ. Eng.* 59(3):25–35, eq. 1–4; after C. Esveld (2001), *Modern Railway Track*, MRT-Productions, Delft.
- Commission Regulation (EU) No 1299/2014, TSI "Infrastructure", §4.2.3.2 table 4, §4.2.4.2 table 7, §4.2.4.3 table 8 (as adopted; later amended, e.g. by Regulation (EU) 2019/776: check the consolidated text).
- B. Kufver (Ferroplan), *EN standards 13803-1 and 13803-2 for track alignment*, KTH lecture slides (about 2019).
- nl.wikipedia, *Wissel (spoorweg)*, read 29-10-2026; after ProRail, *Regels voor het functioneel ontwerp van railinfrastructuur* (2016).
- Clothoid tables: unit clothoid values at l = 0.5 and l = 1 (x = 0.499219, y = 0.020810; x = 0.975288, y = 0.163714), checked in the unit tests against the Fresnel series.
