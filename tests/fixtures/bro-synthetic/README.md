# Zelfgemaakte BRO-testinvoer

Deze kleine XML-bestanden zijn specifiek voor de bestaande BRO-lezer
gemaakt. Ze zijn vereenvoudigde leesvoorbeelden, geen officiële BRO-XSD-
validatiebestanden en geen gegevens uit het register. Alle ids, locaties en
metingen zijn verzonnen. Bron: OpenAEC Foundation, Open Infra Studio, 2026.
Licentie: GPL-3.0-only, zoals [LICENSE](../../../LICENSE).

`cpt.xml` bevat twee metingen met expliciete kolommen en scheidingstekens,
plus RD en een breedte/lengtepositie in EPSG:4258. `borehole.xml` heeft één
bodemlaag. De tests veranderen getallen, scheidingstekens en ontbrekende
meetvelden gericht. Ze gebruiken uitsluitend de lokale parse-functies.

De vaste zoekbestanden in `../bro/` controleren ook herstel per zoekobject:
een kapotte offset in het middelste van drie objecten mag de twee geldige
buren niet veranderen. De nieuwe regressietest vergelijkt hun volledige
zoekmetadata en controleert een melding met het overgeslagen BRO-id.
Aanvullende tests controleren de bestaande nilwaarden en de geselecteerde
Nederlands/Engels-route voor interne leesdetails. Deze details worden tot
Ethans besluit niet aan voortgang of eindtekst toegevoegd. De lijst in het
zoekresultaat telt de overgeslagen objecten. Een onleesbaar afzonderlijk
CPT/BHR-object gebruikt de bestaande foutmelding voor een ontbrekend
bruikbaar object.

`borehole-characteristics.xml` bevat drie verzonnen BHR-zoekobjecten met
verschillende locaties, hoogtes, einddieptes, datums en kwaliteitsregimes.
De buurtest vervangt alleen de `offset` of `finalDepthBoring` van het
middelste object door `broken`. De volledige metadata en volgorde van de
twee gezonde buren moeten behouden blijven, met precies één intern
leesdetail dat het middelste BRO-id noemt. De onbeschadigde fixture moet
alle drie de objecten zonder leesdetails opleveren. Ook deze fixture is
een vereenvoudigd leesvoorbeeld, geen volledig BRO-XSD-validatiebestand.
