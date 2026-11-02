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
