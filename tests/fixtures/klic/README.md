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
