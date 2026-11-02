# Vaste GeoPackage/WKB-testinvoer

`point.wkb` en `line.wkb` zijn zelfgemaakte little-endian WKB-bestanden:
één 2D-punt en één lijn met twee punten. De coördinaten zijn verzonnen;
WKB zelf declareert geen CRS. Bron: OpenAEC Foundation, Open Infra Studio,
2026. Licentie: GPL-3.0-only, zoals [LICENSE](../../../LICENSE).

De tests bouwen met deze bytes kleine tijdelijke SQLite/GeoPackage-tabellen
in EPSG:2056. Daardoor wordt ook de echte bestandsroute getest, zonder
een database of dienst buiten de test. Gerichte varianten dekken kapotte
koppen, afgebroken bytes, ongeldige aantallen, niet-eindige coördinaten,
diepe collecties en gedeeltelijk gelezen geometrie. De bestaande grotere
Zürich-fixture blijft daarnaast behouden.
