---
name: civiel-riolering-en-nuts
description: Use when drafting civil sewer or utility layouts and connection details in Open Infra Studio, including pressure pipes, drainage, ducts, chambers and fittings.
---

# Riolering en nuts

Lees [bibliotheek en aansluitcontract](../../../docs/infra-bibliotheek.md). Stel stelsel, druk/vrijverval, leidingmateriaal, diameterdefinitie, status, coördinaten en bekende peilen vast. Een tekening of symbool is geen hydraulische dimensionering of uitvoerbaar boorplan.

Zoek werkelijke IDs en parametergrenzen via `infra_catalog`. Plaats een leiding met `infra_place`, een herkenbaar `instance`, een expliciet ontwerpafschot in `parameters.slope` (m/m), en alleen een vastgestelde aansluit-/BOB-hoogte als Z. Positief afschot daalt in de tracérichting. Een leiding langs meerdere XY-punten gebruikt `path`; deze punten bepalen de lengte, niet `parameters.length`.

Lees `infra_inspect` en sluit hulpstukken aan op de gerapporteerde poorten met `connect`, `port`, `own`. Gebruik een echte reducer voor Ø75→Ø63; forceer geen andere diameter om een afgewezen verbinding passend te laten lijken. Stelselverschillen en bezette poorten zijn fouten. Poorten bij een generieke put delen een gekozen aansluitniveau; aparte putbodem, BOB's en hydraulische werking zijn daarmee niet bepaald.

Bij 3 versus 5 mm/m uit verschillende documenten: leg revisie, status en toepassingsgebied vast. Kies niet stilzwijgend het gemiddelde of de nieuwste bestandsdatum. Bereid het tracé voor, houd afgeleide hoogten voorlopig en gebruik `confirm=true` pas wanneer de projectspecifieke keuze expliciet gemaakt is. De cataloguswaarde nul is geen aanbevolen verhang.

Een ontbrekende BOB blijft onbekend; leid hem niet af uit maaiveld of de naastgelegen put zonder ontwerpgegevens. Controleer na `infra_update` opnieuw de verbonden eindhoogten, diameters, richting en vrije aansluitingen. Houd mantelbuizen voor water, data en elektriciteit functioneel gescheiden. Bewaar ontwerpkeuzes en bronstatus in de projectgegevens, zonder persoonsgegevens of volledige bronbestanden aan de openbare repo toe te voegen.
