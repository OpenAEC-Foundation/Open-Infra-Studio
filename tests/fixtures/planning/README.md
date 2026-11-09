# Zelfgemaakte IMRO-leesvoorbeelden

`unusable-imro.gml` bevat één Enkelbestemming met id `zone` zonder geometrie,
in geldige XML met EPSG:28992. `partial-imro.gml` bevat daarnaast één goed
woonvlak en een tweede object zonder geometrie. Alle gegevens zijn verzonnen.
Bron: OpenAEC Foundation, Open Infra Studio, 2026. Licentie: GPL-3.0-only,
zoals [LICENSE](../../../LICENSE). Dit zijn kleine leesvoorbeelden voor de
bestaande parser, geen volledige IMRO-XSD-conformiteitsbestanden.

De regressieproeven lezen deze vaste bestanden via de importpagina en de
echte opdrachtimport, in afzonderlijke Nederlandse en Engelse processen.
De testhost bewaart de aangemaakte tekenobjecten, lagen en undo-stappen;
er is geen CAD-installatie, schema-opvraag of live dienst nodig. Een geheel
onbruikbaar bestand moet de bestaande foutzin en objectoorzaak teruggeven,
zonder tekenen of undo. Bij deels leesbare invoer blijft het woonvlak staan
en staat elke oorzaak één keer, op een aparte regel onder de vertaalde
resultaatzin. De proeven veranderen de taal van het hoofdproces niet.
