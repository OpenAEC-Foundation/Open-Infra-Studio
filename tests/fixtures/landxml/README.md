# Vaste LandXML-testinvoer

`site.xml` is voor deze repository zelf gemaakt: één meetpunt, één TIN-vlak
en één recht assegment. Alle gegevens zijn verzonnen. Bron: OpenAEC
Foundation, Open Infra Studio, 2026. Licentie: GPL-3.0-only, zoals
[LICENSE](../../../LICENSE).

De tests veranderen gericht coördinaten, eenheden, stations en verplichte
puntvelden. Ze lezen geen externe schema's of diensten. De bestaande
inline tests voor bogen, spiralen en voet-eenheden blijven daarnaast bestaan.

De reproducties van Sol staan als afzonderlijke tests in `landxml.rs`:

- `sol_start_coordinate_text_overrides_pntref` vervangt de Start letterlijk
  door `<Start pntRef="1">205 105</Start>` en verwacht één as vanaf
  E=105, N=205.
- `sol_named_cgpoint_coordinate_text_overrides_pntref` voegt een benoemd
  CgPoint toe met zowel `pntRef="1"` als `205 105 7`. Het nieuwe punt krijgt
  de tekstcoördinaten, die afwijken van punt 1.
- `sol_unnamed_cgpoint_coordinate_text_overrides_pntref` controleert hetzelfde
  zonder naam; dit is een eigen punt, geen lege groepsverwijzing.
- `sol_negative_infinite_spiral_radius_keeps_zero_start_curvature` vervangt
  in de bestaande `SAMPLE` letterlijk `INF` door `-INF`. De as blijft
  identiek aan de versie met nul beginkromming.

LandXML 1.2 `PointType` geeft coördinaattekst voorrang wanneer ook `pntRef`
aanwezig is. De documentatie van `Spiral` noemt zowel `INF` als `-INF`.
Die schematekst is ook opgenomen in de [officiële publicatie van het
Japanse onderzoeksinstituut NILIM](https://www.nilim.go.jp/lab/qbg/researchfields/pdf/h30_basedLandXML1.2.v1.2.pdf).
De tests downloaden deze publicatie niet. Er is geen invoer uit overgenomen.

`partial.xml` is eveneens zelfgemaakt en valt onder dezelfde bron en
GPL-3.0-only-licentie. Eén punt is bruikbaar en één punt bevat kapotte tekst.
`tests/landxml_import.rs` voert de echte import uit met een lokale testhost:
één punt met twee kruisarmen, één undo-stap en de volledige bestaande
succesmelding. De waarschuwing blijft intern. Er is geen CAD-installatie of
live dienst nodig en de productielogica wordt niet gewijzigd.
