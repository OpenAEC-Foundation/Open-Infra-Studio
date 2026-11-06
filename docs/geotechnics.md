# Geotechnics: soil profiles, slope stability and settlement

Open Infra turns the subsurface data of a project into three everyday
geotechnical checks:

- **`CIV_GRONDPROFIEL`**: a soil profile along a line, from the CPTs and
  boreholes of the Dutch key register (BRO), kept as a soil model.
- **`CIV_STABILITEIT`**: the stability of a slope in a cross section (a dike,
  an embankment or a ditch bank), with Bishop's simplified method or the
  ordinary method of slices.
- **`CIV_ZETTING`**: the settlement of a fill, over an area and in time.

They sit in the **Grading** group of the **Civiel** ribbon, in the
**Geotechnics** menu. The soil profile from the BRO appears there when the Dutch
modules are on. Stability and settlement also work anywhere in the world with
layers you give yourself (`LAGEN=`).

> **A calculation aid, not a stamped design.** The defaults come from a cited
> table, and the values that have no source are marked as estimates. The
> results depend on the soil parameters, the phreatic level and the load. An
> engineer must check them, together with the assumptions below, before any
> design decision is based on them. Every result and drawing says so.

## Soil units and their parameters

### Table

`CIV_GRONDSOORTEN` lists the soil units with their codes and values.
Each unit is a row of **NEN 9997-1 table 2.b** ("Karakteristieke waarden van
grondeigenschappen"), the Dutch table of characteristic soil properties by
main soil, admixture and consistency. The rows are, for example,
`klei_schoon_slap` (clay, clean, soft), `veen_slap` (peat, not preloaded,
soft) and `zand_schoon_vast` (sand, clean, dense). English codes such as
`clay_clean_soft` are accepted as well.

Per unit the table gives these values:

| Symbol | Meaning | Unit |
|---|---|---|
| γ, γsat | unit weight above and below the phreatic level | kN/m³ |
| qc | representative cone resistance | MPa |
| E100 | stiffness | MPa |
| φ', c' | effective friction angle and cohesion | °, kPa |
| cu | undrained shear strength (cohesive soils) | kPa |
| C'p, C's | Koppejan's primary and secondary compression coefficients | – |
| Cc/(1+e0), Csw/(1+e0), Cα | compression ratio, reloading (swelling) ratio and secondary compression coefficient | – |

### Source and status

- **The standard.** NEN 9997-1+C2:2017 is sold by NEN and is not freely
  available.
- **Where the values come from.** They were taken from a public reproduction of
  table 2.b, the *Constructief tabellenboek voor bouwkunde en civiele techniek*.
  Its preview edition at tabellenboek.eu has the table under "Grondeigenschappen"
  on pp. 269–270. Part of the values were checked against public Dutch
  geotechnical reports that quote the table.
- **Consistency check.** The unit tests check every row against the table's
  own relation Cc/(1+e0) = ln 10 / C'p, within 3 %.
- **Before you rely on them,** check the values against your copy of the
  standard.
- **Ranges.** Where the table gives a range (the dense, stiff and strongly mixed
  rows), the default is the **first value**: the lower strength and stiffness,
  with the matching higher compression ratio.
- **Not in the table** (all of them are estimates):
  - **cv**, the consolidation coefficient. **Estimate**, as an order of
    magnitude for Holocene Dutch deposits:
    - clean and organic clay: 3·10⁻⁸ m²/s;
    - slightly sandy clay: 10⁻⁷ m²/s;
    - very sandy clay: 10⁻⁶ m²/s;
    - loam: 3·10⁻⁷ m²/s;
    - peat: 10⁻⁷ m²/s;
    - sand and gravel drain freely.

    Replace cv with oedometer values.
  - **Cp below the preconsolidation stress** is taken as ln 10 / (Csw/(1+e0)),
    the table's reloading ratio. That is about 3 × C'p, in line with the
    common practice of taking 3 to 4 times C'p. Cs below the preconsolidation
    stress is C's · Cp / C'p (**estimate**).
  - **POP** (pre-overburden pressure, σ'p − σ'0) defaults to 0, which means
    normally consolidated. That is conservative for settlement.

### Your own values

Set your own values per unit with `GROND=`:

```
GROND=veen_slap:c=2,phi=17,cv=2e-7;klei_schoon_slap:cc=0.6,e0=1.8,pop=10
```

The keys are:

- `g`, `gsat`: unit weights;
- `c`, `phi`: strength;
- `cp`, `cs`, `cppre`, `cspre`: Koppejan;
- `cr`, `rr`, `ca`: compression ratios;
- `cc` and `e0` together, which give cr = cc/(1+e0);
- `cv` in m²/s (0 means free draining);
- `pop` in kPa.

The report keeps the source of every value. Overridden values are marked as
"own values".

## The soil model

### Classification

- **CPT layers.** Each CPT layer is classified by Robertson's (2010) soil
  behaviour type (P.K. Robertson, "Soil behaviour type from the CPT: an
  update", CPT'10). The classification is computed from qc and Rf, as
  `CIV_SONDERINGEN3D` already does. The zone gives the main soil and the
  admixture, and the layer's mean qc gives the consistency: the row with the
  nearest representative qc on a log scale.

  | SBT zone | Unit |
  |---|---|
  | 2 | peat |
  | 3 | clean clay |
  | 4 | slightly sandy clay |
  | 5 | very sandy clay or very silty sand, by qc |
  | 6 | clean sand |
  | 7 | medium or dense clean sand |

- **Borehole layers.** These are classified by their described name (NEN-EN-ISO
  14688 `geotechnicalSoilName` or NEN 5104 `soilNameNEN5104`):
  - the main noun gives the soil;
  - "zwak/matig/sterk zandig", "humeus", "venig", "siltig" and "kleiig" give the
    admixture;
  - a borehole gives no consistency, so the soft or loose row is taken (the
    less favourable one);
  - made ground (rubble, pavement) is taken as loose sand (**estimate**).
- **Joining layers.** Consecutive layers of the same unit are joined. Layers
  thinner than `MINLAAG=` (default 0.3 m) join their thicker neighbour, and
  layers without a measurement take the unit below them. The first unit
  starts at the surface level of the test.
- **Shallow tests.** Columns less than 2 m deep are left out (and listed as
  skipped), because their last unit would stand for the whole depth.

### Interpolation between columns

The method, as implemented in `src/civiel/geotechnics/model.rs`:

1. **Matching.** The unit sequences of two neighbouring columns are matched in
   order, with a dynamic-programming alignment as in sequence correlation:
   - two units may match when they have the same main soil (gravel, sand,
     loam, clay or peat);
   - a match weighs the smaller thickness, divided by 1 + |Δ mid level| / 2 m;
   - the weight counts 0.6 when the rows differ in admixture or consistency.

   The alignment with the largest total weight wins.
2. **Layers.** Matched units form one layer between the columns. An unmatched
   unit is a **wedge**: it has its full thickness at its own column and none
   at the neighbour, at the place the neighbour's sequence puts it.
3. **Boundaries.** Every boundary is interpolated **linearly in elevation**
   between the two columns. Boundaries never cross, and the model is exact at
   the columns.
4. **Rows within a layer.** A matched pair of different rows, such as soft and
   firm clay, changes row halfway between the columns.
5. **Ends and depth.**
   - Before the first column and after the last, the nearest column holds.
   - Below the end of a test, its last unit continues down to the model base,
     which is the deepest end of investigation.
6. **The ground.**
   - Where the ground (TIN or AHN) is higher than the model top, the top unit
     continues up to it.
   - Where it is lower, the soil above it is cut away.
7. **In the calculations** (not in the drawing), bands thinner than 0.2 m
   join their thicker neighbour. These are the thin ends of wedges; near the
   surface the log laws of compression would otherwise give them a large
   strain at almost no effective stress.

The model is stored in the drawing (XRecord `OPENINFRA/GRONDMODEL`) under its
name, with its line in project coordinates, its columns and its unit
parameters. `CIV_STABILITEIT` and `CIV_ZETTING` take the soil from it:

- they use `MODEL=<name>`, or the stored model whose line passes nearest;
- without a model, they use `LAGEN=` or the BRO around the section or area.

For an area without a model, the nearest column holds under each point.

## `CIV_GRONDPROFIEL`

```
CIV_GRONDPROFIEL [LIJN=<handle>|AS=<handle>] [BRON=sonderingen|boringen|alles]
                 [STRAAL=50] [MAX=30] [MINLAAG=0.3] [NAAM=] [GROND=] [PLAATS=x,y] [ID=]
```

- **What it collects.** The BRO CPTs (`publiek.broservices.nl`, CC0) and
  geotechnical boreholes within `STRAAL` m of the line (or the selected line or
  axis), at most `MAX`, the nearest first.
- **What it draws.** A profile view, placed below the project or at `PLAATS`:
  - the soil units as coloured hatches (MO-BODEM_BOORPROFIEL-A) with their
    boundaries;
  - the columns themselves, with their BRO id and end level
    (MO-ONDERZOEK_SONDERING / _BORING);
  - the ground line from the TIN or AHN (GW-MAAIVELD);
  - a height grid in m NAP and the stations;
  - a legend with every unit's γ/γsat, φ', c', C'p and C's.
- **Vertical scale.** The vertical exaggeration is the `OVERHOGING` setting.
- **Result.** The JSON result holds the whole model.

## `CIV_STABILITEIT`

```
CIV_STABILITEIT [LIJN=<handle> | AS=<handle> STATION=<m> [LINKS=] [RECHTS=]]
                [FREATISCH=<m NAP> | FREATISCH=x:z;x:z;…] [BELASTING=kPa:from:to]
                [METHODE=bishop|fellenius] [OPPERVLAK=AUTO|B|N] [MINDIEPTE=0.5]
                [LAGEN=top:code;… [BASIS=]] [MODEL=] [GROND=] [BRON=] [STRAAL=]
                [NAAM=] [PLAATS=x,y] [ID=]
```

### The section

- **Where it lies.** Along a drawn line (x = distance along it), or square to
  an axis at a station (x = offset, negative to the left).
- **The surface.** It is sampled every 0.25 m. It follows the design TIN (N)
  where there is one and the existing TIN (B) or AHN elsewhere
  (`OPPERVLAK=AUTO`). It can also be made to follow only one of them.
- **The soil.** It comes from the model every 0.5 m, as above.

### Factor of safety

The slip surfaces are circular, in effective stresses.

- **Ordinary method of slices** (Fellenius 1936):

  F = Σ [c'·b/cos α + max(0, W·cos α − u·b/cos α)·tan φ'] / Σ W·sin α

- **Bishop's simplified method** (Bishop, A.W. 1955, "The use of the slip
  circle in the stability analysis of slopes", *Géotechnique* 5(1), 7–17):

  F = Σ [c'·b + (W − u·b)·tan φ'] / m_α / Σ W·sin α,  m_α = cos α + sin α·tan φ'/F

  It is solved by iteration from the Fellenius value until |ΔF| < 10⁻⁷. A
  circle with m_α < 0.2 in any slice is rejected, after Whitman, R.V. &
  Bailey, W.A. 1967, "Use of computers for slope stability analysis",
  *J. Soil Mech. Found. Div.* ASCE 93(SM4), 475–498.

The symbols:

- **b**: the slice width (50 slices per circle).
- **α**: the inclination of the slice base.
- **c', φ'**: the strength of the unit at the base.
- **W**: the slice weight. It counts γ above the phreatic line and γsat below
  it, plus free water standing on the surface (a ditch, γw · depth) and the
  surcharge `BELASTING=q:from:to` (kPa over x from–to).
- **u**: the pore pressure at the base, γw · (phreatic level − base level),
  hydrostatic below the phreatic line. This is the usual simplification of
  the flow net. γw = 9.81 kN/m³.

### Search for the critical circle

1. **Grid.** A grid of 21 × 15 centres:
   - x over the section (5 % in from each end);
   - z from 0.2 H to 2.5 H + 2 m above the highest ground, where H is the
     height difference of the section.
2. **Radii.** For every centre, circles tangent to 16 levels from just below
   the lowest ground down to 2 H + 5 m deeper (or the model base), and to six
   levels within the slope height (slips in the slope face). Circles tangent
   to **every layer boundary** in that range are added.
3. **Refinement.** A compass (pattern) search on centre x, centre z and the
   tangent level, halving the steps until they are below 2 cm.
4. **Valid circles.** A circle must enter and leave the ground inside the
   section, stay above the model base and reach at least `MINDIEPTE` (0.5 m)
   below the ground. This keeps out skin slips in cohesionless soil, whose
   factor tends to tan φ'/tan β.
5. **Both directions.** Slopes facing either way are found: the sliding
   direction follows from the sign of Σ W sin α.

The search runs for the chosen method and for the other one. The report gives
both minima and the other method's factor on the critical circle.

### Phreatic level and defaults

- **Default phreatic level.** Without `FREATISCH`, the phreatic level is
  horizontal at the **lowest ground level of the section**, a stand-in for the
  polder ditch level. This is an **assumption**, stated in the drawing. Give
  the real level, or a line `FREATISCH=x:z;x:z` in section coordinates.
- **No surcharge** unless you give one (e.g. 13 kPa traffic: `BELASTING=13:2:6`).

### Output

- **The drawing**, at 1:1:
  - the section with the soil in colour, the ground and the phreatic line;
  - the load;
  - the **critical circle** with its slices, centre and radii;
  - the centre grid;
  - F in the drawing (MO-BODEM-G/T).
- **The JSON report**:
  - `factor_of_safety`;
  - `critical_circle`: centre, radius, entry, exit, direction;
  - `other_method`;
  - `slices`: x, b, top, base, α, W, u, c', φ' and unit of every slice;
  - `units`;
  - `surface`, `phreatic`, `load`;
  - `grid`, with each centre's lowest F.

## `CIV_ZETTING`

```
CIV_ZETTING [GEBIED=x,y;x,y;… HOOGTE=<m> | ONTWERP=ja [GEBIED=…]] [GAMMA=18] [TIJD=30]
            [CEL=] [PUNT=x,y] [METHODE=koppejan|terzaghi]
            [FREATISCH=<m NAP> | GRONDWATER=<m below ground, 1>] [MARGE=5] [BANDEN=8]
            [LAGEN=…] [MODEL=] [GROND=] [BRON=] [STRAAL=] [ID=]
```

### Inputs

- **The fill** is one of two:
  - `HOOGTE` m over the polygon `GEBIED`;
  - the design surface (N) above the existing ground (B, or AHN), optionally
    inside `GEBIED`.
- **Fill unit weight.** It is `GAMMA` kN/m³. The default 18 is an
  **estimate** for a sand fill.
- **The grid.** The fill is sampled on a grid of `CEL` m (by default 1/30 of
  the area, at most 60 × 60 points), with 3 × 3 samples per cell. The grid
  reaches `MARGE` m beyond the fill.
- **The phreatic level.** It is `FREATISCH` (m NAP). Without it, it lies
  `GRONDWATER` m below the existing ground: 1 m by default, an **assumption**.
- **The existing ground** comes from the TIN (B), and from AHN (or the
  worldwide terrain) where the TIN has no height.

### Stress increase

The stress increase follows Boussinesq's (1885) solution for a point load on
an elastic half space. It is integrated over a uniformly loaded rectangle by
Newmark (1935, *Simplified computation of vertical pressures in elastic
foundations*, Univ. of Illinois Eng. Exp. Station Circular 24). Fadum
tabulated it (1948, "Influence values for estimating stresses in elastic
foundations", Proc. 2nd ICSMFE Rotterdam, vol. 3, 77–84).

Under the corner of a rectangle B × L, at depth z, with m = B/z and n = L/z:

```
Δσz = q · I(m, n)
I   = 1/(4π) · [ 2mn√(m²+n²+1) / (m²+n²+m²n²+1) · (m²+n²+2)/(m²+n²+1)
               + atan2( 2mn√(m²+n²+1), m²+n²+1 − m²n² ) ]
```

- **Superposition.** Every loaded cell (q = γ · h) adds four corner rectangles
  with signs, so the stress under any point is exact for the stepped load.
- **No other corrections.** There is no correction for a fill that sinks below
  the water table, and the stress increase acts at the existing ground.

### Compression per sublayer

Each layer is divided into sublayers of at most 0.5 m. For each sublayer:

- σ'0 is taken at its middle: γ above the phreatic level and γsat below, less
  the hydrostatic pore pressure;
- σ'p = σ'0 + POP;
- σ'e = σ'0 + Δσ.

**Koppejan** (1948; the usual Dutch method). With t₀ = 1 day, the
preconsolidation branch applies up to min(σ'e, σ'p):

```
ε = (1/Cp + log₁₀(t/t₀)/Cs) · ln(σ'p/σ'0) + (1/C'p + log₁₀(t/t₀)/C's) · ln(σ'e/σ'p)
```

**Terzaghi**, with compression ratios:

```
ε = RR · log₁₀(σ'p/σ'0) + CR · log₁₀(σ'e/σ'p),   CR = Cc/(1+e0), RR = Csw/(1+e0)
```

It has a secondary compression Cα · log₁₀(t/t_p) after the end of primary
consolidation, after Mesri, G. 1973, "Coefficient of secondary compression",
*J. Soil Mech. Found. Div.* ASCE 99(SM1), 123–137. The end of primary, t_p,
is taken at U = 95 %.

### Consolidation in time

Consolidation in time follows Terzaghi (1925), with Uv = 1 − Σ 2/M² ·
exp(−M²·Tv), M = π(2k+1)/2 and Tv = cv·t/H_dr². For Tv < 0.01 it uses
√(4Tv/π).

- **Packages.** Consecutive clay, loam and peat layers between sand layers (or
  the surface) consolidate as one package.
- **Transformed thickness.** The package's thickness is transformed to the
  reference layer, the thickest: H' = Σ Hᵢ · √(cv,ref / cv,ᵢ). This is a
  standard **approximation** for layered soil.
- **Drainage length.** It is H'/2 with sand below, and H' on the
  (impermeable) model base. The fill and the surface drain.
- **Settlement at time t.**
  - Koppejan: s(t) = U(t) · s_K(t), so the secular term develops with
    consolidation (a common simplification).
  - Terzaghi: s(t) = U(t) · s_p + s_s(t).

### Output

- **The map.** Colour bands (tool layers `OI-ZETTING-nn`) and contours at the
  band edges, labelled in mm (GW-HOOGTE_ZETTING-G/T). Values in mm per point
  are thinned so that they stay 12 mm apart on paper.
- **The fill outline** (GW-OPHOGING-WERK).
- **A legend.**
- **A settlement–time graph** (log time, from 1 day to `TIJD` years) for
  `PUNT`, or for the point of largest settlement.
- **The JSON result** has:
  - every grid point (fill thickness, primary and total settlement);
  - the chosen point's layers (σ'0, Δσ, primary and secondary settlement) and
    its consolidation packages (t50 and t90);
  - the time curve.

## Tests

### Unit tests

The unit tests (`cargo test --lib geotechnics`) check against published
values:

- **Slope stability.**
  - **ACADS test problem 1(a).** Giam, P.S.K. & Donald, I.B. 1989, *Example
    problems for testing soil slope stability programs*, Civil Engineering
    Research Report 8/1989, Monash University. The referee factor of safety
    is 1.00; it is repeated in Donald 1994, *Proc. XIII ICSMFE* New Delhi,
    Example 1. Our Bishop must lie in 0.97–1.01, Fellenius in 0.92–0.97 and
    below Bishop. Published runs give about 0.99 and 0.95.
  - **Taylor's stability number.** For φ = 0 and a 60° slope, Ns = 0.191
    (Taylor, D.W. 1937, "Stability of earth slopes", *J. Boston Soc. Civil
    Eng.* 24(3), 197–246). It is checked for both methods, within ±0.004.
  - **The infinite slope.** A cohesionless slope gives tan φ'/tan β.
  - **Bishop = Fellenius** for φ = 0.
- **Stress under a rectangle corner.**
  - The Fadum (1948) / Newmark (1935) values I(1,1) = 0.1752,
    I(2,2) = 0.2325, I(0.5,0.5) = 0.0840, I(0.5,1) = 0.1202 and
    I(1,2) = 0.1999.
  - A numerical integration of Boussinesq's point load.
- **Consolidation.**
  - **Single clay layer.** Das, B.M., *Principles of Geotechnical
    Engineering*, 7th ed. (Cengage, ISBN 978-1-133-10866-5), Example 11.4(a),
    p. 323: a normally consolidated clay layer gives Sc = 0.21 m.
  - **Time.** Example 11.13, p. 356: T = 0.36 gives U ≈ 0.67.
  - **Terzaghi's time factors.** T50 = 0.197 and T90 = 0.848.
  - **Koppejan by hand.**
- **Soil table and model.**
  - The table relation Cc/(1+e0) = ln 10 / C'p for every row.
  - Classification of CPT and borehole layers.
  - Interpolation and wedges.

### End-to-end test

The e2e test `tests/e2e/geotechniek_e2e.py` runs in Open CAD Studio v2026.44
at two Dutch locations with BRO CPTs:

- **A dike at Wieldrecht, Dordrecht.** It makes a soil profile across the dike
  through four CPTs, 20 to 28 m deep. On that dike section it runs the
  stability three times: Bishop on the soil model; Fellenius with a phreatic
  line and 13 kPa traffic on the crest; and with layers given by hand, with
  own peat parameters.
- **The polder at Moordrecht, near Gouda**, at about −2 m NAP. It makes a soil
  profile along a row of CPTs from 2026, then computes the settlement of a
  1.5 m fill with Koppejan and with Terzaghi.

It saves the drawing, window captures and PDF plots of every view to
`target/e2e-geotechniek/`.

## Known limits

- **Slip surfaces.** Only circular slip surfaces are searched, so there is no
  uplift (Uplift-Van) or non-circular search. The search can miss a critical
  circle outside its grid, although the pattern search can move beyond the
  grid.
- **Pore pressures.**
  - Hydrostatic below one phreatic line: there are no head differences in
    aquifers and no flow net.
  - The default phreatic level is an assumption.
- **Strength.** Effective-stress strength only: no undrained (cu) analysis and
  no partial factors. The results are characteristic factors of safety, which
  you compare with your own required value.
- **The soil model.**
  - It is two-dimensional along its line. Away from the line (and for areas
    without a model), the nearest station or column holds.
  - Borehole layers get the soft or loose row.
- **Settlement.**
  - One-dimensional per point: no lateral deformation and no immediate
    (elastic) settlement.
  - No correction for a fill that sinks under water, and no unloading.
  - The equivalent-layer consolidation of layered packages is an
    approximation.

## References

- Bishop, A.W. (1955). The use of the slip circle in the stability analysis of
  slopes. *Géotechnique* 5(1), 7–17.
- Boussinesq, J. (1885). *Application des potentiels à l'étude de l'équilibre
  et du mouvement des solides élastiques*. Gauthier-Villars, Paris.
- Das, B.M. (2010). *Principles of Geotechnical Engineering*, 7th ed. Cengage
  Learning. Examples 11.4 (p. 323) and 11.13 (p. 356).
- Donald, I.B. (1994). The extended CRISS method for calculating slope
  stability. *Proc. XIII ICSMFE*, New Delhi (ISSMGE online library).
- Fadum, R.E. (1948). Influence values for estimating stresses in elastic
  foundations. *Proc. 2nd ICSMFE*, Rotterdam, vol. 3, 77–84.
- Fellenius, W. (1936). Calculation of the stability of earth dams. *Trans.
  2nd Congress on Large Dams*, Washington, vol. 4, 445–462.
- Giam, P.S.K. & Donald, I.B. (1989). *Example problems for testing soil slope
  stability programs*. Civil Engineering Research Report 8/1989, Monash
  University.
- Koppejan, A.W. (1948). A formula combining the Terzaghi load-compression
  relationship and the Buisman secular time effect. *Proc. 2nd ICSMFE*,
  Rotterdam, vol. 3, 32–37.
- Mesri, G. (1973). Coefficient of secondary compression. *J. Soil Mech.
  Found. Div.* ASCE 99(SM1), 123–137.
- NEN 9997-1+C2:2017. *Geotechnisch ontwerp van constructies — Deel 1:
  Algemene regels*, table 2.b. NEN, Delft (not public; values via the
  reproduction in the *Constructief tabellenboek voor bouwkunde en civiele
  techniek*, preview at tabellenboek.eu, pp. 269–270).
- Newmark, N.M. (1935). *Simplified computation of vertical pressures in
  elastic foundations*. Univ. of Illinois Engineering Experiment Station,
  Circular 24.
- Robertson, P.K. (2010). Soil behaviour type from the CPT: an update. *2nd
  Int. Symp. on Cone Penetration Testing (CPT'10)*, Huntington Beach.
- Taylor, D.W. (1937). Stability of earth slopes. *J. Boston Soc. Civil Eng.*
  24(3), 197–246.
- Terzaghi, K. (1925). *Erdbaumechanik auf bodenphysikalischer Grundlage*.
  Deuticke, Vienna.
- Whitman, R.V. & Bailey, W.A. (1967). Use of computers for slope stability
  analysis. *J. Soil Mech. Found. Div.* ASCE 93(SM4), 475–498.
