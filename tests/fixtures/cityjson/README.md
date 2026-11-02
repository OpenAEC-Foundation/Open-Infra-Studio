# Vaste CityJSON-testbestanden

Deze kleine bestanden zijn voor deze repository zelf gemaakt. Ze bevatten
uitsluitend verzonnen geometrie, geen gegevens van personen of projecten.
Bron: OpenAEC Foundation, Open Infra Studio, 2026. Licentie: GPL-3.0-only,
zoals het [repositorybestand LICENSE](../../../LICENSE).

- `building.city.json`: één dakvlak met vier hoekpunten in EPSG:2056,
  met schaal en verschuiving; optionele attributen en semantiek ontbreken.
- `building.city.jsonl`: dezelfde geometrie als CityJSONSeq, met een kopregel
  en één feature. De transformatie staat alleen in de kopregel.

De unit-tests veranderen deze vaste invoer ook gericht: kapotte JSON-regels,
ontbrekende velden, ongeldige transformaties en overflow. Geen test vraagt
gegevens op bij een dienst. De namespace- en CRS-URL's zijn identificatoren.
