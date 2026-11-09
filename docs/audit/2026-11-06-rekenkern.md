# Calculation core against standards and fixed sources (2026-11-06)

This note checks the calculations of Open Infra Studio against a standard or another fixed source, where one exists. It covers the code on `main` at `841f5fa` (6 November 2026). Per calculation it gives the source, the test values, what the code gives and the difference.

The work was reading and probing, not a rewrite. Findings that the reviews of #61 and #97 already reported (MCP over HTTP, paths outside the work folder, and the seven findings in `src/civiel`, `nlcspp`, `hydx.rs` and `geoexport.rs`) are not repeated here.

## Summary

| Area | Source | Result |
|---|---|---|
| RD ↔ ETRS89, NAP | NSGI RDNAPTRANS2018, self-validation set Z001 | Within tolerance on all 7,959 points in the grid |
| Sewer hydraulics | Prandtl-Colebrook (Colebrook-White) | Matches an independent Darcy–Weisbach solution to 0.05 % |
| GWSW pipe size | GWSW 1.7 data dictionary | Matches: `BreedteLeiding` is the nominal (trade) size |
| Horizontal and vertical alignment | Clothoid and parabola geometry | Matches |
| Design criteria | ROA 2017, AASHTO Green Book 2018 | All checked ROA 2017 values match; AASHTO values match their formulas |
| Rail cant | D = s·v²/(g·R) | Matches |
| Volumes and mass haul | Prismoid, average end area | Matches |
| **Parcel areas** | Circle segment area | **Error: arcs in a plot are ignored.** Fixed in #105 |
| Settlement and stability | Fadum, Terzaghi, Koppejan, Bishop, Robertson 2010 | Formulas match; three table values to check against NEN 9997-1 |
| Parking | NEN 2443:2013 tables 4A–4C | Internally consistent; not checked against the standard itself |
| AHN heights | GeoTIFF raster type, PDOK WCS tile | Matches (PixelIsArea, corner tie point) |

## 1. RD ↔ ETRS89 and NAP height (`src/gis/rdnap.rs`)

**Source.** NSGI, *RDNAPTRANS™2018* specification `RDNAPTRANS2018_v220627.pdf`, implementation variant 1, with the grids `rdcorr2018.txt` and `nlgeo2018.txt` and the self-validation file `Z001_ETRS89andRDNAP.txt` (10,000 points). Package: <https://www.nsgi.nl/documents/d/nsgi/rdnaptrans2018_v220627-zip>. The readme of the test set gives the tolerances: 0.0010 m for RD x, y and NAP, and 1·10⁻⁸ degree for latitude and longitude.

**Method.** The module carries two ignored tests for this (`official_points_with_grids`, `whole_self_validation_set`). The machine was too busy for a full build of the crate during this audit. So the procedure was ported line by line to Python (same constants, same iteration limits, same rounding of the sphere coordinates and of the RD input) and run over the whole Z001 file with the official grids.

**Result.**

| Points | Direction | Outside tolerance | Largest difference |
|---|---|---|---|
| 7,959 inside the grid | ETRS89 → RD | 0 | 0.28 mm |
| | RD → ETRS89 | 0 | 2.3·10⁻⁹ ° |
| | NAP (h − N) | 0 | 0.05 mm |
| 2,041 outside the grid | ETRS89 → RD | 0 | 0.78 mm |
| | RD → ETRS89 | 239 | 4.2·10⁻⁸ ° (about 5 mm) |

Outside the grid the procedure applies no correction (c₀ = 0, specification §2.3). The 239 inverse differences are the same without any grid, and all of these points lie at least 574 km outside the grid bounds (up to 70° N and 10° W). The specification (§2.2.2 and §3.3) says that the fixed heights of the datum step keep back-and-forth differences below 0.0010 m only up to 500 km outside the grid bounds. So these differences belong to the procedure, not to the code. Not a finding.

**Observations (no change made).**

- `EPSG:4326` (WGS 84) is treated as ETRS89. In 2026 the two differ by about 0.9 m in the Netherlands. This is the usual choice for map data, but it is not written in `docs/`.
- The quasi-geoid (`nap_with`) is only used by the tests. NAP heights in exports are handled in a separate fix (GeoJSON NAP height).

## 2. Sewer design (`src/civiel/riool.rs`)

**Full-pipe velocity.** The code uses Prandtl-Colebrook: v = −2·log₁₀(k/(3.71·D) + 2.51·ν/(D·√(2gDI)))·√(2gDI), ν = 1.31·10⁻⁶ m²/s (water at 10 °C). An independent check solves Darcy–Weisbach with the Colebrook-White friction factor by iteration:

| D (m) | I | k (mm) | Code v (m/s) | Code Q (l/s) | Darcy–Weisbach + Colebrook v (m/s) |
|---|---|---|---|---|---|
| 0.300 | 1:300 | 0.4 | 0.9410 | 66.5 | 0.9407 |
| 0.2964 (PVC 315 inner) | 1:315 | 0.4 | 0.9107 | 62.8 | 0.9104 |
| 0.500 | 1:500 | 3.0 | 0.7785 | 152.9 | 0.7781 |

The difference (0.05 %) comes from the constant 3.71 against 3.7. No finding.

**Invert levels and cover.** Cover is measured from the outside top of the pipe (BOB + inner diameter + wall), crowns are matched when a larger pipe follows, and the slope follows from the lowest allowed invert along the pipe. No finding.

**GWSW export.** The GWSW data dictionary defines `BreedteLeiding` as "De bij het materiaal gebruikelijke aanduiding van de breedte van een leiding (zie diameter leiding)", and `DiameterLeiding` as the trade size: the outside for plastics, the inside for other materials (<https://data.gwsw.nl/1.7.0/totaal/BreedteLeiding>, <https://data.gwsw.nl/1.7.0/totaal/DiameterLeiding>). The export writes the nominal size, and the import reads it back the same way. No finding.

**Follow-ups (VERVOLG).**

1. The API result field `vullingsgraad` holds Q/Q_full. In Dutch sewer practice *vullingsgraad* usually means the water depth over the pipe height (h/D). A primary RIONED definition was not found. A clearer name would be `q_over_qvol`, or the field could give h/D.
2. `verhang_text` rounds 1:n to a multiple of 5. A pipe at 1:252 against a minimum of 1:250 is reported as "verhang 1:250 flauwer dan 1:250".

## 3. Alignment geometry (`src/civiel/as_profiel.rs`, `src/ifc/alignment.rs`)

- Clothoid: tangent length T = (R + ΔR)·tan(Δ/2) + X_M with ΔR = Y − R·(1 − cos τ), X_M = X − R·sin τ, τ = L/(2R); the clothoid itself by Gauss–Legendre integration of the Fresnel integrals. Matches the textbook construction.
- Parameter A = √(L/|Δκ|), so L = A²/R from a straight. Matches.
- Vertical curve: z = z_BVC + g₁·x + (g₂ − g₁)/(2L)·x², and R = L/|g₂ − g₁|. Curve lengths are capped at the neighbouring straights, so curves never overlap. Matches.

No finding.

## 4. Design criteria (`assets/criteria/*.json`, `src/civiel/ontwerpcriteria.rs`)

**ROA 2017.** The values were compared with the tables of *Richtlijn Ontwerp Autosnelwegen 2017* (27 November 2017), read from an online copy of the document (<https://online.flipbuilder.com/fjkp/pihc/files/basic-html/page63.html> and the following pages; the official PDF is at <https://puc.overheid.nl/rijkswaterstaat/doc/PUC_154078_31/>).

| Table | Values in the app | ROA 2017 |
|---|---|---|
| 5.4 stopping sight distance | 260 / 135 / 80 / 40 m | match (p. 56) |
| 5.15 minimum radius, main carriageway | 1500 m (120), 700 m (90) | match (p. 63) |
| 5.16 minimum radius, other carriageways | 750 / 330 / 160 / 80 m | match: the last cell of each column, at 5.0 / 6.0 / 7.0 % superelevation (p. 64) |
| 5.18–5.20 clothoids | A_min 270 / 175 / 95 / 60 m; transition needed below R 4000 / 2000 / 800 / 300 m | match (pp. 69–70) |
| 5.21 maximum grade | 3 % (120, 90), 4 % (70, 50) | match (p. 73, page image checked) |
| 5.22 eye and object height | 1.10 m, 0.50 m | match (pp. 55, 74) |
| 5.23 crest radius | 12,400 / 6,600 / 3,000 / 750 m | match (p. 74) |
| 5.25 sag radius | 24,800 / 13,200 / 6,000 / 1,500 m | match (p. 76) |
| 5.45 superelevation | 2.5 % main, 7 % other | match (p. 99) |

The stopping sight distances also follow from d = V·t/3.6 + V²/(254·f) with the reaction times and friction values the source key names (120 km/h: 83.3 + 177.2 = 260 m).

**AASHTO Green Book 2018 (metric).** Stopping sight distance (table 3-1), crest K (table 3-35) and sag K (table 3-37) match the published values, with R = 100·K. R_min = V²/(127·(e + f)) with the metric f_max values gives the listed radii (for example 50 km/h, e = 4 %: 2500/(127·0.23) = 86 m). The app marks these as computed values.

**Follow-up (VERVOLG).** ROA table 5.21 also gives a maximum length per grade (1300 m at 3 %, 700 m at 4 %). The check does not test grade length.

## 5. Sight distance (`src/civiel/zicht.rs`)

The profile check compares the steepest rise from the eye to the road with the line to the object. A unit test checks a long crest against S = √(2R)·(√h₁ + √h₂). No finding.

## 6. Rail cant (`src/civiel/spoor.rs`)

D_eq = c·V²/R with c = 1500/(9.81·3.6²) = 11.8 (standard gauge, rail head distance 1.5 m). Cant deficiency I = D_eq − D, excess E = D − D_eq(V_min), ramp rate dD/dt = (dD/ds)·V/3.6. A 1:n turnout has angle atan(1/n) and tangent R·tan(α/2). No finding. The limits in `assets/criteria/spoor.json` are marked with their sources and estimates; they were not checked against TSI INF or EN 13803 here.

## 7. Volumes and quantities

- **TIN to TIN** (`src/civiel/terrein.rs`): the two triangulations are overlaid and the height difference, linear on each piece, is integrated exactly (area × value at the centroid), split where it changes sign. Unit tests check a box, a pyramid and a tilted plane. No finding.
- **Corridor sections** (`src/civiel/corridor.rs`) and **mass haul** (`src/civiel/massa.rs`): average end areas between stations; the mass haul diagram reads the corridor list by the right columns (distance, cut m³, fill m³). No finding.
- **Quantities** (`src/hoeveelheden.rs`): polyline areas add the circular segment of each bulge with the bulge's sign. No finding.

## 8. Parcels (`src/civiel/percelen.rs`) — error

**Finding.** `CIV_PERCELEN` and `CIV_PERCEEL_INFO` read the plot (and the street line) through `outline()`, which keeps the vertices of a polyline but drops their bulges. An arc in the plot boundary becomes its chord.

**Source.** The area between a chord and its arc is the circular segment R²/2·(θ − sin θ). `CIV_HOEVEELHEDEN` already measures the same polyline this way.

**Test values.**

| Plot | Exact area | `CIV_PERCELEN` before the fix |
|---|---|---|
| 40 × 30 m with the back an arc of 60°, R = 40 m | 1,344.94 m² | 1,200.00 m² (−144.94 m²) |
| A street front of 20 m on R = 50 m | segment 13.50 m² | 13.50 m² missing from that parcel |

So "4 equal parcels" of the first plot get 300 m² each on paper, and their real areas differ. The drawn parcels also follow the chord, not the arc.

**Fix.** PR #105 follows each arc with chords at most 0.1 mm from it, so the areas, the clipping and the drawn parcels follow the arc. Parcel areas that users already took from arched plots change.

## 9. Geotechnics (`src/civiel/geotechnics/`, `src/gis/bro.rs`)

- **Stress below a loaded rectangle**: the Newmark/Fadum influence factor, with `atan2` so the case m²n² > m² + n² + 1 is right. I(1, 1) = 0.1752 matches Fadum's table; under a wide load the factor goes to 0.25. Unit tests check this and compare with a numerical integration of Boussinesq.
- **Consolidation**: U(T) = 1 − Σ 2/M²·exp(−M²T), with √(4T/π) below T = 0.01. Matches Terzaghi.
- **Koppejan and compression ratios**: the formulas match; C_p below the preconsolidation stress is ln 10 / RR, consistent with the Terzaghi form.
- **Bishop and Fellenius**: m_α = cos α + sin α·tan φ'/F; circles with m_α < 0.2 are rejected. Matches.
- **CPT classification**: I_SBT = √((3.47 − log(qc/pa))² + (log Rf + 1.22)²) and the zone limits 3.60 / 2.95 / 2.60 / 2.05 / 1.31 match Robertson (2010).

**Table 2.b (VERMOED, check against the standard).** In the table the swelling ratio Csw/(1+e₀) is one third of the compression ratio, rounded, in every row but three: `grind_zwak_siltig_matig` and the first value of `zand_schoon_vast` give 0.0006 where C'p = 1000 gives 0.0023/3 = 0.0008, and `grind_sterk_siltig_vast` with the same C'p gives 0.0008. It may be a copy error in the public reproduction or in the code. The effect on settlement is negligible (sand and gravel), so this is a follow-up: check these three cells against NEN 9997-1.

## 10. Parking (`src/library/parking.rs`)

The row depths P1 of NEN 2443 tables 4A–4C follow P1 = 5.13·sin α + 1.87·cos α for every angle (30°: 4.18, 45°: 4.95, 60°: 5.38, 70°: 5.46). So the copied tables are internally consistent. They were not compared with the standard itself, which is not public.

## 11. AHN heights (`src/gis/ahn.rs`)

**Source.** GeoTIFF 1.0: the raster type key 1025 (`GTRasterTypeGeoKey`) says whether the tie point is the corner of a pixel (PixelIsArea, 1) or its centre (PixelIsPoint, 2).

**Test.** A 2 × 2 m tile of `dtm_05m` from the PDOK AHN WCS (box x 104850–104852, y 425300–425302) has key 1025 = 1, pixel size 0.5 m and tie point (104850, 425302), the top-left corner of the box. `decode_geotiff` takes the tie point as the corner and `Grid::sample` puts heights at the pixel centres. So a height is read at the right place. No finding.

**Follow-up (VERVOLG).** Neither `ahn.rs` nor `rasterbestand.rs` reads key 1025. A local GeoTIFF written as PixelIsPoint would be read half a cell off.

## Seen in passing, outside the calculation core

**CityGML rings with more than one segment** (`src/gis/citygml.rs`, `ring()`). The function takes the first `posList` below a ring element and stops. A `gml:Ring` made of several `curveMember`s, or a `gml:Curve` with several `segments` (for example a `LineStringSegment` followed by a `gml:Arc`), keeps only its first segment. The surface is then cut short, or dropped when that segment has fewer than three points, without a warning. The GML reader in `src/gis/files.rs` joins linear segments and reports arcs as unsupported; the CityGML reader does neither. Code reading only (BEVESTIGD for the code path); whether IMGeo files from the BGT download use such rings was not checked (VERMOED). Left for the readers work, not fixed here.

## Not checked

- The Rust tests for RDNAPTRANS with the official grids (`cargo test --lib rdnap -- --ignored`) were not run to the end on this machine; the Python port stands in for them above.
- HydX (`src/civiel/hydx.rs`), the geodata export and NLCS++: under repair in other PRs.
- IFC 4.3 alignment parameters (signs of radii and clothoid constants) against IfcOpenShell: covered by `docs/ifc43-validatie.md`.
- Hydrology (drop paths, Priority-Flood): algorithms without a normative value to test against.
