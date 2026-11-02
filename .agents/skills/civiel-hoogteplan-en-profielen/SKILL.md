---
name: civiel-hoogteplan-en-profielen
description: Use when drafting height plans, ground levels, civil cross sections or longitudinal profiles in Open Infra Studio, especially with NAP, slopes and differing horizontal and vertical scales.
---

# Hoogteplan en profielen

Lees [eenheden en profielbeperkingen](../../../docs/infra-bibliotheek.md). Identificeer meet-/terreinbron, opnamedatum, horizontaal CRS, verticaal datum, bestaande en nieuwe toestand. Een AHN-hoogte, maaiveldpeil en leiding-BOB zijn verschillende gegevens. Gebruik onbekende peilen niet als nul NAP.

Voor maatgevende hoogtepunten: `infra_place` met `item="hoogtepunt"` en vastgestelde `at=[x,y,z]`. Zonder Z blijft `NAP ?` zichtbaar. `scale` is de noemer van de plotschaal; tekst 2,5 mm op papier is niet 2,5 meter in het model.

Voor echte terrein-/asprofielen gebruik je de aanwezige MCP-functies `civ_length_profile`, `civ_design_profile` en `civ_cross_sections`, nadat je de werkelijke invoerschema's hebt gelezen. Gebruik een bestaande as en gecontroleerd terrein. Stel horizontale en verticale schaal afzonderlijk in; noteer eventuele overhoging op het blad. Een catalogusprofiel zoals `profiel-nutssleuf` is een schematische doorsnede en vervangt geen ingemeten lengteprofiel.

Gebruik `infra_catalog(category="profielen")` voor materiaaldoorsneden. Plaats via `infra_place`; lees de laagdikten en componentclassificaties terug. Een samenstel van rechthoeken bewijst geen taludstabiliteit, grondbalans of veilige ligging van kabels.

Controleer hoogten met een onafhankelijke berekening: `eindpeil = beginpeil − horizontale tracélengte × afschot`. Controleer maatgevende punten, kruin/teen, waterpeilen en aansluiting op bestaand terrein. Bij ontbrekende inmetingen blijft het betreffende profieldeel voorlopig. Bij RD naar een relatieve millimetertekening trek je eerst de projectoorsprong af en schaal je daarna; pas geen tweede omzetting toe op reeds omgezette geometrie.
