# Exchange: 3D models, IFC import and IDS checks

Three ways to exchange models with partners, all on the Civiel tab:

- **3D models** go out as glTF binary (GLB), OBJ or STL, and come in from OBJ, STL and glTF/GLB (#71).
- **IFC models** come in: alignments, terrain and elements (#66). The IFC 4.3 export itself is described in [civiel-uitwisseling.md](civiel-uitwisseling.md).
- **IFC models are checked** against a client's IDS (Information Delivery Specification, #65).

The file formats are implemented in the calculation core (`rekenkern`): `mesh3d.rs` (GLB, OBJ, STL), `step.rs` (the ISO 10303-21 reader) and `ids.rs` (IDS 1.0). The commands are in `src/civiel/uitwisseling3d.rs`, `src/ifc/import.rs` with `src/civiel/ifc_import.rs`, and `src/civiel/ids.rs`.

## Commands

| Command | Arguments | Does |
|---|---|---|
| `CIV_3D_EXPORT` | `[FORMAAT=GLB\|OBJ\|STL]` `[SELECTIE]` `[PAD=<path>]` | The drawing's 3D content to a mesh file |
| `CIV_3D_IMPORT` | `<path.obj\|stl\|glb\|gltf>` `[PLAATS=x,y[,z]]` `[SCHAAL=]` `[ROTATIE=<degrees>]` `[LAAG=]` | A mesh file as MESH entities |
| `CIV_IFC_IMPORT` | `<path.ifc>` `[STATUS=B\|N]` | Alignments, terrain and elements from IFC |
| `CIV_IFC_IDS` | `IDS=<path.ids>` `[IFC=<path.ifc>]` | Check an IFC model against an IDS |
| `CIV_IFC` | `… IDS=<path.ids>` | Export IFC 4.3, then check it against the IDS |

All commands take `ID=<name>` for scripts and agents: the outcome is written as JSON to `<config>/api/<name>.json` (see the fields per command below).

**Ribbon:**

- Civiel → Points → **IFC import**, beside the LandXML import.
- Civiel → Output and 3D → **3D model** (import) and **IDS check**, beside CityGML.
- Civiel → Output and 3D → Export → **3D (glTF)**. This writes GLB beside the drawing. Use `FORMAAT=` from a script for OBJ or STL.

**MCP tools:** `civ_3d_export`, `civ_3d_import`, `civ_ifc_import`, `civ_ifc_ids`, and `ifc_export` with `ids`.

```
CIV_3D_EXPORT FORMAAT=GLB
CIV_3D_EXPORT FORMAAT=OBJ SELECTIE PAD=/data/brug.obj
CIV_3D_IMPORT /data/brug.stl PLAATS=105000,425000,1.5 ROTATIE=30 LAAG="KW KUNSTWERK_BRUG"
CIV_IFC_IMPORT /data/partner.ifc STATUS=N ID=ifc
CIV_IFC_IDS IDS=/data/opdrachtgever.ids
CIV_IFC IDS=/data/opdrachtgever.ids
```

## 3D export (`CIV_3D_EXPORT`)

**What goes out:** every triangle in model space, or only in the selection with `SELECTIE`:

| Drawing entity | Exported as |
|---|---|
| MESH (TINs, photo terrain, 3D BAG, BGT in 3D, CityGML, KLIC 3D, CPT columns, corridor bodies, point cloud meshes, imported models) | its faces, fanned into triangles |
| Polyface mesh, polygon mesh (M×N, open or closed) | its faces, as triangles |
| 3D face | one or two triangles |

Not exported, and counted in the result under `overgeslagen`:

- ACIS 3D solids, regions, bodies and surfaces. There is no ACIS kernel to triangulate them.
- Lines, polylines, points, texts, dimensions, hatches, images and block references. The formats carry triangles only.
- Invisible entities.

To get extruded or swept solids into the file, first draw them as meshes (as the 3D commands already do).

**Structure of the file:**

- **Meshes and materials:** one mesh and material per layer and colour, named after the layer. A layer with several colours gets `#rrggbb` after the name.
- **Colours:** the entity colour, or the layer colour when it is ByLayer. glTF stores them linear, OBJ in the MTL `Kd`. Transparency becomes alpha.
- **Photo terrain:** `CIV_FOTOTERREIN` draws the aerial photo as one mesh per palette colour (true colours), not as an image. It therefore leaves as a set of coloured meshes, which looks the same in a viewer. No texture image and no vertex colours are written.

**Coordinates:**

- Metres in the project CRS. A drawing in millimetres or with relative coordinates gives the same file.
- The local origin is the base point of the project location (`GIS → Location`). Without a project location it is the rounded lower corner of the content.
- The origin and the EPSG code travel with the file:
  - glTF: `asset.extras` and `scenes[0].extras` (`{"origin":[x,y,z],"epsg":28992}`);
  - OBJ: `# origin` and `# epsg` comments;
  - STL: the 80-byte header (`origin=x,y,z epsg=n`).
- **Axes per format:**
  - glTF is Y-up, as the specification requires: (x, y, z) is written as (x, z, −y).
  - OBJ is written Y-up as well, with a `# up Y` comment, because that is what most viewers and modellers expect.
  - STL is Z-up, binary, one solid, without colours.

**Where it goes:** without `PAD=`, beside the drawing as `<drawing>_3D.glb` (or `.obj` + `.mtl`, `.stl`). The format follows `FORMAAT=`, else the extension of `PAD=`, else GLB.

**Result** (`ID=`): `bestand`, `formaat`, `driehoeken`, `meshes`, `elementen`, `oorsprong`, `epsg`, `lagen` (name and triangles per mesh), `overgeslagen`.

**USD:** not supported. `FORMAAT=USD` says so. Most USD tools import glTF.

## 3D import (`CIV_3D_IMPORT`)

**Formats:**

- OBJ with its MTL (`mtllib`, read from beside the OBJ);
- STL, binary or ASCII;
- GLB, and glTF with embedded buffers.

**What comes in:**

- Every mesh of the file becomes MESH entities, with at most 100 000 vertices each, so a big mesh is split.
- Degenerate triangles are dropped. The triangle count is otherwise kept, which the unit tests check for each format.
- A material colour other than the readers' default grey becomes the entity's true colour.

**Where it goes:**

- **A file with a georeference:** a file written by `CIV_3D_EXPORT`, or any file with an origin or EPSG code, returns to its own coordinates. A different EPSG code than the project's is transformed point by point.
- **`PLAATS=x,y[,z]`:** the file's local origin goes to this point in project coordinates. The local origin is the stored origin, or (0, 0, 0) of a plain file.
- **A plain file without `PLAATS=`:** its origin goes to the project base point. `PLAATS=0,0,0` takes a plain file's coordinates as project coordinates.
- **`SCHAAL=` and `ROTATIE=`:**
  - `SCHAAL=` scales about the local origin, for example `0.001` for a model in millimetres.
  - `ROTATIE=` turns the model counter-clockwise, in degrees about the vertical through the local origin.

**Layer:**

- `LAAG=` takes a full layer name, NLCS or not, or an NLCS `HOOFDGROEP OBJECT` such as `KW KUNSTWERK_BRUG`.
- The default is `KW KUNSTWERK`, with the status from the settings or `STATUS=`.

**Object data** per mesh:

- `mesh3d` (the mesh's name in the file), `bron`, `driehoeken` and `oppervlak_m2`;
- `volume_m3` for a closed mesh, where every edge is shared by exactly two triangles;
- `gesloten=nee` for an open one, whose volume would be meaningless.

**Result** (`ID=`): `driehoeken`, `meshes`, `oppervlak_m2`, `volume_m3`, `open_meshes`, `laag`, `handles`, and `onderdelen` (per mesh: name, triangles, area, volume).

**Not done (yet):** cutting the model into the design TIN.

## IFC import (`CIV_IFC_IMPORT`)

The STEP file is read by `rekenkern::step` (IFC2X3, IFC4 and IFC4X3 alike). The importer then picks out alignments, terrain and elements. `STATUS=` sets the status of the terrain, and of the axes and elements that have no Open Infra layer of their own. Without it, terrain is B and the rest follows the settings.

### Alignments

Read from `IfcAlignment` → `IfcRelNests` → `IfcAlignmentHorizontal` / `IfcAlignmentVertical` → `IfcAlignmentSegment.DesignParameters` (IFC 4.3).

| IFC | Becomes |
|---|---|
| Horizontal `LINE`, `CIRCULARARC`, `CLOTHOID` | line, arc, clothoid (exact; the clothoids as `spiraal…` object data, as `CIV_AS` stores them) |
| Horizontal `CUBIC`, `HELMERTCURVE`, `BLOSSCURVE`, `COSINECURVE`, `SINECURVE`, `VIENNESEBEND` | a clothoid with the same start and end radius and length (approximated, reported) |
| Vertical `CONSTANTGRADIENT`, `PARABOLICARC` | grades and parabolic vertical curves (exact) |
| Vertical `CIRCULARARC`, `CLOTHOID` | a parabola between the same grades over the same length (approximated, reported) |
| `IfcReferent` with `Pset_Stationing.Station` | the start station (`beginstation`) |
| `IfcAlignmentCant` | not read |

**Drawing the axis.** Each alignment is drawn as `CIV_AS` draws an axis:

- the polyline on `AM-AS` with stationing, tangent points, radii and clothoid parameters;
- registered in the object index, so the length profile, cross sections, corridor and checks work on it straight away;
- a track (`OpenInfra_NLCS` layer `AM-AS_SPOOR_…` in our own export) keeps its layer;
- a name that the drawing already has gets a suffix: `A1 (2)`.

**The design profile** is stored as `CIV_ONTWERPPROFIEL` stores it:

- `pvi` (station:height) and `boog_m`, plus the curve length of every PVI in `bogen_m`, so curves of different lengths survive;
- the tools that read a design profile (corridor, labels, LandXML, IFC export) use `bogen_m` when it is there;
- the PVIs are the tangent intersections of the vertical curves and the kinks between grades;
- it is drawn as the axis in 3D (a 3D polyline at design height on the axis layer), because there is no length profile yet. `CIV_LENGTEPROFIEL` draws one.

### Terrain

A terrain is one of:

- an `IfcGeographicElement` with PredefinedType `TERRAIN`;
- any element with an `IfcTriangulatedIrregularNetwork`;
- an `IfcSite` with a body.

It becomes a TIN on `OG-TRIANGULATIE`, with its outline on `OG-GRENS`, exactly as `CIV_TIN` draws one. It is registered as the existing (B) or design (N) surface:

- status B by default, N with `STATUS=N`;
- a terrain from our own export keeps the status of its layer.

A terrain from our own export on another layer, such as the photo terrain (`OG-DTM`), stays a set of coloured meshes on that layer.

### Elements

**Which elements:** every other product with surfaces, meaning a GlobalId and an `IfcProductDefinitionShape`. It becomes MESH entities, one set per colour.

**Which geometry is read:**

- the `Body` representation (or, without one, any representation that is not an axis, footprint or annotation);
- placed by its `IfcLocalPlacement` chain (`IfcAxis2Placement3D`/`2D`; `IfcLinearPlacement` by its CartesianPosition).

| Representation item | Read |
|---|---|
| `IfcTriangulatedFaceSet`, `IfcTriangulatedIrregularNetwork` (with `PnIndex`) | yes |
| `IfcPolygonalFaceSet` (faces with voids) | yes, triangulated in their plane |
| `IfcFacetedBrep` (and with voids), `IfcShellBasedSurfaceModel`, `IfcFaceBasedSurfaceModel` with `IfcPolyLoop` bounds | yes |
| `IfcMappedItem` (type geometry, `IfcCartesianTransformationOperator3D`, also non-uniform) | yes |
| `IfcExtrudedAreaSolid` with `IfcRectangleProfileDef`, `IfcCircleProfileDef`, `IfcCircleHollowProfileDef`, `IfcArbitraryClosedProfileDef` / `…WithVoids` of `IfcPolyline` or `IfcIndexedPolyCurve` | yes (circles as 24 sides, arc segments through their three points) |
| `IfcSweptDiskSolid` along an `IfcPolyline` / `IfcIndexedPolyCurve` | yes (tube of 12 sides) |
| `IfcBooleanResult`, `IfcBooleanClippingResult` | the first operand only (approximated, reported): openings and clippings are not cut |
| `IfcSectionedSolidHorizontal`, `IfcRevolvedAreaSolid`, `IfcSurfaceCurveSweptAreaSolid`, `IfcAdvancedBrep`, NURBS, CSG primitives, other profiles | not read; counted per class in the result |
| Curves, points and texts (axes, footprints, annotation) | not drawn; such elements count as `zonder_vlakken` |

**Colours** come from `IfcStyledItem` (surface style shading/rendering, also through `IfcPresentationStyleAssignment` in IFC2X3) and per face from `IfcIndexedColourMap`.

**Layer:**

- An element from our own export goes back to its layer (`OpenInfra_NLCS.Laag`).
- Other elements go to the NLCS object of their class:

| IFC class | NLCS object |
|---|---|
| IfcBuilding, IfcRoof, IfcDoor, IfcWindow, IfcStair, IfcCovering, … | OG BEBOUWING |
| IfcPavement, IfcCourse | VH GESLOTENVERHARDING |
| IfcKerb | VH KANTOPSLUITING |
| IfcEarthworksFill / IfcEarthworksCut | GW OPHOGING / GW ONTGRAVINGSVAK |
| IfcEarthworksElement, IfcGeotechnicalStratum, … | GW GROND |
| IfcGeographicElement (VEGETATION) / other | GR BEPLANTING / OG TERREIN |
| IfcPipeSegment, IfcPipeFitting, IfcDistributionChamberElement | RI OVERIG |
| IfcRailing, IfcFence | IE TERREINAFSCHEIDING |
| IfcSign, IfcSignal | IE BORD |
| IfcFurnishingElement | IE MEUBILAIR |
| everything else (walls, slabs, beams, bridges, civil elements, proxies) | KW KUNSTWERK |

**Object data** per element:

- `ifc` (class), `ifc_guid`, `naam`, `ifc_type` (PredefinedType), `ifc_objecttype` and `bron`;
- every single, enumerated or list value and quantity of its property sets as `Set.Property`. The type's sets are included where the occurrence does not set the property itself;
- our own `OpenInfra_ObjectData` returns under its own keys. Keys that would make a mesh pose as an axis, profile, corridor or sheet are left out;
- at most 120 values per entity, each at most 255 characters.

### Georeference

`IfcMapConversion` (or `IfcMapConversionScaled`) with its `IfcProjectedCRS` places everything in the project CRS:

- Eastings, Northings and OrthogonalHeight;
- the rotation from XAxisAbscissa/XAxisOrdinate;
- `Scale`, applied in plan: to coordinates, lengths and stations, not to heights.

Units:

- The project's length and plane angle units are honoured: `IfcSIUnit` prefixes, and conversion-based units such as feet and degrees.
- The map offsets are in the CRS's MapUnit, else in the project's length unit.

Special cases:

- **A CRS other than the project's** (an EPSG code) is transformed point by point. Arcs keep their shape and directions are not corrected for meridian convergence, which is negligible over a project area.
- **No `IfcMapConversion`:** the model's local coordinates are placed from the project base point. The IFC2X3 `IfcSite` latitude and longitude are not used.

### Result and limitations

**Result** (`ID=`):

- `schema` and `georeferentie`;
- `assen` (name, handle, length, PVIs, what was approximated);
- `terreinen` (status, triangles);
- `elementen_ifc`, `driehoeken` and `klassen`;
- `zonder_vlakken`, `niet_gelezen` (items per class) and `benaderd`.

The command line gives the same in one sentence.

**Round trip:** the unit tests export the sample drawing with the IFC 4.3 exporter and import it again. They check that the result has:

- the same axes: lengths, pieces, points and directions along them;
- the same design profile: PVIs and curve lengths;
- the same triangles: 2 in the TIN, 12 in all, with the building's and the photo terrain's colours;

in drawings in metres and in millimetres.

**Limitations:**

- IFC 4.1/4.2 alignments (`IfcAlignment2DHorizontal`), `IfcAlignmentCant` and the cant of rail tracks are not read.
- Openings (`IfcRelVoidsElement`) are not cut out of their host.
- The whole file is read into memory.
- Object data is limited per entity, see above.

## IDS check (`CIV_IFC_IDS`, `CIV_IFC IDS=`)

`rekenkern::ids` reads an IDS 1.0 file (buildingSMART) and checks every applicable element against the requirements:

- **Facets:** entity, attribute, property (with data type and units), classification, material and partOf.
- **Values:** simple values or XSD restrictions (enumeration, pattern, bounds, lengths).
- **Cardinalities:** required, optional, prohibited.
- **Occurrence:** minOccurs/maxOccurs on the applicability.

It passes the official buildingSMART test cases that are part of its unit tests.

**Which IFC is checked:**

- `IFC=` when given.
- Otherwise the drawing's last export, `<drawing>_IFC4X3.ifc`. When that file does not exist yet, the drawing is exported first, as `CIV_IFC` does.
- `CIV_IFC … IDS=<path>` always exports first, with `CIV_IFC`'s own arguments (`BESTAND=`, `SELECTIE`, `TEKST=`), and then checks that file.

**Report** beside the drawing:

- `<drawing>_ids.md`: a table per specification (pass/FAIL, applicable, passed, failed, remarks), then per failing specification the failing elements (instance, GlobalId, name) with the reason per facet.
- `<drawing>_ids.json`: the same, complete, for further processing.

**Command line and result:**

- The command line shows the counts and the names of the failing specifications.
- `ID=` gives `voldoet`, `specificaties`, `specificaties_voldaan`, `specificaties_niet_voldaan`, `controles`, `controles_niet_voldaan`, `per_specificatie`, and the paths of the IDS, the IFC and both reports.

**Cross-check:**

- `cargo test` checks our own export contract (`tests/data/ifc43-export.ids`) with `rekenkern::ids`. It writes the report to `target/ifc-test/voorbeeld_ids.json`.
- In CI, `tests/test_ifc_ids_crosscheck.py` checks the same `voorbeeld.ifc` with IfcTester and compares the verdict and the number of applicable elements per specification. Without IfcOpenShell (locally) it is skipped.

**Limitations** (from `rekenkern::ids`):

- XSD regex `\p{…}` categories and class subtraction are reported as "not checked".
- The IFC2X3 type-mapping table and unit-carrying attributes are not handled.
- BCF topics are not written.
