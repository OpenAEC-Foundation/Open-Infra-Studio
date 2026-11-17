# Example: Dordrecht, Grote Kerk — situatie bestaand conform NLCS

![Example sheet](dordrecht-grote-kerk-nlcs.png)

An existing-situation drawing of the area around the Grote Kerk in Dordrecht, made entirely with Open Infra in Open CAD Studio v2026.42. It is an A1 sheet at 1:500 in RD (EPSG:28992), with the drawing units in metres.

| File | What it is |
|---|---|
| `dordrecht-grote-kerk-nlcs.dwg` | The drawing: 3,439 entities on 68 NLCS layers. Every BGT, BRK and KLIC object carries its source attributes as XDATA `OPENINFRA`. |
| `dordrecht-grote-kerk-nlcs.pdf` | Plot on A1 landscape, with NLCS lineweights. |
| `dordrecht-grote-kerk-nlcs.png` | Preview of the plot. |

## Contents

- **BGT** (current objects from the PDOK OGC API). Roads and paving by material, green areas, water, buildings, civil structures, walls and fences. Street names and house numbers are placed at their BGT rotation. Status B, discipline XX.
- **Cadastral map** (BRK). Parcel boundaries on `B-XX-KG-GRENS_PERCEEL-G` and parcel numbers at their BRK rotation.
- **KLIC delivery** 21O072584 (Lange Geldersekade, sample from GIS2BIM), drawn with discipline OI:
  - Low and medium voltage, data, gas low pressure, water and sewer, each on its own NLCS KL/RI layer with the NLCS colour and linetype.
  - Cable and pipe labels such as `G-LD Stedin 160 mm PVC`.
  - Fittings as point symbols.
- **Drawing sheet**:
  - Frame and title block.
  - Legend of the layers used, grouped by NLCS main group.
  - North arrow, scale bar and RD coordinate crosses every 50 m.

## How it was made

In an empty metric drawing, with the Open Infra commands (the same as the ribbon buttons under *Basisdata NL (NLCS)*):

```
GIS_SITE RD 104885 425320 300 250 Dordrecht, Grote Kerk
GIS_Q_BGT
GIS_Q_BRK
GIS_KLIC_IMPORT <path>/Levering_21O072584_1.zip
GIS_SHEET Situatie bestaand - BGT, kadaster en kabels en leidingen
```

The KLIC sample comes from [GIS2BIM](https://github.com/DutchSailor/GIS2BIM), at `supportfiles/Sandbox/KLIC/`.

Data:
- BGT: PDOK, CC0.
- Kadastrale kaart: Kadaster, CC BY 4.0.
- KLIC: Kadaster.
- NLCS 5.02: DigiGO / Stichting NLCS, CC BY 4.0.
