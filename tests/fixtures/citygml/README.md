# Vaste CityGML-testinvoer

`building.gml` is voor deze repository zelf gemaakt: één verzonnen dakvlak
in EPSG:2056 met expliciete 3D-dimensie. Er zijn geen externe gegevens of
klantgegevens gebruikt. Bron: OpenAEC Foundation, Open Infra Studio, 2026.
Licentie: GPL-3.0-only, zoals [LICENSE](../../../LICENSE).

De tests veranderen deze vaste invoer gericht voor de versies 1/2/3, lege
en kapotte documenten, ontbrekende eindtags, comments/CDATA, XML-aanhalingstekens,
ongeldige dimensies en getallen. De leesroute en objectfiltering worden ook
gecontroleerd. URL's worden uitsluitend als identificatoren gebruikt.

`mixed.gml` is zelfgemaakt uit hetzelfde verzonnen gebouw en valt onder
dezelfde bron en GPL-3.0-only-licentie. Het bevat daarnaast een object met
`BAD` na XML-commentaar en een object met een onbekende entiteit. Die twee
objecten blijven onleesbaar; het eerste gebouw moet via de bestandsroute
volledig binnenkomen. De interne `CityModel.warnings`-lijst bevat één item
per overgeslagen object of onleesbare member. Tot Ethans besluit worden
deze details niet aan de bestaande voortgang of eindtekst toegevoegd.

Aanvullende vaste varianten testen alle tekstdelen rond commentaar en CDATA,
ook bij `pos` en GML 2 `coordinates`. De gekozen LoD gebruikt één strikte
decoder. Een overgeslagen lager LoD wordt niet apart gevalideerd. Voor
`coordinates` gelden de opgegeven `cs`, `ts` en `decimal`; de standaardwaarden
volgen uit [CoordinatesType in basicTypes.xsd](https://schemas.opengis.net/gml/3.2.1/basicTypes.xsd).
Witruimte rond de waarde van `srsDimension` wordt getrimd; niet te lezen optionele dimensiemetadata
houdt de bestaande terugval van main. Complete dimensies vanaf twee worden
naar XYZ geprojecteerd, bij 2D met de bestaande ontbrekende Z-waarde.

De XML-structuur rond de members wordt apart gecontroleerd. Een onbekende
entiteit of tweede XML-declaratie daar is een documentfout; een onbekende
entiteit binnen één member laat de andere members leesbaar. Een verklaarde
entiteit, `&amp;` en numerieke verwijzingen buiten de members blijven geldig.
De XML-regels staan in de [W3C XML-specificatie](https://www.w3.org/TR/REC-xml/).
Een variant met 300 buitenste containers bewaakt dat geen nieuwe vaste
dieptelimiet invoer afwijst die main las. De recursieve XML-bibliotheek krijgt
voor diepe documenten een werkstack op basis van de gemeten XML-diepte.
