# Vaste algemene GML-testinvoer

`features.gml` is zelfgemaakt: een lijn met 3D-coördinaten en een punt in
EPSG:2056, zonder optionele attributen. Bron: OpenAEC Foundation, Open Infra
Studio, 2026. Licentie: GPL-3.0-only, zoals [LICENSE](../../../LICENSE).
Alle geometrie is verzonnen. URL's identificeren namespaces en CRS; tests
halen geen schema's of andere diensten op.

De tests lezen het bestand ook via de algemene bestandsroute. Gerichte
varianten dekken overgeërfde dimensies, incomplete punten, tekst tussen
getallen, niet-eindige getallen, lege collecties en afgebroken XML. Voor
GML 2 worden komma-gescheiden coördinaten uit dezelfde waarden gemaakt.
