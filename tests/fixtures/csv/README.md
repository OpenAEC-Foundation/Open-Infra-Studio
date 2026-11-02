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
