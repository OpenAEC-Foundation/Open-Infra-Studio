---
name: civiel-nlcs-oplevercontrole
description: Use when reviewing or delivering a civil drawing from Open Infra Studio, including NLCS mapping, library connections, units, quantities and source conflicts.
---

# Civiele NLCS-oplevercontrole

Lees [de bibliotheekreferentie](../../../docs/infra-bibliotheek.md), vooral conformiteit en bekende beperkingen. Controleer tegen de vastgelegde NLCS 5.0.2-tabellen; meng geen eisen of objecten uit een andere versie zonder afzonderlijke migratie.

Lees eerst de actieve sessie en tekeningidentiteit. Gebruik `infra_inspect` om instantie-ID's, parameters, onbekende hoogten, bronwaarschuwingen en verbindingen te controleren. Een zichtbaar gesloten lijntje is geen bewijs dat de opgeslagen aansluitpunten verbonden zijn. Controleer na opslaan en heropenen opnieuw de inspectiegegevens.

Voer `nlcs_check` eerst rapporterend uit. Controleer object-ID, hoofdgroep, discipline, status en toegestaan element G/S/A/T, gevolgd door kleur, lijndikte, lijntype en ByLayer-eigenschappen. Een opsluitband met alleen G mag niet op een verzonnen S-laag staan. Maak onderscheid tussen officiële symbolen en de eigen schematische bibliotheekvormen. Vereenvoudigde complexe lijntypen blijven een opleverbeperking.

Verifieer coördinaten en een bekende afstand; toets m/mm en eventuele relatieve oorsprong. Controleer NAP/BOB-bekendheid, peilverschillen uit afschot, poortbezetting, diameters en richting. Verwar CONCEPT in het titelblok niet met een bestandsnaam die `def` bevat.

Bereken hoeveelheden met een selectie van het ontwerp. Decoratieve onderdelen tellen niet opnieuw mee; controleer gemengde meetregels en sluit catalogus-/profielillustraties uit. Beoordeel legenda, maatvoering, profielverwijzingen, teksthoogten, noordpijl, plotschaal en titelblok ook visueel.

Rapporteer afzonderlijk wat automatisch is getoetst, wat visueel is gecontroleerd en welke bronconflicten of ontbrekende engineering openstaan. Noem een afgeleide PDF-DXF met alleen kleurlagen niet NLCS-conform. Herstel alleen onderbouwde fouten en bewaar het origineel. Bevestig opslaan pas na een geslaagde save-/heropencontrole van het bedoelde bestand.
