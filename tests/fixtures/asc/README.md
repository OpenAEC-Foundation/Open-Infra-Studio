# Vaste ASC-testinvoer

`terrain.asc` is een zelfgemaakt raster van drie bij twee cellen met één
ontbrekende hoogte. De waarden zijn verzonnen. Bron: OpenAEC Foundation,
Open Infra Studio, 2026. Licentie: GPL-3.0-only, zoals
[LICENSE](../../../LICENSE).

De tests veranderen gericht deze invoer voor nul, negatieve, gebroken en
zeer grote afmetingen; ongeldige getallen; ontbrekende en extra waarden;
en overflow bij het berekenen van de begrenzing. Ze gebruiken geen diensten.
