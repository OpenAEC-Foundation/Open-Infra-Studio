# Data

- `locaties.json`: alle plekken met het commando om de projectlocatie te zetten en de diensten (ids uit de
  catalogus van Open Infra) die in beeld waren.
- `dordrecht-showcase.dwg` (24 MB): het 3D-model van de binnenstad van Dordrecht zoals in de video. Het bevat:
  - de BGT in 3D;
  - de 3D BAG met egale gevels;
  - KLIC in 3D;
  - 68 BRO-sonderingen en -boringen;
  - een straatontwerp (as, lengteprofiel, corridor met dwarsprofielen).

  Openen in Open CAD Studio v2026.43 met Open Infra.

## Opnieuw opnemen op een andere pc

1. Installeer Open CAD Studio v2026.43 en Open Infra (main of de release daarna); zie de README van
   `OpenAEC-Foundation/Open-Infra`.
2. Kaartbeelden: zet de locatie (`GIS_SITE …` uit `locaties.json`) en laad de dienst met `GIS_Q_SERVER <id>`, of
   `GIS_Q_NATURA2000`, `GIS_Q_GEOMORFOLOGIE`, `GIS_Q_HISGIS`, `GIS_Q_SERVER JAAR=1900`.
3. 3D: open `dordrecht-showcase.dwg`. Doorzichtig maaiveld: `CIV_DOORZICHT PCT=78`, terug met `CIV_DOORZICHT UIT`.
   Opnieuw opbouwen kan met de commando's in `../opname/showcase7.py`.
4. Schermafbeeldingen op 2× voor bovenaanzichten: `../opname/stills.py` (3 × 3 opnames aan elkaar).
