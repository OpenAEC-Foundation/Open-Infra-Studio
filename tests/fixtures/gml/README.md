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
De bestandsroute moet het punt behouden en de overgeslagen lijn tellen.
Samengestelde ringen, meerdere lijnsegmenten, lege `segments` en volledige
4D/5D-tuples worden in vaste varianten van `features.gml` getest.

De toegestane lege `segments`-container en dimensies zonder bovengrens
volgen uit de [GML 3.2.1 schema's](https://schemas.opengis.net/gml/3.2.1/):
`CurveSegmentArrayPropertyType` heeft `minOccurs="0"`; `srsDimension` is een
`positiveInteger`. De lezer projecteert dimensies vanaf twee naar XY.
Niet ondersteunde curveonderdelen worden overgeslagen, zodat geen
gedeeltelijke ring wordt getekend. Tot Ethans besluit verschijnen geen
nieuwe meldingen. `FileData.skipped` telt de overgeslagen objecten en de
interne lijst bewaart hun leesdetails. De importpagina en de opdracht
gebruiken de bestaande teksten van main.

Aanvullende varianten controleren hogere dimensies in losse `pos`-elementen,
witruimte rond de dimensie en alle tekstdelen rond XML-commentaar. Een
getal na commentaar mag niet verdwijnen achter een geldig gelezen begin.

Diepe varianten nesten 65 en 256 `CompositeCurve`- of `MultiGeometry`-
containers rond één lijn. De lezer van main houdt de lijn en het buurpunt
bij 65 containers behouden. Bij 256 containers loopt de oude XML-parser
in de CI over zijn stack. De geometrieverwerking gebruikt een expliciete
werklijst; de werkstack voor de recursieve XML-bibliotheek groeit mee met
de vooraf gemeten XML-diepte. De testgevallen volgen de geneste typen uit
`geometryAggregates.xsd` en `geometryComplexes.xsd`.

`malformed.gml` is een zelfgemaakt bestand met een verkeerde sluittag,
onder dezelfde GPL-3.0-only-licentie. Het legt de bestaande fout uit Sols
reproductie vast, inclusief de verwachte tag en de positie `1:75`. De
algemene bestandsroute moet dezelfde oorspronkelijke XML-fout teruggeven.

`tests/gml_safety.rs` maakt twee vaste diepe varianten met 256 geneste
`MultiGeometry`/`geometryMember`-paren: een verkeerde sluittag en een
onbekende entiteit. De Linux-procesproef begrenst het kindproces op tien
seconden wandtijd, vijf seconden CPU en 1 GiB virtueel geheugen; core dumps
staan uit. Alleen het kindproces krijgt deze grenzen. Een abort, hang,
overschreden grens of ontbrekende XML-foutdetails laat de proef falen. De
normale offline CI voert deze proef uit, zonder live dienst of schemaopvraag.
