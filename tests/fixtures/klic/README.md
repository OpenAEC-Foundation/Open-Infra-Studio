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

De adapter verwijdert lijnen met minder dan twee bruikbare punten en telt
een geconverteerd object alleen als overgeslagen wanneer geen punt of lijn
overblijft. De telling staat in `Levering.skipped` en wordt nog niet getoond.
De externe converter laat bronlijnen met nul of één positie al weg voordat
hij GeoJSON teruggeeft; zulke bronobjecten vallen buiten deze adaptertelling.
De tests leggen dit onderscheid vast en controleren dat de geldige buur en
de aantallen per thema behouden blijven.

Als de converter een lege levering teruggeeft, blijft dat de bestaande
lege succesuitkomst. De letterlijke invoer `<Other/>` levert daardoor weer
`KLIC 1: 0 objecten van 0 netbeheerders.` op bij het verzonnen meldnummer 1.
De test controleert de volledige melding met de bestaande appformattering;
de productiecode van de app en de bestandsopdracht blijft ongewijzigd.

Voor geconverteerde objecten waarvan geen tekenbare geometrie overblijft
en voor een opgevangen converterpanic blijft de bestaande NL/EN-fout via
`lang::tr` behouden. Er is geen nieuwe zichtbare tekst toegevoegd en de
bestaande app voegt de KLIC-prefix nog steeds één keer toe.

De geometriecontrole is een controle op tekenbaarheid, geen nieuwe
IMKL-schemavalidatie. Zij weigert alleen een niet-lege converteruitkomst
waarvan geen punt of lijn met minstens twee punten overblijft. Lege of
door de converter al overgeslagen brongeometrie blijft toegestaan, net als
op main. De onderliggende converter kan bij
kapotte XML of een interne fout nog steeds het hele bestand weigeren;
de adapter heeft geen betrouwbare objectgrens om die fouten te herstellen.
