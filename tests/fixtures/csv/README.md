# Vaste CSV-testinvoer

`points.csv` is een kleine zelfgemaakte tabel met twee verzonnen punten,
decimale komma's en een aangehaald tekstveld met een puntkomma. Bron:
OpenAEC Foundation, Open Infra Studio, 2026. Licentie: GPL-3.0-only,
zoals [LICENSE](../../../LICENSE).

De tests lezen de tabel via de algemene bestandsroute. Gerichte varianten
dekken ontbrekende coördinaatvelden, onleesbare en niet-eindige getallen,
zeer grote eindige getallen, lege bestanden en geografische kolomnamen.
Ze gebruiken geen diensten. CSV met x/y declareert zelf geen CRS; de
bestaande lon/lat-variant geeft EPSG:4326 terug.

`multiline.csv` is eveneens zelfgemaakt en valt onder dezelfde bron en
licentie. Het bevat Sols twee punten met `put` en `west` op twee fysieke
regels binnen één aangehaald veld. De tests behouden beide punten en de
volledige attribuuttekst, ook met CRLF, lege regels en verdubbelde quotes
binnen het veld. Dit volgt [RFC 4180 §2](https://www.rfc-editor.org/rfc/rfc4180#section-2).

Aanvullende tests controleren het fysieke regelnummer na een meerregelig
record. Bij een niet afgesloten aangehaald veld blijft de oude verwerking
van de resterende fysieke regels beschikbaar, met een leesmelding; zo kan
een kapotte notitie geen latere geldige punten opslokken. De nieuwe
leesmeldingen gebruiken de bestaande Nederlands/Engels-vertaalroute en
staan in de PR ter beoordeling door Ethan.

De letterlijke reproductie zonder afsluitend regeleinde staat apart als
`sol_csv_multiline_note_preserves_both_points_full_text_and_no_warnings`:
twee punten, de eerste notitie exact `put\nwest` en nul waarschuwingen.
De negatieve controle gebruikt de CSV-lezer van reviewhead `1a1d1dd99`;
de recordlezer daarna bewaart ook LF/CRLF en het fysieke beginregelnummer.

`FileData.skipped` telt alleen records met onbruikbare coördinaten. Een
melding over een niet afgesloten notitie telt niet als overgeslagen punt
als de terugval op fysieke regels beide punten behoudt. Een regressietest
controleert die situatie. Een andere test leest werkelijk kapotte algemene
GML naast CSV en bewaakt dat de interne GML-details stil blijven.

Volgens de laatste opdracht blijven de bestaande CSV-meldingen en de
IMRO-melding `{id}: geen tekenbare geometrie` beschikbaar voor de
bestandenpagina. Die IMRO-tekst kan daardoor eerder verschijnen, bij het
lezen van het bestand. De plaatsingsroute houdt de bestaande IMRO-uitvoer.
De exacte NL/EN-teksten en het moment van tonen staan in de PR voor Ethan;
deze wijziging geeft geen akkoord om ze samen te voegen of uit te rollen.
