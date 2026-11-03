# Exchange: LandXML 1.2 export (and corridors in IFC 4.3)

The Civiel tab reads LandXML (points, surfaces, alignments). `CIV_LANDXML_EXPORT` writes the model back out, so it can go to a partner who works in other civil design software. For open BIM there is the IFC 4.3 export (`GIS_IFC` / `CIV_IFC`); this stream adds the corridor's road parts and earthworks to it (see [below](#corridors-in-the-ifc-43-export)).

## The command

| Command | Arguments | Writes |
|---|---|---|
| `CIV_LANDXML_EXPORT` | `[BESTAND=<path>]` (or a path as a word), `[ALLES]`, `[ID=<name>]` | LandXML 1.2 |

- **Ribbon:** Civiel → Points (Punten) → **LandXML export**, beside the LandXML import.
- **What goes out:** with a selection, only the objects that have an entity in it (an alignment, a TIN mesh, survey points, a corridor line). Without a selection, or with `ALLES`, everything.
- **Where it goes:** without a path the file goes beside the drawing: `<drawing>_landxml.xml`. The ribbon button always writes there, because the plugin API has no save dialog.
- **Result:** with `ID=` the JSON result lists the file, the number of points, the surfaces (name, status, points, triangles), the alignments (name, length, start station, segments, ground profile points, PVIs) and the corridors (name, feature lines).

From a script or an agent (MCP tool `civ_landxml_export`):

```
CIV_LANDXML_EXPORT BESTAND=/data/dordrecht.xml ALLES ID=lx
```

## Where the data comes from

The command reads the drawing the way the other civil commands do: through the object index `OPENINFRA/INDEX` and the object data (XDATA `OPENINFRA`) on the entities, so it also works on drawings too large to share whole. All coordinates are converted to project coordinates: metres in the site's coordinate system (RD in the Netherlands), heights in metres NAP. A drawing in millimetres or with relative coordinates gives the same file.

| Object | Found as |
|---|---|
| Survey points | POINTs on `OG-HOOGTEPUNT`, or with a point number in their object data |
| Surfaces | meshes and 3D faces on `OG-TRIANGULATIE`; status B (existing) or N (design) from the layer; one surface per object data record, so two TINs never merge; the name from `terrein` (or `corridor`) |
| Alignments | polylines on `AM-AS` with an alignment name, without offset, design profile or corridor data; the start station from `beginstation` |
| Ground profile | sampled along the alignment every `STAP` m (and at every tangent point) on the existing TIN (B); with a length profile but no TIN, from AHN |
| Design profile | the PVIs (`pvi`) and curve length (`boog_m`) of the design profile, with the same parabolic curves the drawing shows |
| Corridors | the feature lines of the index entry `corridor`, the design surface by its `corridor` data, the alignment and assembly from the corridor's 3D axis line |

## What the file holds

| LandXML | From |
|---|---|
| `Units/Metric` | metres, square and cubic metres; angles in decimal degrees |
| `CoordinateSystem` | the site's CRS: `name`, `epsgCode`, `verticalDatum` (`NAP`, or `MSL` outside the Netherlands) |
| `Project`, `Application` | the site name; Open Infra and its version |
| `CgPoints/CgPoint` | survey points: name, `code`, "N E Z"; a repeated name gets a suffix (`101-2`), as names must be unique |
| `Surfaces/Surface` | every TIN: `state="existing"` for B, `state="proposed"` for N; `Definition surfType="TIN"` with `Pnts` and `Faces` (counter-clockwise), `area2DSurf`, `elevMin`, `elevMax` |
| `SourceData/Breaklines` | a corridor's feature lines on its design surface |
| `Alignments/Alignment` | `name`, `length`, `staStart`; `CoordGeom` with `Line` (Start, End) and `Curve` (`rot`, `radius`, `length`, `delta`, `chord`; Start, Center, End, PI), each with its `staStart` |
| `Profile/ProfSurf` | the ground line: station and height pairs |
| `Profile/ProfAlign` | the design: `PVI` at both ends, `ParaCurve length=…` at every PVI with a vertical curve |
| `PlanFeatures/PlanFeature` | per corridor, its feature lines (pavement edge, shoulder, crest, toe and top of slope, axis) as 3D `Line`s |

Northing comes before easting everywhere, as LandXML requires and as the reader expects. Alignment geometry is written to 6 decimals (so tangency survives), the rest to 4 (0.1 mm).

**Checked:**
- the file validates against the [LandXML 1.2 schema](http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd);
- read back with `CIV_LANDXML` it gives the same points, the same surfaces (triangle for triangle) and the same alignments (lengths to 1 mm); the unit test `exports_and_reads_back` and the e2e test check this.

## Corridors in the IFC 4.3 export

Main's IFC 4.3 export (`src/ifc/`) maps every object to its IFC class by its NLCS layer and object data. A corridor became an `IfcRoad` holding its design surface as terrain. Now the design surface is split into what it is:

| IFC | What |
|---|---|
| `IfcRoadPart` `CARRIAGEWAY` | holds an `IfcPavement` (`FLEXIBLE`) with an `IfcCourse` `PAVEMENT`: the lanes' top surface |
| `IfcRoadPart` `SOFTSHOULDER` | an `IfcCourse` `PROTECTION`: the shoulders (a dike's crest) |
| `IfcRoadPart` `ROADSIDE` | an `IfcCourse` `PROTECTION`: the slopes |
| `IfcEarthworksFill` `EMBANKMENT` | a closed triangulated body between the design surface and the existing ground, where the design lies higher; `OpenInfra_Earthworks.Volume` (m³) and `Closed` |
| `IfcEarthworksCut` `CUT` | the same where the design lies lower; it voids the existing ground (`IfcRelVoidsElement`) |
| `IfcRelPositions` | the corridor's alignment positions the road |

The corridor's feature lines (asphalt edge, shoulder, toe and top of slope) go into the road too: they are found by running through the design surface, as the corridor gives only its axis line object data. A dike corridor (no lanes) gets its crest, slopes and dike body in the road without road parts.

**How the surface is split:** each triangle by the offset of its centre from the alignment: up to the pavement edge (the widest asphalt feature line) pavement; up to the shoulder edge, shoulder; up to the crest edge (dike), crest; the rest slope.

**How the earthworks are built:** the ground height under every corner of the design surface comes from the existing TIN (status B). Where the design lies above the ground, each triangle is clipped to that part: its top is the design, its bottom the ground, and the two meet where the thickness is zero. Walls close the body along the open border of the corridor. The result is watertight; two parts of a body may touch along a line where the design meets the ground. The volume follows from the faces. Thinner than 0.2 mm counts as no thickness. Cut is the same with ground and design swapped, and is only written when the existing ground is in the file (a cut must void something).

**Checked:** the unit test `a_corridor_becomes_road_parts_and_earthworks` writes `target/ifc-test/corridor.ifc`; `tests/ifc_validate.py` reports no schema or EXPRESS rule errors. The fill volume equals the hand-computed cross-section area times the length. In the Dordrecht e2e test the fill and cut volumes agree with the corridor's own average-end-area volumes within the tolerance of the test (15 %).

## Spirals

The as-spoor stream adds `Seg::Spiral` (a clothoid). The exhaustive matches on `Seg` show the merge where spirals go, and each documents what a spiral needs:

- `uitwisseling::seg_to_project`: move the start point, divide length and radii by the drawing unit;
- `landxml::coord_geom_segment`: `<Spiral length radiusStart radiusEnd rot spiType="clothoid">` with Start, PI, End (radii "INF" for a straight end), which the reader takes;
- IFC: `src/ifc/alignment.rs`, `horizontal()`, has the marked place: an end radius in `HSeg`, kind `CLOTHOID`, and in `mod.rs` an `IFCCLOTHOID` parent curve with the constant and SegmentStart from `alignment::clothoid` (ready and tested, after the ifcopenshell alignment API).

## Validation tools

`tests/e2e/uitwisseling_check.py` checks files offline:

- LandXML against the LandXML 1.2 XSD (lxml);
- IFC with ifcopenshell: the file opens, `ifcopenshell.validate` with EXPRESS rules reports no issues, every body gives geometry, the alignments are read back (segment types, lengths from the layout and from the evaluated curves, start station), the georeferencing, the road parts, courses and earthworks.

```
python3 -m venv target/e2e-uitwisseling/venv
target/e2e-uitwisseling/venv/bin/pip install lxml ifcopenshell pytest
target/e2e-uitwisseling/venv/bin/python tests/e2e/uitwisseling_check.py --landxml file.xml --ifc file.ifc
```

`tests/e2e/uitwisseling_e2e.py` builds a model in Dordrecht in Open CAD Studio (TIN from AHN, survey points, an alignment from PIs with a curve, length and design profile, a corridor) and checks the LandXML export, its round trip, the export of a selection, the IFC export of the corridor, and a drawing in millimetres.

## Limits

- **No spirals yet:** they come with `Seg::Spiral` (see above).
- **No cross sections, superelevation or assembly in LandXML:** the corridor goes out as its surface (with breaklines) and feature lines.
- **The reader** (`CIV_LANDXML`) does not read profiles back; the round trip compares points, surfaces and alignments.
- **Pavement build-up (IFC):** the assemblies have no layer thicknesses, so the pavement and shoulders are surfaces (the top), not solids. The fill body under them is a solid.
- **Ground under the corridor (IFC):** taken at the corners of the design triangles; ground relief within one triangle is not followed. That is why the IFC volumes differ a little from the corridor's section volumes.

## Sources

- LandXML 1.2 schema: <http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd>.
- IFC 4.3 ADD2 (ISO 16739-1:2024), buildingSMART: road parts, courses, earthworks, `IfcRelPositions`.
- The clothoid mapping follows the ifcopenshell alignment API (IfcOpenShell, LGPL-3.0-or-later).
- No design values: the exports write what the drawing holds. The colours in the IFC file are only for viewing.
