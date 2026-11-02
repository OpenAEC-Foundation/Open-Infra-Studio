---
name: civiel-situatietekening
description: Use when creating or editing civil situation drawings in Open Infra Studio, including paving, kerbs, public space layouts and existing versus proposed conditions.
---

# Civiele situatietekening

Gebruik de repositorybibliotheek voor herleidbare situatieobjecten. Lees [de commandoreferentie](../../../docs/infra-bibliotheek.md) voor argumenten, coördinaten en beperkingen.

Stel bronstand, project-CRS, eenheden, oorsprong, tekenstatus en plotschaal vast uit het project. Vergelijk bestandsnaam met titelblok; een recentere bestandsdatum vervangt geen vastgestelde documenthiërarchie. RD is horizontaal; NAP wordt niet uit RD afgeleid.

Lees `oi_sessions`, kies de bedoelde tekening en lees `infra_catalog` met thema/zoekwoord. Bij gebruik van de directe CAD-bridge: lees eerst capabilities en relevante schema's. Een beschikbare toolnaam is geen bewijs dat de actieve host het nieuwe plugincommando kent.

Plaats puntobjecten met `infra_place(item=..., at=[x,y,z])`, lijnen/vlakken met absolute projectpunten in `path`. Alleen als de hoogte bekend is een Z meesturen. Geef MCP en de plugincommando's altijd absolute projectmeters: de adapter trekt de oorsprong af en rekent naar tekeningeenheden. Reken de invoer niet vooraf om. De controleformule voor een relatieve millimetertekening is `(projectpunt − oorsprong) × 1000`. Controleer minstens twee bekende punten en één afstand.

Gebruik `connect` en de uit `infra_inspect` gelezen poortnaam voor aansluitende onderdelen. Een band langs een vlak krijgt `port="edge-0"` of de werkelijke randindex; de binnenrand sluit aan. Gebruik `infra_update` bij parameterwijzigingen, zodat afhankelijke objecten volgen. Verwar plotschaal niet met objectafmetingen.

Classificeer kleuren uit een afgeleide PDF-DXF niet automatisch als NLCS-objecten. Behoud de bron en bouw een afgeleide tekening met onderbouwde objectbetekenis. Controleer werkgrens, aansluitingen, materiaalgrenzen, status B/N/T/V, legenda, schaal en leesbare labels. Voer `nlcs_check` uit en beoordeel ook de visuele uitvoer; een geldige laagnaam bewijst geen volledige conformiteit.
