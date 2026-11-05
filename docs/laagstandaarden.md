# Layer standards

The CAD layer set Open Infra draws with is a setting. NLCS (Dutch) is one layer standard; the U.S. National CAD Standard, a UK and an Australian ISO 13567 profile and a generic ISO 13567 profile come with the plugin, and more can be added as data.

## How it works

- **Semantic key.** Open Infra names everything it draws by NLCS object: main group plus object name, e.g. `RI DWA_RIOOLLEIDING_PVC_250` (a foul sewer pipe, PVC 250). All code already uses these keys. A layer standard maps the keys to its own layer names and pens.
- **One chokepoint.** `nlcs::layer(object, status, discipline, element)` asks the active standard (`laagstandaard::current()`) for the layer. Under NLCS it returns the NLCS layer unchanged. `LayerSpec.object_id` always keeps the NLCS object, and the layer description in the drawing names it.
- **Recognising layers.** Code that reads a drawing looks at object data (XDATA `OPENINFRA`) first. Where it matches on layer names, a name that is not NLCS is taken back to its object through the mapping of the active standard, then of the other standards (`nlcs::parse_layer`, `nlcs::as_nlcs`, `nlcs::LayerMemo`). This covers terrain (OG-TRIANGULATIE by status), KLIC cables (KL), sewers (RI), axes (AM-AS), survey points, quantities, IFC classes and BGT mutations. A drawing made under one standard still works after switching to another.
- **Status.** NLCS statuses B (existing), N (new), T (temporary) and V (to be removed) map to each standard's status field.
- **Elements.** The NLCS element types G (geometry), S (symbol), T (text) and A (hatching) map to each standard's presentation or annotation convention.

### Choosing the standard

- **Workbench:** *Localisation → Layer standard*.
- **Command line:** `GIS_LAAGSTANDAARD` (also `GIS_LAYER_STANDARD`) lists the standards and example layer names. `GIS_LAAGSTANDAARD ncs-us` switches.
- **MCP:** tool `layer_standard` with an optional `id`.
- **Storage:** `layer_standard` in `<config>/OpenCADStudio/open-infra/settings.json`.
- **Default without a setting:** the default region decides. Netherlands → `nlcs`, United States → `ncs-us`, International → `iso13567`. The UK and Australian profiles are chosen explicitly; they are not regions of their own.

New layers follow the chosen standard; layers already in the drawing stay as they are.

### Adding a standard

Put a TSV file in `<config>/OpenCADStudio/open-infra/laagstandaarden/`. It is read at start. A file with the id of a built-in standard replaces that standard. Malformed files are reported on the console and skipped.

```
#id	my-dot
#name	State DOT — example
#region	US
#source	https://…
#status	B=E	N=N	T=T	V=D
#element	G=	S=	T=IDEN	A=PATT
group	name	layer	annotation	color	linetype	lineweight_mm	note
*	*	C-MISC{-status}		7	CONTINUOUS	0.25	catch-all
RI	*	C-SSWR{-status}	C-SSWR-{element}{-status}	3
RI	DWA_RIOOLLEIDING*	C-SSWR-PIPE{-status}	C-SSWR-{element}{-status}	0,255,0		0.35
```

**Header lines:**
- `#id`, `#name`, `#region`, `#source` (repeatable): describe the standard.
- `#status`: the code for B, N, T and V.
- `#element`: the code for G, S, T and A.
- Other lines starting with `#` are comments.

**Columns:**
- **group:** NLCS main group, or `*`.
- **name:** the exact object name, a prefix ending in `*`, or `*` for the whole group.
- **layer:** the layer template. `{status}` and `{element}` insert the codes. `{-status}` adds the hyphen (any single character before the key) only when the code is not empty.
- **annotation:** an optional template for text (T) and hatching (A).
- **color:** an ACI number, or `R,G,B`. RGB is stored as the nearest ACI colour plus a true colour when the two differ.
- **linetype:** `CONTINUOUS`, an NLCS linetype, or one of `assets/laagstandaarden/linetypes.tsv` (DASHED, HIDDEN, CENTER, PHANTOM, DOT, DASHDOT, DIVIDE, BORDER and the ISO 128 types ISO02W100 … ISO10W100).
- **lineweight_mm**, and **note** (ignored).

**Matching rules:**
- The most specific row wins: exact name, then the longest prefix, then the group, then the catch-all `* *`, which is required.
- An empty pen cell takes the next less specific row's value. Without any, it takes the NLCS pen of the object.

## United States — `ncs-us`

### Format

U.S. National CAD Standard (NCS), Layer Guidelines (AIA CAD Layer Guidelines):

| Field | Length | Notes |
|---|---|---|
| Discipline designator | 2 | Second character `-` |
| Major group | 4 | |
| Minor group | 4 | Up to two |
| Status | 1 | Always last |

Fields are separated by `-`.

The NCS layer module gives names only. Colours and line widths are in its separate Plotting Guidelines, which have not been tied to colour numbers since NCS 4.

### Layer list and pens

Layer names, RGB colours, line widths and line styles come from the **A/E/C CAD Standard Release 6.0** (US Army ERDC/ITL TR-12-6, 2015, cleared for public release). It follows the NCS format, lists about 1,900 layers and is the most complete public civil list in NCS form.

The `note` column of `ncs-us.tsv` says for every row whether the layer is verbatim from A/E/C R6, or *derived*: built from the format, with the pen of a named A/E/C layer.

**Derived layers:**
- `C-SSWR-CMBD-PIPE` (combined sewer)
- `C-SSWR-FORC-PIPE` (force main)
- `C-SSWR-VACU-PIPE` (vacuum sewer)
- `E-PRIM-UGND-HVLT` (HV)
- `E-PRIM-UGND-EHVT` (EHV)
- `*-DUCT` (casings)
- `C-NGAS-PIPE-HPRS` (high-pressure gas)
- `C-PVMT-UNIT` (block paving)
- `C-ROAD-SGHT` (sight triangles)
- a few others listed in the file

### Status

A/E/C R6 Table 4-1 status codes:

| Code | Meaning |
|---|---|
| A | Abandoned |
| D | Existing to demolish |
| E | Existing to remain |
| F | Future work |
| M | Items to be moved |
| N | New work |
| T | Temporary work |
| X | Not in contract |
| 1–9 | Phase |

The mapping is B→`E`, N→`N`, T→`T`, V→`D`.

Existing conditions are told apart by the status field, not by the `V-` (survey/mapping) designator. `V-` is used only for survey data such as spot elevations (`V-TOPO-SPOT`) and borings (`V-BORE-…`). A user file can move existing features to `V-` names.

### Text and hatching

- Text goes on `-IDEN` and hatching on `-PATT`. A/E/C has `C-STRM-IDEN`, `C-SSWR-IDEN`, `C-ROAD-CNTR-IDEN`, `C-TOPO-MINR-IDEN` and `C-ROAD-PATT`.
- The annotation layer is the object's own `-IDEN` layer where A/E/C has one, otherwise the major group's.
- Sheet items (`G-ANNO-TTLB`, `G-ANNO-TEXT`, `G-ANNO-DIMS`, …) have no status.

### Linetypes

| A/E/C line style | Linetype |
|---|---|
| Center Line | CENTER |
| Dash | DASHED |
| Dash Short | HIDDEN |
| Property Line | PHANTOM |
| Right of Way | DASHDOT |
| Construction | DASHED |

Styles with embedded text or symbols ("Drain - Storm", "Sanitary", "Water Line", "Gas - Low", "Fence", …) are drawn CONTINUOUS. Their definitions are not public in a CAD-neutral form.

### Limits

- Many State DOTs and cities use their own standards (Caltrans, TxDOT, FDOT, …). Add them as user files.
- The A/E/C pen of an object row also applies to its `-IDEN` text layer.

### Sources

- NCS v6 contents: https://www.nationalcadstandard.org/ncs6/content.php
- A/E/C CAD Standard R6 (ERDC/ITL TR-12-6): https://www.wbdg.org/FFC/AECCAD/ERDCITL_TR12-6_r6.pdf
- NCS 4 plotting change: https://facilitiesnet.com/designconstruction/article.aspx?id=8279
- AIA layer format summary: https://blog.draftsperson.net/the-aia-cad-layer-format

## United Kingdom — `uk-iso13567`

### Format

BS EN ISO 13567 as profiled by BS 1192:2007+A2:2016, in the form of the **AEC (UK) Protocol for Layer Naming v4.1** (2018):

```
Role-Classification-Presentation_Description
```

**Role** (BS 1192 codes, as used here):

| Code | Role |
|---|---|
| C | Civil |
| D | Drainage and highways |
| E | Electrical |
| G | GIS and land surveying |
| L | Landscape |
| M | Mechanical |
| S | Structural |
| Z | General |

**Classification:** Uniclass 2015, checked on uniclass.thenbs.com (v1.43, July 2026), and the AEC (UK) `Zz` table for CAD-only items:

| Code | Title | Source |
|---|---|---|
| Ss_50_35_08_30 | Foul wastewater below-ground drainage pipeline systems | Uniclass 2015 |
| Ss_50_35_08_85 | Surface water below-ground drainage pipeline systems | Uniclass 2015 |
| Ss_50_35_08_15 | Combined below-ground drainage pipeline systems | Uniclass 2015 |
| Ss_50_35_06 | Below-ground inspection systems | Uniclass 2015 |
| Ss_50_35_10 | Below-ground pumped drainage systems | Uniclass 2015 |
| Pr_65_52_24 | Drainage gullies and channels | Uniclass 2015 |
| Ss_70_30_45_45 | Low-voltage distribution systems (LV) | Uniclass 2015 |
| Ss_70_30_50_55 | Medium-voltage distribution systems (MV) | Uniclass 2015 |
| Ss_70_30_35_35 | High-voltage distribution systems (HV) | Uniclass 2015 |
| Ss_75_10_21_88 | Telecommunications systems | Uniclass 2015 |
| Ss_55_20_33_56 | Natural gas distribution network systems | Uniclass 2015 |
| Ss_55_70_95_50 | Treated water pressure pipeline systems | Uniclass 2015 |
| Ss_60_40_84_22 | District heating – primary distribution network systems | Uniclass 2015 |
| Ss_30_14_05 | Asphalt road and paving systems | Uniclass 2015 |
| Ss_30_60_45 | Kerb and traffic separation systems | Uniclass 2015 |
| Ss_40_10_90 | Traffic signage and marking systems | Uniclass 2015 |
| Ss_25_16_94 | Vehicle restraint systems | Uniclass 2015 |
| Ss_20_50 | Bridge structure systems | Uniclass 2015 |
| Ss_20_60 | Retaining wall systems | Uniclass 2015 |
| Ss_45_35_30 | Forestry, biomass, hedging and roadside planting systems | Uniclass 2015 |
| Ss_45_35_45 | Lawn and meadow planting systems | Uniclass 2015 |
| Zz_35_20 | Centrelines | AEC (UK) |
| Zz_40_15_50 / Zz_40_15_55 | Major / minor contours | AEC (UK) |
| Zz_40_20 | Cuttings and embankment indicators | AEC (UK) |
| Zz_40_50 | Level indicators | AEC (UK) |
| Zz_50_60 | Parcels | AEC (UK) |
| Zz_10_20 | Drawing frame | AEC (UK) |
| Zz_20_10_90 | Text | AEC (UK) |
| Zz_20_20 | Dimensions | AEC (UK) |

Where Uniclass has no class, the parent class and a unique description are used, as AEC (UK) advises. Examples: ITS cables on `Ss_75`, oil and chemical pipelines on `Ss_55`, ground investigation on `Ss_15_10`. The terrain TIN is on `Zz_40` (Topography).

**Presentation:**

| Code | Meaning | Used for |
|---|---|---|
| M | Model | NLCS G and S |
| T | Text | NLCS T |
| H | Hatching | NLCS A |
| D | Dimensions | |
| P | Paper | Sheet items |

AEC (UK) also defines `X` (existing). It is not used here, because it would put existing text and geometry on one layer.

**Description:** CamelCase, singular.

### Status

BS 1192 has no status field. The status goes in front of the description, in the style of the AEC (UK) Zz entries (`ExistingOutline`, `ExistingToBeDemolishedOutline`):

| NLCS status | Description prefix |
|---|---|
| B | `Existing…` |
| N | none |
| T | `Temporary…` |
| V | `ExistingToBeRemoved…` |

Example: `D-Ss_50_35_08_30-M_ExistingFoulSewer`.

### Colours and linetypes

BS 1192 and AEC (UK) define no colours. Empty colour and lineweight cells keep the NLCS pen of the object, an established infrastructure pen set.

Linetypes are ISO 128 dash patterns:

| Use | Linetype |
|---|---|
| Centrelines and axes | ISO04W100 |
| Construction lines | ISO02W100 |
| Boundaries | ISO05W100 |
| Everything else | CONTINUOUS |

### Limits

- BS 1192 was withdrawn in 2018 for BS EN ISO 19650. The 19650 UK National Annex covers container naming, not layer naming, so the BS 1192 / AEC (UK) layer profile is still the documented UK practice.
- National Highways' DMRB GG 184 uses another profile: ISO 13567 fields with Uniclass codes stripped of underscores and two-letter roles, e.g. `CH-EF3060-P-Pavements`. It can be added as a user file.
- Uniclass codes change between versions. AEC (UK) 2018 still cites `Ss_50_30_08_30`, now `Ss_50_35_08_30`.

### Sources

- AEC (UK) Protocol for Layer Naming v4.1: https://aecuk.wordpress.com/wp-content/uploads/2018/06/aecukprotocolforlayernaming-v4-1.pdf
- BIMuk BS 1192 layer naming convention: https://bimuk.co.uk/wp-content/uploads/2021/04/BIMuk-Layer-Naming-Convention.pdf
- Uniclass 2015: https://uniclass.thenbs.com/ (e.g. https://uniclass.thenbs.com/taxon/ss_50_35_08)
- DMRB GG 184: https://www.standardsforhighways.co.uk/tses/attachments/f184cf11-a54c-4f82-9a79-e5ff4332daf0

## Australia — `au-iso13567`

### Format

AS 13567.2-1999 is identical to ISO 13567-2. NATSPEC's BIM Reference Schedule lists ISO 13567 for CAD layer naming, AS 1100 for 2D drawing, and the AEC (UK) standard and US NCS as references. The NATSPEC National BIM Guide leaves layer naming to the project. Uniclass 2015 is the classification Australian governments recommend: Victoria's VDAS and Transport for NSW (per NBS).

Transport for NSW's digital engineering standard uses `[Discipline]+[Classification]+[Presentation]+[Description]` with Uniclass and Zz codes, which is the AEC (UK) field order. Its layer table (DMS-FT-535) could not be retrieved.

The Australian default is therefore **the same profile as the UK**: AEC (UK) v4.1 with Uniclass 2015. It is kept as its own file so it can diverge.

### Limits

- State road authorities have their own conventions. Queensland TMR uses its own non-ISO format, e.g. `MRS_BP` for a property boundary. No public layer list was found for Main Roads WA (67-08-43) or VicRoads/DTP. Add these as user files.
- Victoria's SPEAR SCFF layers are for cadastral plans only.

### Sources

- AS 13567.2-1999: https://shop.standards.ie/en-ie/standards/as-13567-2-1999-127916_saig_as_as_270349
- NATSPEC BIM Reference Schedule: https://www.bim.natspec.org/images/NATSPEC_Documents/NATSPEC_BIM_Reference_Schedule_v1.0_Sep_2011.pdf
- NATSPEC National BIM Guide: https://ipweaq.intersearch.com.au/ipweaqjspui/bitstream/1/2373/1/NATSPEC_National_BIM_Guide.pdf
- Uniclass 2015 in Australia (NBS): https://thenbs.com/knowledge/the-adoption-of-uniclass-2015-in-australia
- TfNSW application of Uniclass: https://www.transport.nsw.gov.au/system/files/media/documents/2023/TfNSW-Application-of-Uniclass-v3.1.pdf
- TMR drafting standard, Appendix 2E: https://tmr.qld.gov.au/_/media/a5d2250817d54beab29948935e7da1e2.pdf

## International — `iso13567`

### Format

ISO 13567-2 positional fields:

| Positions | Field | Length | Required? |
|---|---|---|---|
| 1–2 | Agent responsible | 2 | Mandatory |
| 3–8 | Element | 6 | Mandatory |
| 9–10 | Presentation | 2 | Mandatory |
| 11 | Status | 1 | Optional |
| 12–15 | Sector | 4 | Optional |
| 16 | Phase | 1 | Optional |
| 17 | Projection | 1 | Optional |
| 18 | Scale | 1 | Optional |
| 19–20 | Work package | 2 | Optional |
| 21+ | User defined | any | Optional |

A `-` fills a field that has no further subdivision. Open Infra fills the unused optional fields with `-` (as in the Wikipedia example `T-811---E-N----30F--DESCRIPTION_OF_LAYER`) and writes a CamelCase description from position 21.

- **Agent:** the role letter plus `-`.
- **Element:** the Uniclass 2015 code without underscores, first six characters, `-`-filled. This is the way DMRB GG 184 fits Uniclass into the field, e.g. `Ss5035`.
- **Presentation:**

| Code | Meaning |
|---|---|
| `E-` | Element graphics (NLCS G and S) |
| `T-` | Text |
| `H-` | Hatching |
| `D-` | Dimensions |
| `B-` | Border, sheet |
| `U-` | Construction lines |
| `G-` | Grid |

### Status

ISO 13567-2 status codes:

| Code | Meaning |
|---|---|
| N | New |
| E | Existing to remain |
| R | To be removed |
| T | Temporary |
| O / F | To be moved: original / final position |
| `-` | Whole |

The mapping is B→`E`, N→`N`, T→`T`, V→`R`. Sheet and text items use `-`.

### Colours and linetypes

ISO 13567 defines no colours, apart from an optional pen-number suffix. Pens are chosen as for the UK profile.

### Limits

- Six characters keep only the first levels of a Uniclass code. Foul sewer, surface-water sewer and manholes all have element `Ss5035`; the description tells them apart.
- The ISO text is paywalled. The field table follows the Wikipedia summary of ISO 13567-2, clauses 5–7.

### Sources

- ISO 13567-1:2017: https://www.iso.org/standard/70182.html
- ISO 13567 summary: https://en.wikipedia.org/wiki/ISO_13567
- DMRB GG 184: https://www.standardsforhighways.co.uk/tses/attachments/f184cf11-a54c-4f82-9a79-e5ff4332daf0

## Example layer names

Golden names checked by the test `laagstandaard::tests::golden_layer_names`:

| Object (NLCS key, status, element) | NLCS | ncs-us | uk-iso13567 / au-iso13567 | iso13567 |
|---|---|---|---|---|
| Foul sewer pipe (RI DWA_RIOOLLEIDING_PVC_250, N, G) | N-XX-RI-DWA_RIOOLLEIDING_PVC_250-G | C-SSWR-MAIN-PIPE-N | D-Ss_50_35_08_30-M_FoulSewer | D-Ss5035E-N---------FoulSewer |
| Manhole (RI DWA_RIOOLPUT_INSPECTIEPUT, N, S) | N-XX-RI-DWA_RIOOLPUT_INSPECTIEPUT-S | C-SSWR-JBOX-N | D-Ss_50_35_06-M_FoulManhole | D-Ss5035E-N---------FoulManhole |
| Road centre line (AM AS_WEG, N, G) | N-XX-AM-AS_WEG-G | C-ROAD-CNTR-N | D-Zz_35_20-M_RoadCentreline | D-Zz3520E-N---------RoadCentreline |
| Contour (OG HOOGTELIJN, B, G) | B-XX-OG-HOOGTELIJN-G | C-TOPO-MINR-E | G-Zz_40_15-M_ExistingContour | G-Zz4015E-E---------Contour |
| KLIC LV cable (KL ET_LS, B, G) | B-XX-KL-ET_LS-G | E-SECD-UGND-E | E-Ss_70_30_45_45-M_ExistingLVCable | E-Ss7030E-E---------LVCable |
| Survey point (OG HOOGTEPUNT, B, S) | B-XX-OG-HOOGTEPUNT-S | V-TOPO-SPOT-E | G-Zz_40_50-M_ExistingLevelIndicator | G-Zz4050E-E---------LevelIndicator |

## What stays NLCS-specific

- **The semantic keys** are NLCS objects. They are Open Infra's internal vocabulary, not output.
- **`GIS_NLCS_CONTROLE` / `GIS_NLCS_HERSTEL` and the Check page** check NLCS layer names, pens and text classes. Under another standard they say so and do not run.
- **NLCS text classes.** Pens by text height are an NLCS convention. Under another standard, text keeps its layer's pen.
- **NLCS++** (`GIS_NLCSPP_*`) is the Dutch utility-delivery format and reads `*-OI-KL-*` layers and NLCS++ objects. It stays Dutch.
- **Source mappings.** The BGT → NLCS and KLIC (IMKL) → NLCS mappings still pick the semantic object. What is drawn then follows the active standard.
- **BGT mutations** (`GIS_BGT_MUTATIE`) are Dutch by nature. They recognise designs drawn in any standard, but the NLCS statuses R and X exist only in NLCS names.
- **The sheet legend** shows the NLCS object name in Dutch (`humanize`).

## Limits of the mappings

- **Many-to-one.** A standard's layer usually holds several NLCS objects; every PVC/concrete diameter of a foul sewer is on `C-SSWR-MAIN-PIPE`. Reading such a layer back gives a representative object of the right kind and main group; the exact object is in the object data. The tests require that a layer with a cable, pipe, sewer or axis object holds no object of another main group.
- **Coverage.** Every one of the 3,659 NLCS objects maps to a row of its main group or a more specific one; none falls through to the catch-all (test `no_object_falls_through_to_the_catch_all`). The objects the code draws have rows of their own (test `objects_the_code_draws_have_their_own_row`). Group rows are a coarse fit for constructions (BC, SC, MC, …), environmental research (MO) and traffic installations (IW).
- **User files are read at start.** A changed file needs a restart. Switching between standards does not.
