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

`circular.wkb` is eveneens zelfgemaakt, met dezelfde bron en GPL-3.0-only-
licentie. De 57 bytes coderen een CircularString (type 8) door de verzonnen
punten (0,0), (5,5) en (10,0). Volgens de [GeoPackage 1.4-specificatie,
Annex G](https://www.geopackage.org/spec140/index.html#geometry_types)
is dit een uitbreidingsgeometrie. De lezer tekent die curve niet als rechte
lijn. Hij houdt een niet ondersteunde soort apart van kapotte bytes.
De gewone GeoPackage-kop geldt ook voor deze OGC-uitbreidingssoorten; bit 5
is voor eigen coderingen. Een bestaande kop met dat bit en volledig leesbare
WKB blijft werken; een onbekende eigen codering wordt apart overgeslagen.

De tests maken twee lagen: `a_mix` bevat een lijn, de onderzochte geometrie
en een punt; `b_good` bevat een gezond punt. CircularString, afgebroken WKB
en een kapotte GeometryCollection mogen alleen hun eigen rij laten vervallen.
Beide laagnamen en alle gezonde rijen blijven beschikbaar. Dat geldt ook als
de standaardlaag geen enkele leesbare rij meer bevat. Dit bewaakt de
bestandsroute voordat de gebruiker een laag kiest.

De letterlijke reproducties hebben de volgende namen:

- `sol_gpkg_circular_string_keeps_readable_rows_and_other_layer`;
- `sol_gpkg_truncated_wkb_keeps_readable_rows_and_other_layer`;
- `sol_gpkg_broken_collection_keeps_readable_rows_and_other_layer`;
- `sol_gpkg_65_nested_collections_preserve_the_point_read_by_main`.

De oude vaste grens van 64 is verwijderd. De WKB-lezer telt nog te lezen
geometrieën in de binaire volgorde, zonder recursieve functieaanroepen.
Naast de letterlijke 65-containersproef testen vaste varianten 100 en
5000 containers. Een gedeeltelijke of niet ondersteunde verzameling mag
geen eerder gelezen punt achterlaten. Ook een afgebroken CircularString
wordt van een geldige maar niet ondersteunde curve onderscheiden.

SQL-NULL, lege geometrie en lege CircularString blijven als records met hun
attributen beschikbaar. Intern tellen `warnings` en `skipped` de overgeslagen
rijen. De importpagina en plaatsingsroute tonen volgens het besluit van
7 november de laag, rij en oorzaak bij het resultaat. Goede rijen en
andere lagen blijven beschikbaar; goede invoer houdt de bestaande melding.
De tijdelijke SQLite-tabellen dienen uitsluitend voor deze leesroutes en zijn
geen volledige GeoPackage-conformiteitsbestanden. Tests downloaden geen
schema's en gebruiken geen live diensten.

`partial.sql` is zelfgemaakt onder dezelfde GPL-3.0-only-licentie. De
vaste SQLite-inhoud gebruikt de bestaande lijn en punt, een afgebroken
geometrie op rij 2 van `a_mix` en een gezonde laag `b_good`. De importpagina
moet de goede objecten en beide laagnamen behouden en de kapotte rij met
oorzaak melden. Ook de echte opdrachtimport wordt zonder CAD-installatie
getest; de testhost bewaart de werkelijk aangemaakte tekenobjecten.
