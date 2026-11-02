# Vaste KLIC-testinvoer

Deze kleine levering is zelfgemaakt. De waterleiding, KL-code en het
meldnummer zijn verzonnen. Bron: OpenAEC Foundation, Open Infra Studio,
2026. Licentie: GPL-3.0-only, zoals [LICENSE](../../../LICENSE).

`GI_gebiedsinformatielevering_SYNTHETIC_1_V2.xml` is een vereenvoudigd
IMKL-leesvoorbeeld, geen officiële Kadaster- of XSD-validatielevering.
`delivery.zip` bevat precies dit XML-bestand in de map `delivery/`, met
een vaste ZIP-datum van 1 januari 2026. Beide worden via `klic::read` getest.
De `example.invalid`-URL is uitsluitend een thema-id, geen dienst.

De tests maken tijdelijke varianten voor leeg/kapot XML, een kapotte ZIP,
ontbrekende bestanden en een Unicode-object-id waarop de onderliggende
lezer kan panikeren. Ze gebruiken geen lokale voorbeeldlevering buiten de
repository en geen live diensten.

Aanvullende vaste varianten controleren dezelfde levering onder de naam
`copy_SYNTHETIC_V2.xml` en met extensie `.gml`, met behoud van zowel het
meldnummer als het attribuut `melding`. Andere varianten bevatten alleen
een lege of onbruikbare lijn, een losse geldige puntgeometrie, of een
geldige leiding naast een lege of onbruikbare lijn. De fouttekst uit de
lezer bevat geen `KLIC: `-prefix; de bestaande app voegt die zelf toe.

De geometriecontrole is een controle op tekenbaarheid, geen nieuwe
IMKL-schemavalidatie. Zij weigert de levering alleen als geen punt of lijn
met minstens twee punten overblijft. De onderliggende converter kan bij
kapotte XML of een interne fout nog steeds het hele bestand weigeren;
de adapter heeft geen betrouwbare objectgrens om die fouten te herstellen.
