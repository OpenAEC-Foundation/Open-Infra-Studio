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

`mixed.gml` is eveneens zelfgemaakt en valt onder dezelfde licentie. Eén
lijn bevat bewust kapotte coördinaattekst; het losse punt blijft geldig.
De bestandsroute moet het punt behouden en de overgeslagen lijn melden.
Samengestelde ringen, meerdere lijnsegmenten, lege `segments` en volledige
4D/5D-tuples worden in vaste varianten van `features.gml` getest.

De toegestane lege `segments`-container en dimensies zonder bovengrens
volgen uit de [GML 3.2.1 schema's](https://schemas.opengis.net/gml/3.2.1/):
`CurveSegmentArrayPropertyType` heeft `minOccurs="0"`; `srsDimension` is een
`positiveInteger`. De lezer projecteert dimensies vanaf twee naar XY.
Niet ondersteunde curveonderdelen worden met een melding overgeslagen,
zodat geen gedeeltelijke ring wordt getekend.
