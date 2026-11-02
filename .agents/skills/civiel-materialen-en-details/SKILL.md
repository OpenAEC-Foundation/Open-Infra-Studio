---
name: civiel-materialen-en-details
description: Use when selecting civil material variants or drafting construction build-ups and infrastructure connection details in Open Infra Studio.
---

# Materialen en details

Lees [bibliotheekgegevens en commando's](../../../docs/infra-bibliotheek.md). Haal onderdelen uit `infra_catalog`; controleer materiaal, functionele toepassing, maatdefinitie, bron en toegestane parameters. Werkelijke productmaten en schematische symboolmaten mogen niet worden verwisseld.

Gebruik expliciete varianten: `grasbeton-100` en `grasbeton-120` bewaren verschillende dikten; een bronconflict wordt geen gemiddelde 110 mm. `confirm=true` betekent dat een bronkeuze beoordeeld is, niet dat een onopgeloste ontwerpvraag verdwenen is. Leg vast welke projectbron doorslaggevend is.

Vlakken meten m², lijnen m en punt-/detailobjecten st. Laagdikten staan afzonderlijk in meters; de UI toont hun materiaalmaten ook in millimeters. Dubbeltel contour en decoratieve lijn niet. Profielen zijn illustraties en horen alleen in hoeveelheden als de selectie en meetregel dat beogen.

Plaats een laagopbouw met een werkelijk bestaand `profiel-*`-ID. Een samengesteld profiel krijgt per component zijn eigen NLCS-object. Gebruik voor leidingdetails echte aansluitbare hulpstukken en controleer hun poorten met `infra_inspect`; een profielrechthoek is geen maatvast leidingdetail.

Nieuwe catalogusvarianten worden in `tools/build_infra.py` beheerd en met `python tools/build_infra.py` gegenereerd. Controleer het object-ID in `assets/nlcs/objects.tsv`, toegestane G/S/A-representatie, parametergrenzen, bronlocatie en licentie. Geen ontbrekende mapping verhullen met een algemene laag. Neem geen leveranciersgeometrie over zonder passende rechten; eigen parametrische geometrie krijgt een herkenbare herkomst.

Voer de catalogus- en geometriecontroles uit en vergelijk preview met de maatvoering. Houd bibliotheekbestanden en deze skills in de repository; verwijs relatief, zonder persoonlijke installatiepaden.
