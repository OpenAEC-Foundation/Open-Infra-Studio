# Exchange: LandXML 1.2 and IFC 4.3 export

The Civiel tab reads LandXML (points, surfaces, alignments). These two commands write the model back out, so it can go to a partner who works in other civil design software (LandXML) or to a client who asks for open BIM (IFC 4.3).

Both are in the ribbon group **Uitwisseling / Exchange** and take arguments like every `CIV_` command.

| Command | Arguments | Writes |
|---|---|---|
| `CIV_LANDXML_EXPORT` | `[BESTAND=<path>]` (or a path as a word), `[ALLES]`, `[ID=<name>]` | LandXML 1.2 |
| `CIV_IFC_EXPORT` | `[BESTAND=<path>]` (or a path as a word), `[ALLES]`, `[ID=<name>]` | IFC 4.3 (`IFC4X3_ADD2`) |

- **What goes out:** with a selection, only the objects that have an entity in it (an axis, a TIN mesh, survey points, a corridor line). Without a selection, or with `ALLES`, everything.
- **Where it goes:** without a path the file goes beside the drawing: `<drawing>_landxml.xml` and `<drawing>_ifc4x3.ifc`. The ribbon buttons always write there, because the plugin API has no save dialog.
- **Result:** with `ID=` the JSON result lists the file, the counts per object type, the alignments with their lengths and start stations, and (IFC) the fill and cut volumes.

Example, from a script or an agent:

```
CIV_LANDXML_EXPORT BESTAND=/data/dordrecht.xml ALLES ID=lx
CIV_IFC_EXPORT BESTAND=/data/dordrecht.ifc ALLES ID=ifc
```

## Where the data comes from

The commands read the drawing the way the other civil commands do: through the object index `OPENINFRA/INDEX` and the object data (XDATA `OPENINFRA`) on the entities, so they also work on drawings too large to share whole. All coordinates are converted to project coordinates: metres in the site's coordinate system (RD in the Netherlands), heights in metres NAP.

| Object | Found as |
|---|---|
| Survey points | POINTs on `OG-HOOGTEPUNT`, or with a point number in their object data |
| Surfaces | meshes and 3D faces on `OG-TRIANGULATIE`; status B (existing) or N (design) from the layer; one surface per object data record, so two TINs never merge; the name from `terrein` (or `corridor`) |
| Alignments | polylines on `AM-AS` with an axis name, without offset, design profile or corridor data; start station from `beginstation` |
| Ground profile | sampled along the axis every `STAP` m (and at every tangent point) on the existing TIN (B); with a length profile but no TIN, from AHN |
| Design profile | the PVIs (`pvi`) and curve length (`boog_m`) of the design profile, with the same parabolic curves the drawing shows |
| Corridors | the feature lines of the index entry `corridor`, the design surface by its `corridor` data, the axis and assembly from the corridor's 3D axis line |
| 3D BAG buildings (IFC) | roof and wall meshes with a BAG id; only when Open CAD Studio shares the whole drawing, because they are not in the index |

## LandXML 1.2

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

Northing comes before easting everywhere, as LandXML requires and as the reader expects. Plan coordinates of alignments are written to 6 decimals (so tangency survives), the rest to 4 (0.1 mm).

The file validates against the [LandXML 1.2 schema](http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd). Read back with `CIV_LANDXML` it gives the same points, surfaces (triangle for triangle) and alignments; the unit test `exports_and_reads_back` and the e2e test check this.

## IFC 4.3

The file follows IFC 4.3 ADD2 (ISO 16739-1:2024), written by Open Infra's own small STEP writer (ISO 10303-21).

| IFC | From |
|---|---|
| `IfcProject` | SI units (metre, square metre, cubic metre, radian), a 3D model context with `Body` and `Axis` sub-contexts |
| `IfcProjectedCRS` | `Name` the site's EPSG code, `Description` its name, `VerticalDatum` NAP (or MSL), `MapUnit` metre |
| `IfcMapConversion` | `Eastings`/`Northings` the site's base point, `OrthogonalHeight` 0, no rotation (`XAxisAbscissa` 1, `XAxisOrdinate` 0), `Scale` 1 |
| `IfcSite` | the site, with its latitude and longitude |
| `IfcAlignment` | per axis, aggregated in the project and referenced in the site |
| `IfcAlignmentHorizontal` | nested `IfcAlignmentSegment`s with `IfcAlignmentHorizontalSegment` `LINE` and `CIRCULARARC` (radius positive to the left), closed by a zero-length segment |
| `IfcAlignmentVertical` | from the design profile: `CONSTANTGRADIENT` and `PARABOLICARC` (with its `RadiusOfCurvature`), closed by a zero-length segment |
| geometry | `FootPrint`/`Curve2D`: an `IfcCompositeCurve` of `IfcCurveSegment`s (`IfcLine`, `IfcCircle`); `Axis`/`Curve3D`: an `IfcGradientCurve` (`IfcLine`, `IfcPolynomialCurve`) on it; each segment also carries its curve segment as `Axis`/`Segment`; transition codes from the joints (same position, direction, curvature) |
| stationing | an `IfcReferent` (`STATION`) at the start, linearly placed on the alignment, with `Pset_Stationing.Station` |
| terrain | every TIN that is not a corridor surface as `IfcGeographicElement` `TERRAIN` with an `IfcTriangulatedIrregularNetwork`; property set `OpenInfra_Terrain` (status, source) |
| corridor | `IfcRoad`, positioned by its alignment (`IfcRelPositions`), with `IfcRoadPart`s `CARRIAGEWAY` (an `IfcPavement` with an `IfcCourse` `PAVEMENT`), `SOFTSHOULDER` (`IfcCourse` `PROTECTION`, the shoulders) and `ROADSIDE` (`IfcCourse` `PROTECTION`, the slopes) |
| earthworks | `IfcEarthworksFill` `EMBANKMENT` and `IfcEarthworksCut` `CUT` (voiding the existing terrain): closed triangulated bodies between the design surface and the existing ground; their volume and closedness in `OpenInfra_Earthworks` |
| dike corridor | the same without the road: the crest, slopes and the dike body (`IfcEarthworksFill`) in the site |
| 3D BAG | `IfcBuilding` per building with an `IfcRoof` and an `IfcWall` (triangulated), `Pset_BuildingCommon` with `BuildingID` and `YearOfConstruction` |

**Coordinates:** the IFC file holds metres relative to the site's base point, so the numbers stay small. The map conversion adds the base point back. Heights are metres NAP. In a drawing with relative coordinates (base point at 0,0), the IFC coordinates equal the drawing coordinates in metres; in an absolute RD drawing they are the RD coordinates minus the base point.

**How the corridor is split:** the triangles of the corridor's design surface are sorted by the offset of their centre from the axis. Up to the pavement edge (the widest asphalt feature line) they are pavement; up to the shoulder edge, shoulder; up to the crest edge (dike), crest; the rest are slopes.

**How the earthworks are built:** for every triangle of the design surface the ground height under its corners is taken from the existing TIN (or AHN when there is none). Where the design lies above the ground, the triangle is clipped to that part: its top is the design, its bottom the ground, and the two meet where the thickness is zero. Along the open border of the corridor (start and end section) walls close the body. The result is a closed body; its volume is computed from its faces and written as a property. Cut is the same with ground and design swapped. In the e2e test the volumes agree with the corridor's own average-end-area volumes within the tolerance stated there.

**GUIDs** are stable: they are derived from the site, the object's kind and name, so exporting the same drawing again gives the same GUIDs and a receiving model can follow the changes.

## Spirals

The as-spoor stream adds `Seg::Spiral` (a clothoid). Each format has one function with an exhaustive match on `Seg`, so the merge shows where spirals go, and each documents what a spiral needs:

- `uitwisseling::seg_to_project`: move the start point, divide length and radii by the drawing unit;
- `landxml::coord_geom_segment`: `<Spiral length radiusStart radiusEnd rot spiType="clothoid">` with Start, PI, End (radii "INF" for a straight end);
- `ifc::horizontal_segment`: `CLOTHOID` with signed `StartRadiusOfCurvature` and `EndRadiusOfCurvature` (0 for straight) and `SegmentLength`; the parent curve is ready (`clothoid_parent`, an `IfcClothoid`), after the ifcopenshell alignment API.

## Validation

`tests/e2e/uitwisseling_check.py` checks both files offline:

- LandXML against the LandXML 1.2 XSD (lxml);
- IFC with ifcopenshell: the file opens, `ifcopenshell.validate` with EXPRESS rules reports no issues, every body gives geometry, the alignments are read back (segment types, lengths from the business logic and from the evaluated curves, start station), and the georeferencing.

```
python3 -m venv target/e2e-uitwisseling/venv
target/e2e-uitwisseling/venv/bin/pip install lxml ifcopenshell pytest
target/e2e-uitwisseling/venv/bin/python tests/e2e/uitwisseling_check.py --landxml file.xml --ifc file.ifc
```

`tests/e2e/uitwisseling_e2e.py` builds a model in Dordrecht in Open CAD Studio (TIN from AHN, survey points, an alignment from PIs with a curve, length and design profile, a corridor) and checks both exports, the LandXML round trip, export of a selection, and a drawing in millimetres.

## Limits

- **No spirals yet:** they come with `Seg::Spiral` (see above).
- **Pavement build-up:** the assemblies have no layer thicknesses, so the pavement and shoulders are surfaces (the top), not solids. The fill body under them is a solid.
- **Ground under the corridor:** taken at the corners of the design triangles; ground relief within one triangle is not followed.
- **Cut needs terrain in the file:** an `IfcEarthworksCut` must void an element. Without an existing TIN in the export (for instance a selection without it) the cut is left out; the fill is still written.
- **No cross sections, superelevation or assembly in LandXML:** the corridor goes out as its surface (with breaklines) and feature lines.
- **No rotation in the map conversion:** Open Infra's site has none (grid north is the drawing's Y axis).
- **3D BAG buildings** go out only when Open CAD Studio shares the whole drawing (up to its view size); they are not in the object index.
- **The reader** (`CIV_LANDXML`) does not read profiles back; the round trip compares points, surfaces and alignments.

## Sources

- LandXML 1.2 schema: <http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd>.
- IFC 4.3 ADD2 (ISO 16739-1:2024), buildingSMART: the alignment, linear placement, georeferencing and road concepts; `IfcTriangulatedIrregularNetwork` flags from the IFC 4.3 documentation (buildingSMART/IFC4.3.x-development).
- The mapping of alignment segments to curve segments follows the ifcopenshell alignment API (IfcOpenShell, LGPL-3.0-or-later).
- No design values: the exports write what the drawing holds. The colours in the IFC file are only for viewing.
