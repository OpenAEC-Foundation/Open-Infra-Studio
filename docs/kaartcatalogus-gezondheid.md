# Gezondheid van de kaartcatalogus

De openbare catalogus komt uit twee bronnen:

- `assets/servers/world.json`: 600 wereldwijde vermeldingen, met `id`, `kind`, `name`, `url`, `default_layer`, land, aanbieder, licentie, coördinatenstelsels en WGS84-dekking (`bbox`). De import komt uit GIS2BIM via `tools/import_gis2bim_servers.py`.
- `src/gis/servers.rs`: 29 ingebouwde presets, inclusief de vier Topotijdreis-caches. `src/gis/catalog.rs` voegt deze samen met de wereldcatalogus en de eigen servers van de gebruiker.

De wekelijkse controle leest de eerste twee bronnen rechtstreeks. Eigen servers van gebruikers worden niet gelezen. De wereldcatalogus gebruikt `wms`, `wfs` en `xyz`; WMTS-tegeltemplates vallen daarin onder `xyz`. Het controlescript begrijpt daarnaast expliciete `wmts`-capabilities. Het wijzigt geen catalogusgegevens, verificatiedata of appbestanden.

## Een meting uitvoeren

Vereist: Python 3.12 en curl 8.4 of nieuwer, voor een maximale antwoordgrootte die ook bij een onbekende Content-Length wordt afgedwongen. De workflow gebruikt Ubuntu 24.04. Er is geen Rust-build of Python-pakketinstallatie nodig.

```bash
python3 tools/check_map_services.py
```

Dit schrijft `target/catalogue-health/rapport.json` en `rapport.md`. De JSON bevat alle oorspronkelijke vermeldingen met de uitkomst, reden, duur, HTTP-status, proefaanvraag en het aantal HTTP-verzoeken. Beide rapporten leggen de SHA-256 van de catalogusbronnen en het controlescript vast. De Markdown groepeert alle diensten op uitkomst, met de volledige lijst mislukte controles vooraan.

Instellingen, bijvoorbeeld voor een kleinere catalogus:

```bash
python3 tools/check_map_services.py --catalogue /tmp/diensten.json --no-builtins \
  --timeout 8 --slow 3 --workers 6 --output /tmp/kaartcatalogus
```

## Wat wordt gemeten

- **WMS, WFS en WMTS:** één GetCapabilities, zonder een versie af te dwingen. Bestaande endpointparameters zoals `map=` blijven behouden. Het antwoord moet geldige capabilities van het juiste protocol bevatten, met benoemde lagen of objecttypen. Een ingestelde standaardlaag moet aanwezig zijn. HTTP 200 met een HTML-foutpagina of OGC-exception telt als mislukt.
- **XYZ en WMTS-tegeltemplates:** één tegel op zoom 14, of een lager aangegeven maximum, op het midden van de WGS84-dekking. De volgorde van `{x}` en `{y}` blijft zoals de template die beschrijft.
- **ArcGIS-tegelcaches:** cachemetadata, gevolgd door één tegel bij het midden van de native dekking. Dit ondersteunt ook de Topotijdreis-caches in RD zonder externe coördinatentransformatie.

Identieke proefaanvragen worden gedeeld; de standaardlaag wordt daarna voor iedere vermelding apart gecontroleerd. Er zijn maximaal zes verzoeken tegelijk, één per oorspronkelijke host. Ieder verzoek heeft een totale time-out van acht seconden, maximaal twee redirects en een antwoordlimiet van 16 MiB. Er zijn geen automatische herhalingen. Een ArcGIS-controle doet maximaal twee proefaanvragen. Redirects tellen mee in het gerapporteerde totaal.

De uitkomsten zijn:

- **werkt:** de proef slaagt binnen drie seconden;
- **traag:** de proef slaagt vanaf drie seconden; voor ArcGIS telt de som van metadata en tegel;
- **kapot:** de proef mislukt, met de concrete reden.

Dit is een momentopname vanaf de meetomgeving. Een time-out, TLS-probleem, toegangsblokkade of ontbrekende tegel is geen bewijs van een wereldwijde of blijvende storing. Een tegel kan buiten het werkelijke gegevensgebied liggen, ook binnen de grove catalogusdekking. PNG/JPEG/WebP wordt op bestandskenmerken gecontroleerd, niet volledig gedecodeerd. Geldige capabilities bewijzen geen geslaagde GetMap, GetFeature of import in Open CAD Studio. Gebruik deze uitkomsten dus om nader onderzoek te kiezen; verwijder geen diensten op basis van één meting.

Een onleesbare catalogus, onbekende ingebouwde declaratie, ontbrekende afhankelijkheid, programmeerfout of mislukte rapportuitvoer geeft exitcode 1. Mislukte openbare diensten geven exitcode 0 en blijven zichtbaar in het rapport.

## Automatisch en offline

De afzonderlijke workflow **kaartcatalogus** draait elke woensdag om 04:23 UTC en is handmatig te starten vanuit Actions. Hij bewaart JSON en Markdown 90 dagen als `kaartcatalogus-rapport` en toont het Markdown-rapport bij de uitvoering. Deze workflow reageert niet op pushes of pull requests. De wekelijkse planning gaat pas werken nadat de workflow op de standaardbranch is samengevoegd.

De workflow **kaartcatalogus-tests** voert bij relevante PR-wijzigingen uitsluitend de vaste tests uit. Ook de bestaande offline testontdekking vindt die tests. Er worden geen openbare diensten benaderd:

```bash
python3 -m unittest discover -s tests -p 'test_catalogue_health.py' -v
```

De fixtures in `tests/fixtures/catalogue-health/` zijn synthetisch. Ze behandelen geldige en ongeldige capabilities, verdwenen standaardlagen, tegeltemplates, ArcGIS-coördinaten, HTTP-fouten, time-outs, gedeelde endpoints en de exitcodes van de rapportage.
