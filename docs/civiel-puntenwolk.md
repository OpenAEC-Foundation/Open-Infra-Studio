# Point clouds: TIN, profiles, ground filter and 3D meshes

Point clouds used to be shown only (`GIS_PUNTENWOLK`, an attached scan). The Civiel tab now builds with them:

- a **TIN from a point cloud**: the AHN point cloud of the project area (or a selected area), or a file;
- **length profiles and cross sections** whose ground line comes from a strip of points along the axis;
- a **ground filter** for scans without classification;
- **more formats**: E57, PLY, PTS and XYZ, besides LAS/LAZ/COPC;
- a **3D mesh** of an object (a bridge, quay wall or tunnel) inside a closed polyline or a box.

Everything goes through the existing paths: the TIN lands on `OG-TRIANGULATIE` like every terrain, so contours, elevation analysis, earthwork and profiles work on it unchanged.

## Ribbon

Civiel → **Puntenwolk** (Point cloud):

| Button | Command | What it does |
|---|---|---|
| **TIN uit AHN-punten** (only with the Dutch modules on) | `CIV_PUNTENWOLK_TIN` | A TIN from the AHN5 ground points of the project area, or of the selected closed polyline. |
| **TIN uit puntenwolk** | `CIV_PUNTENWOLK_BESTAND <file>` | A file dialog; a TIN from the ground of the file. |
| **3D-mesh** | `CIV_MESH` | A closed 3D mesh of the points inside the selected closed polyline. |

## TIN from a point cloud

```
CIV_TIN BRON=PUNTENWOLK [BESTAND=<las/laz/e57/ply/pts/xyz>] [EDITIE=AHN5|AHN4]
        [GEBIED=x,y;x,y | x,y;x,y;x,y;…] [KLASSEN=2;9 | AUTO] [CEL=0.5] [TOL=0.10]
        [MAX=200000] [GAT=2] [HELLING=0.3] [VENSTER=18] [STATUS=B|N] [ID=…]
```

`CIV_TIN BESTAND=<file>` implies `BRON=PUNTENWOLK`.

| Argument | Default | Meaning |
|---|---|---|
| `BESTAND` | (AHN) | A point cloud file. Without it, the AHN COPC tiles of Het Waterschapshuis. |
| `EDITIE` | AHN5 | AHN edition; a tile that AHN5 lacks comes from AHN4. |
| `GEBIED` | selection, else the project area (AHN) or the whole file | Two corners or a polygon in project coordinates. A selected closed polyline (or circle) does the same. |
| `KLASSEN` | `2;9` | ASPRS classes taken as ground: 2 ground and 9 water, as AHN classifies them. `AUTO` runs the ground filter whatever the classes say. |
| `CEL` | 0.5 m | Thinning cell: one point per cell. |
| `TOL` | 0.10 m | Vertical tolerance of the TIN: every kept ground point lies within it. |
| `MAX` | 200,000 | Most triangles. |
| `GAT` | 2 m | A triangle with a part farther than this from any ground point is left out, so buildings and other gaps stay holes (at least three cells). `GAT=0` keeps every triangle. |
| `HELLING`, `VENSTER` | 0.3, 18 m | Ground filter: slope still ground, largest window (see below). |

### How it works

1. **Streaming.** AHN is read as COPC over HTTP range requests, tile by tile (1 km tiles), and only the octree nodes inside the area's box are downloaded, at a level of detail of half the cell. A local COPC file is read the same way. Other files are read in one pass. No step holds the whole cloud: points go straight into the cell grid.
2. **Classes.** Noise (ASPRS 7 and 18) is never used. Points of the wanted classes compete for their cell.
3. **Thinning per cell.** Each cell keeps the measured point nearest its centre. It also keeps the lowest point of any class, for the ground filter. When the grid would hold more than 2 million cells, its cell doubles (four cells become one). The area also sets the start cell, so a city fits in memory: for 25 km² the cell becomes about 3.5 m.
4. **Ground filter when needed.** A cloud without class 2 points (or `KLASSEN=AUTO`) goes through SMRF (below) on the lowest points.
5. **Thinning by error.** Greedy insertion after Garland & Heckbert (1995): start from the convex hull and an 8 × 8 sample, triangulate (Delaunay), and insert in every triangle the point that lies farthest from it vertically. This repeats until every point lies within `TOL`, or `MAX` is reached (the largest remaining error is then reported). Flat ground keeps few points; curbs, ditches and embankments keep many.
6. **Output.** The TIN is drawn like any terrain: one or more MESH entities on `B-…-OG-TRIANGULATIE-G` (or `N-` with `STATUS=N`), the outline on `OG-GRENS`, object data and the object index. Contours (`CIV_HOOGTELIJNEN`), elevation analysis (`CIV_ANALYSE`), earthwork (`CIV_VOLUME`) and profiles read it like any other TIN.

The answer (`ID=`) holds `driehoeken`, `punten`, `laag`, `hoogte_min`, `hoogte_max`, `bron`, `crs`, `punten_gelezen`, `punten_grond`, `grondfilter`, `klassen`, `cel_m`, `tol_m`, `max_fout_m`, `begrensd`, and for AHN `tegels` and `opgehaald_mb`.

## Ground filter (SMRF)

For scans without classification: the Simple Morphological Filter of Pingel, Clarke & McBride (2013).

1. **Minimum surface.** The lowest point per cell of 1 m, or the thinning cell if that is larger. A cell more than 1 m below all its neighbours is a low outlier and is left out. Empty cells are filled from their neighbours.
2. **Progressive opening.** The surface is opened (erosion, then dilation) with windows of radius 1, 2, … up to `VENSTER`. A cell that the opening lowers by more than `HELLING × radius` stands on an object and drops out.
3. **Provisional terrain model.** The ground cells, filled again. A point is ground when it lies within `0.5 m + 1.25 × slope` of that model.
4. **Spikes.** A ground point more than 0.4 m above the median of the ground points within about a metre around it is dropped (a point low on a wall, a car roof).

The windows are squares, as in the progressive morphological filter of Zhang et al. (2003); SMRF uses discs. The openings take constant time per cell whatever the window (van Herk 1992; Gil & Werman 1993).

| Value | Used | Source |
|---|---|---|
| Cell | 1 m | Pingel et al. (2013); PDAL `filters.smrf` default |
| Window | 18 m | idem |
| Elevation threshold | 0.5 m | idem |
| Scaling factor | 1.25 | idem |
| Slope | **0.3** (SMRF: 0.15) | Open Infra's choice, so dikes and embankments of about 1:3 stay ground. *Estimate*, tuned on the synthetic test scene. |
| Low outlier | 1 m below every neighbour | *Estimate* |
| Spike | 0.4 m above the neighbours' median | *Estimate* |

Limits:
- Buildings wider than about twice `VENSTER` (large halls) are not removed: raise `VENSTER`.
- Ground steeper than `HELLING` (steep embankments, quay edges) can be taken for an object: raise `HELLING`.
- Points low on a wall, up to the spike limit, can stay.

The cloth simulation filter (CSF, Zhang et al. 2016) was the alternative; SMRF was chosen because it works on the same grid as the thinning and needs no physics simulation.

Unit tests:
- a synthetic scan of 120 × 100 m with a dike (1:4), three buildings, twenty trees and two low outliers: more than 95 % of the ground found and more than 99 % of what is found is ground;
- the unclassified test file of `tests/data/puntenwolk`.

## Profiles from points

```
CIV_LENGTEPROFIEL AS=<handle> BRON=PUNTENWOLK [STROOK=1] [BESTAND=…] [KLASSEN=2;9|AUTO] [EDITIE=…]
CIV_DWARSPROFIELEN AS=<handle> BRON=PUNTENWOLK [STROOK=1] …
```

The ground line comes from the ground points themselves, not from a TIN or the AHN grid. Each station takes the **median height of the ground points within a strip of `STROOK` m** (a disc of that diameter around the station). Where the strip holds no ground points (buildings, water without points) the line breaks, as it does with AHN.

It is a height source next to the TIN and AHN (`Maaiveld::Puntenwolk` in `src/civiel/mod.rs`), so the profile commands use it unchanged. AHN and COPC files are read in blocks of 100 m as the stations ask for them. Other files are read once.

## Formats

`GIS_BESTAND` (the file button of the GIS tab) and `GIS_RASTER_IMPORT` now read:

| Format | Reader | CRS |
|---|---|---|
| LAS, LAZ, COPC | `las`, `copc-rs` | GeoKeys/WKT in the file (`las-crs`) |
| E57 | Open Pointcloud Studio (`pointcloud-core`) over the `e57` crate | the file's coordinate metadata (EPSG code or WKT) |
| PLY (ASCII, binary little endian) | `pointcloud-core` | — |
| PTS, XYZ, TXT with bare coordinates | `pointcloud-core` | — |

- **CRS:** without one in the file, a `.prj` beside it is used, else the project's CRS.
- **TXT:** a `.txt` counts as a point cloud only when its first line holds bare numbers (`x y z [i] [r g b]`). A CSV with a header row stays vector data.
- **Import:** the points are written as an indexed scan (`.rcs`) beside the drawing and attached, thinned to at most 30 million.
- **Colours:** the file's RGB, else its classes, else intensity, else height.
- **TIN source:** every format also works as a TIN source (`BESTAND=`).

Crates and licences:
- `e57` (MIT);
- `pointcloud-core` from OpenAEC's [Open Pointcloud Studio](https://github.com/OpenAEC-Foundation/open-pointcloud-studio) (LGPL-3.0-or-later, compatible with GPL-3.0), pinned to commit `b273590`, without its network feature;
- `copc-rs` (MIT/Apache-2.0), taken from git at commit `1cff2a6`. The released 0.5 does not build against `las` 0.9.11 and `laz` 0.12, which `pointcloud-core` needs; upstream has fixed this but not released it yet.

## 3D mesh

```
CIV_MESH [GEBIED=x,y;x,y | polygon] [BESTAND=…] [EDITIE=…] [ZMIN=] [ZMAX=] [KLASSEN=6;26|ALLE]
         [VOXEL=] [GAT=1] [MAX=400000] [SOORT=kunstwerk|brug|viaduct|tunnel|sluis|steiger|pijler|landhoofd|kademuur]
         [ZIJDE=BOVEN|MIDDEN|x,y,z] [NAAM=]
```

A mesh of the points inside the selected closed polyline (or `GEBIED`), optionally between `ZMIN` and `ZMAX`. It is 3D, not 2.5D: decks, undersides, walls and piers.

- **Mesher.** The closed mesher of Open Pointcloud Studio (`pointcloud-core::mesh_closed`), used as a library.
  - The points become oriented surface elements.
  - A signed distance to the surface is found at the corners of a voxel lattice.
  - Dual contouring takes the surface out (after Ju et al. 2002).
  - The mesh is closed wherever the scan has data or a gap narrower than `GAT` (at most 3.2 m and 32 voxels).
- **Voxel.** By default 2.5 × the point spacing, between 0.02 and 0.5 m (the mesher allows 0.005–0.5 m). AHN gives 0.5 m. When the mesh would exceed `MAX` triangles, the voxel grows (× 1.6) up to 0.5 m.
- **Side.** Which way a surface faces where no scanner position is known:
  - `BOVEN` for data measured from above (the default for AHN);
  - `MIDDEN` faces the middle of the area, right for a tunnel or room seen from inside (the default for files);
  - `x,y,z` faces a point, the scanner side.
- **Output.**
  - MESH entities of at most 100,000 vertices, sent in batches under the host's 64 MB message limit.
  - The layer is the NLCS object of `SOORT` (KW-KUNSTWERK by default, KW-KUNSTWERK_BRUG for `SOORT=brug`, …) at the drawing status.
  - Object data: `mesh`, `bron`, `punten`, `driehoeken`, `voxel_m`, `gesloten`, `afwijking_p95_m`.
  - The answer adds the number of open edges, pieces, mean and 95 % deviation of the points from the mesh, and the handles.
- **Limits.** At most 600 × 600 m and 12 million points; more are thinned evenly. The mesher runs on two threads.

AHN measures from the air: a bridge from AHN gives its deck, parapets and some of the piers, not its underside. A terrestrial scan (E57) gives the whole object.

## Tests

- **Unit tests:**
  - cell thinning and coarsening;
  - greedy insertion: a plane, the tolerance, the triangle cap, a gap as a hole;
  - SMRF and the spike filter;
  - LAS, PLY, PTS, XYZ and TXT readers;
  - the unclassified test file end to end (ground filter, TIN within the tolerance, the building a hole);
  - a closed mesh of a scanned box;
  - argument parsing.
- **Test data:** `tests/data/puntenwolk/maak_testwolk.py` (standard library only) makes `onbewerkt.las`, an unclassified scan of 80 × 60 m (0.6 MB), and `brug.ply`, a footbridge (0.5 MB).
- **End to end:** `tests/e2e/puntenwolk_e2e.py` (port 18811, isolated `XDG_CONFIG_HOME`).
  - Dordrecht: TIN from AHN, contours, analysis, earthwork, a length profile over the TIN and from points, cross sections from points, the mesh of a footbridge.
  - The synthetic site: TIN from the unclassified LAS through the ground filter, contours, PLY import, the mesh of the PLY bridge.
  - Captures in `target/e2e-puntenwolk/`.

## Sources

- ASPRS (2019). *LAS Specification 1.4 – R15*: point classes 2 ground, 6 building, 7 low noise, 9 water, 18 high noise.
- AHN (Actueel Hoogtebestand Nederland), point cloud classification (ahn.nl): 1 other, 2 ground, 6 buildings, 9 water, 26 civil structures.
- AHN point clouds as COPC: Het Waterschapshuis, object storage `hwh-ahn` (CC0). COPC 1.0 specification, copc.io.
- Garland, M. & Heckbert, P. S. (1995). *Fast Polygonal Approximation of Terrains and Height Fields.* Technical report CMU-CS-95-181, Carnegie Mellon University.
- Pingel, T. J., Clarke, K. C. & McBride, W. A. (2013). An improved simple morphological filter for the terrain classification of airborne LIDAR data. *ISPRS Journal of Photogrammetry and Remote Sensing* 77, 21–30.
- PDAL, `filters.smrf` documentation (pdal.io): the default parameters.
- Zhang, K., Chen, S.-C., Whitman, D., Shyu, M.-L., Yan, J. & Zhang, C. (2003). A progressive morphological filter for removing nonground measurements from airborne LIDAR data. *IEEE Transactions on Geoscience and Remote Sensing* 41(4), 872–882.
- Zhang, W., Qi, J., Wan, P., Wang, H., Xie, D., Wang, X. & Yan, G. (2016). An easy-to-use airborne LiDAR data filtering method based on cloth simulation. *Remote Sensing* 8(6), 501.
- van Herk, M. (1992). A fast algorithm for local minimum and maximum filters on rectangular and octagonal kernels. *Pattern Recognition Letters* 13(7), 517–521.
- Gil, J. & Werman, M. (1993). Computing 2-D min, median, and max filters. *IEEE Transactions on Pattern Analysis and Machine Intelligence* 15(5), 504–507.
- Ju, T., Losasso, F., Schaefer, S. & Warren, J. (2002). Dual contouring of Hermite data. *ACM Transactions on Graphics* 21(3), 339–346.
- ASTM E2807-11, *Standard Specification for 3D Imaging Data Exchange, Version 1.0* (E57).
- Turk, G. (1994). *The PLY polygon file format.*

## Open Infra's own choices

None of these is a standard value; they are estimates, set for AHN-like data.

| Value | Default | Reason |
|---|---|---|
| `CEL` | 0.5 m | The cell of the AHN4 DTM on PDOK |
| `TOL` | 0.10 m | About twice AHN's stated stochastic error of 5 cm (ahn.nl, quality description; not re-checked) |
| `MAX` | 200,000 triangles | Keeps the drawing responsive |
| `GAT` | 2 m | Bridges alleys, leaves buildings out |
| `STROOK` | 1 m | About ten AHN ground points per station |
| Mesh voxel | 2.5 × point spacing | Enough points per voxel for a plane fit |
| Mesh `GAT` | 1 m | Closes gaps between scan lines, keeps doorways |
