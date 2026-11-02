# Infra-bibliotheek

De bibliotheek is onderdeel van de Open Infra-plugin. Open **Bibliotheek → Infra-objecten** of voer `GIS_INFRA_LIBRARY` uit. Voertuigen blijven in hetzelfde venster beschikbaar. Zoek op naam, thema of materiaal; kies een onderdeel, pas de parameters aan en wijs de invoegpunten of hoekpunten aan. Eenheden staan bij ieder invoerveld.

## Objecten en representatie

De volledige inventaris staat in [infra-objecten.md](infra-objecten.md). De catalogus bevat verhardingen, banden, persriool en hulpstukken, vrijvervalriool, drainage, wadi's, nutsleidingen en mantelbuizen, putten, kasten, verlichting, laden, groen, inrichting, verkeer, annotaties en samengestelde profielen.

De objecten gebruiken de NLCS **5.0.2-objectclassificatie** en bijbehorende laagrepresentatie. De geometrie is eigen, parametrische en waar aangegeven schematische geometrie. Deze uitgave claimt geen officiële symboolvormen, volledige conformiteitscertificering, hydraulische toetsing of constructieve dimensionering. De bestaande hostadapter vereenvoudigt complexe NLCS-lijntypen indien hun vormcomponenten niet beschikbaar zijn. Controleer de uitvoer en vereiste templates bij oplevering.

Catalogusgegevens: `assets/library/infra.json`. Reproduceerbaar opbouwen: `python tools/build_infra.py`. Alle ID's worden tegen de meegeleverde NLCS-tabellen gecontroleerd. Materiaaltype, diameter en laagdikte zijn objectparameters; er worden geen eigen materiaalnamen aan officiële NLCS-objectnamen toegevoegd.

## Intelligente aansluitingen

- In-/uitgaande aansluitpunten hebben positie, richting, stelsel en diameter in millimeters.
- Een gekoppeld onderdeel draait en verschuift naar zijn aansluiting, inclusief de bekende hoogte.
- Diameter- of stelselverschillen worden geweigerd. Een expliciete reducer Ø75→Ø63 maakt die overgang mogelijk.
- Bochten, T-stukken, afsluiters, keerkleppen, ontluchters, putten en leidingen hebben eigen aansluitpunten.
- Een poort kan maar één verbinding hebben. Ontbrekende ouders en kringverbindingen worden geweigerd.
- Bij wijzigingen worden het object en afhankelijke onderdelen opnieuw berekend en in één hosttransactie vervangen. Een ongeldige wijziging wordt vóór die transactie geweigerd.
- Banden kunnen aan een vlakrand worden gekoppeld met `PORT=edge-0`, `edge-1`, enzovoort. De band volgt de randlengte; zijn binnenrand sluit op het vlak aan. Dit is een rechte band per vlakrand, geen automatische complete omranding met hoekstukken.
- Leidingen krijgen afschot en bijbehorende eindhoogten. Positief afschot daalt met de stationering. De lengtehoeveelheid is de horizontale tracélengte.
- Objectparameters en verbindingen staan als versiegebonden OPENINFRA-XDATA in de tekening. Decoratie telt niet nogmaals mee als hoeveelheid.

De verbindingsstructuur is een gerichte boom: elk onderdeel heeft maximaal één ouder, maar kan meerdere vrije uitgaande poorten hebben. Gesloten netlussen, vrije geometrische kruisingdetectie, automatische botsingscontrole en automatische selectie van hulpstukken vallen buiten deze uitgave. Verbindingen worden expliciet gekozen, niet geraden uit nabijheid. Gebruik de update-opdracht voor wijzigingen; los verschuiven of exploderen buiten deze opdrachten verandert de opgeslagen parameters niet.

## Commando's

Alle coördinaten zijn **projectcoördinaten in meters**. Bij een relatieve tekenoorsprong wordt deze vóór de omzetting naar tekeningeenheden afgetrokken. `ROTATION` is in graden. Een `PATH` bevat absolute project-XY-punten; de opgeslagen vorm gebruikt lokale meters. `SCALE` is de noemer van de plotschaal en beïnvloedt annotatie/lijntypes, niet de werkelijke objectmaten.

| Commando | Invoer en resultaat |
|---|---|
| `GIS_INFRA_LIBRARY` | Venster openen; alias `GIS_INFRA_BIBLIOTHEEK` |
| `GIS_INFRA_CATALOG QUERY=... CATEGORY=...` | Catalogus inclusief parametergrenzen, bronnen en aansluitmogelijkheden; beide filters zijn optioneel |
| `GIS_INFRA_PLACE ITEM=...` | Interactief invoegpunt/richting of lijn-/vlakpunten kiezen; Enter voltooit, Esc annuleert |
| `GIS_INFRA_PLACE ITEM=... AT=x,y,z` | Op opgegeven positie plaatsen; `INSTANCE=naam` kiest een herkenbaar uniek instantie-ID |
| `GIS_INFRA_PLACE ... PATH=x,y;x,y;...` | Lijn of gesloten vlak; geen zelfsnijdende polygonen |
| `GIS_INFRA_PLACE ... CONNECT=instantie PORT=out OWN=in` | Plaatsen aan een vrij aansluitpunt van een bestaand bibliotheekobject |
| `GIS_INFRA_INSPECT [INSTANCE=...]` | Objecten, opgeslagen parameters, poorten, vlakrandpunten en waarschuwingen lezen |
| `GIS_INFRA_UPDATE INSTANCE=... LENGTH=...` | Een of meer bestaande parameters wijzigen; aangesloten kinderen volgen |

Optioneel bij plaatsing: `STATUS=B|N|T|V`, `DISCIPLINE=XX`, `SCALE=500`, `ROTATION=0`. Parameters staan in de catalogus en gebruiken hun sleutel als argument: bijvoorbeeld `LENGTH`, `WIDTH`, `HEIGHT`, `DIAMETER`, `SLOPE`, `ANGLE`, `OUT_DIAMETER`. Onbekende sleutels, niet-eindige getallen en waarden buiten de grenzen leveren een fout op.

Voor automatisering voeg je `ID=verzoeknaam` toe; het bestaande resultaatprotocol retourneert `data.infra` voor plaatsing, inspectie en bijwerken, en `data.items` voor de catalogus. Gebruik de teruggegeven instantie-ID's en poortnamen. Tekenstatus en documentstatus zijn verschillende gegevens; een verplaatst bestaand object is geen extra NLCS-status.

### Voorbeeld: leiding, afsluiter en reducer

```text
GIS_INFRA_CATALOG QUERY=persleiding
GIS_INFRA_PLACE ITEM=persleiding-pe75 INSTANCE=hoofd AT=0,0,9 LENGTH=10 SLOPE=0.003 CONFIRM=true
GIS_INFRA_PLACE ITEM=afsluiter-pe75 INSTANCE=klep CONNECT=hoofd PORT=out OWN=in
GIS_INFRA_PLACE ITEM=reductie-75-63 INSTANCE=verloop CONNECT=klep PORT=out OWN=in
GIS_INFRA_PLACE ITEM=persleiding-pe63 INSTANCE=tak CONNECT=verloop PORT=out OWN=in LENGTH=5 SLOPE=0.003 CONFIRM=true
GIS_INFRA_UPDATE INSTANCE=hoofd LENGTH=12
GIS_INFRA_INSPECT INSTANCE=tak
```

### Voorbeeld: verharding met aansluitende band

```text
GIS_INFRA_PLACE ITEM=straatbaksteen-kf INSTANCE=vak AT=0,20,9 PATH=0,20;8,20;8,23;0,23
GIS_INFRA_PLACE ITEM=opsluitband-150x250 INSTANCE=band CONNECT=vak PORT=edge-0 OWN=in
```

Wanneer `PATH` is opgegeven bepaalt dat pad de vorm; `LENGTH`/`WIDTH` wijzigen dan niet het bestaande pad. Wijzig het tracé met `GIS_INFRA_UPDATE INSTANCE=... PATH=x,y;x,y;...` of `infra_update(instance=..., path=[[x,y],...])`, met absolute projectmeters. Aangesloten kinderen worden herberekend. Voor een rechthoek die met lengte/breedte moet meegroeien laat je `PATH` weg. Bij een getekend pad is `SLOPE` wel wijzigbaar. Bij een gekoppelde vlakrand bepaalt de rand de lengte van de band; wijzig daarvoor het bovenliggende vlak.

## Bronnen, hoogten en hoeveelheden

De projectreferenties zijn de technische omschrijving van 7 oktober 2026 en de ontwerpbladen in de stand van juli 2026. Bronbestanden zijn niet meegekopieerd. Materiaal-/detailkeuzes daaruit zijn referentievarianten, geen algemene gemeentelijke eisen.

Persleidingafschot 3 of 5 mm/m en grasbeton 100 of 120 mm zijn onopgeloste bronverschillen. De catalogus bewaart beide verwijzingen. Plaatsing van deze varianten vraagt `CONFIRM=true`; stel de ontwerpwaarden expliciet in. Een referentienaam met `def` bewijst geen goedkeuring: sommige titelblokken vermelden CONCEPT.

Zonder expliciete Z bij `AT` is de hoogte **onbekend**. Z=0 in de geometrie is dan uitsluitend schematisch. Een hoogtelabel toont `NAP ?`; een aansluiting erft de bekendheid van zijn ouder. Een putlocatie of maaiveldhoogte bewijst geen BOB. De generieke symbolen gebruiken het gekozen aansluitniveau; putdiepte en leiding-BOB moeten afzonderlijk worden uitgewerkt.

Hoeveelheden worden uit de hoofdrepresentatie berekend: m voor lijnen, m² voor vlakken en st voor puntobjecten/details. De overige lijnen van hetzelfde object zijn decoratie. Meerdere meetregels op dezelfde NLCS-laag blijven afzonderlijke regels. Profielbladen en catalogusoverzichten zijn illustraties: gebruik voor een projectstaat uitsluitend de betreffende ontwerpselectie. Laagdikten zijn metadata; er wordt geen funderings- of grondbalansberekening uit afgeleid.

## MCP en skills

De bestaande MCP-server biedt `infra_catalog`, `infra_place`, `infra_inspect` en `infra_update`. Hun argumenten volgen bovenstaande commando's; `parameters` is een woordenboek met numerieke parameterwaarden. Lees de catalogus vóór plaatsing. Vraag de sessie op met `oi_sessions` en gebruik een expliciete `at`, `path` of `connect` voor geautomatiseerde plaatsing.

De zes projectskills staan onder `.agents/skills/`: situaties, riolering/nuts, hoogteplannen/profielen, materialen/details, groen/inrichting en oplevercontrole. Ze verwijzen naar deze commandoreferentie en werken zonder persoonlijke skillinstallatie.

## Voorbeelden en controle

`cargo run --example infra_catalogue -- examples/infra-bibliotheek` genereert een CAD-catalogus en een visuele SVG uit dezelfde Rust-geometrie als de plugin. Controleer de catalogus, de geïntegreerde tests en de native plaatsing in een bij de plugin passende Open CAD Studio-build. De SVG is een overzicht met een eigen passende schaal per cel, geen maatvast tekenblad.

```text
cargo test -p open-infra
cargo test -p open-infra-civiel
python -m unittest discover -s tests -p "test_infra_*.py"
python tools/build_infra.py
```

Gebruik een Rust-toolchain die de MSRV van de bestaande UI-afhankelijkheden ondersteunt. Voor deze uitbreiding is Rust 1.99.1 gebruikt.

Voer de tests en builds van de twee plugins afzonderlijk uit: een gezamenlijke workspace-test verenigt de plugin-features en veroorzaakt dubbele host-exportsymbolen. Dit volgt uit de bestaande crate-opzet.
