# IFC 4.3-export controleren

De vaste exportvoorbeelden worden bij iedere pull request en push naar `main` opnieuw geschreven en lokaal gevalideerd. De controle gebruikt IfcOpenShell 0.8.4.post1 voor het schema `IFC4X3_ADD2`, de EXPRESS-regels en de asgeometrie, en IfcTester 0.8.4 voor eigen informatie-eisen in buildingSMART IDS 1.0.

## Wat wordt geëxporteerd?

`GIS_IFC` en `CIV_IFC` gebruiken dezelfde route: `src/ifc/mod.rs::command` leest de tekening, selectie, projectlocatie en eenheden, en roept `ifc::write` aan. De schrijver classificeert objecten vanuit NLCS-lagen en objectgegevens, groepeert onder meer terrein en gebouwen, schrijft asgeometrie en relaties en maakt een STEP-bestand met `FILE_SCHEMA(('IFC4X3_ADD2'))`.

De tests gebruiken deze productieschrijver. De DWG-voorbeelden worden opnieuw ingelezen; de eenheden komen uit de DWG-kop en de projectlocatie uit het opgeslagen `OPENINFRA/SITE`-record. Er worden geen actuele kaartgegevens opgehaald.

## Vaste voorbeelden

| Export | Bron en onderzochte grens |
|---|---|
| `voorbeeld.ifc` | Bestaande synthetische gemengde tekening: TIN, fototerrein, gebouw, kabels, leidingen, bodemlagen, ontgraving, weg, markering, tekst en een as met verticaal ontwerpprofiel. Georeferentie EPSG:28992. |
| `catalogus.ifc` | De opgeslagen DWG van alle 94 bibliotheekonderdelen. De export bevat meerdere CAD-elementen per onderdeel. |
| `aansluitvoorbeeld.ifc` | De opgeslagen DWG met de leidingaansluiting en verharding met opsluitband. |
| `dordrecht-nlcs.ifc` | De opgeslagen voorbeeldtekening rond de Grote Kerk, inclusief de eigen projectlocatie in EPSG:28992. |
| `assen.ifc` | Links- en rechtsdraaiende horizontale assen, met beginstation 125 m. |
| `losse-objecten.ifc` | Een markering zonder onderliggende verharding en een ontgraving zonder terrein; de bestaande terugvalclassificaties moeten geldig blijven. |
| `millimeters.ifc` | Een leiding van 10.000 tekeneenheden moet als 10 meter worden geschreven, met quotes, een accent en een backslash in de objectnaam. |
| `leeg.ifc` | Een lege tekening, met een project en terreinlocatie maar zonder geëxporteerde ontwerpobjecten. |
| `lege-selectie.ifc` | Een lege expliciete selectie uit een niet-lege tekening. |

De unit-test in `src/ifc/mod.rs` schrijft het gemengde model. `tests/ifc_examples.rs` schrijft de andere acht. `tests/data/ifc43-cases.json` legt de verwachte klassen, minimumaantallen producten, georeferentie en de millimeterconversie vast. Daardoor is ook een schema-geldige maar onverwacht lege export een fout.

## Controles en bewijs in CI

`tests/ifc_suite.py` controleert ieder verwacht bestand en geeft exitcode 1 als een bestand ontbreekt, een controle afbreekt of een controle faalt. Per model worden uitgevoerd:

1. `tests/ifc_validate.py`: schema-identificatie, IfcOpenShell-schema- en EXPRESS-validatie en de bestaande geometriecontrole van horizontale en verticale assen.
2. Het eigen IDS-contract in `tests/data/ifc43-export.ids`: één benoemd project en namen en `OpenInfra_NLCS.Laag` voor de opgesomde geëxporteerde objectklassen. Gegenereerde structuuronderdelen zoals een `IfcRoad`, `IfcBorehole` en de onderverdeling van een as vallen niet onder die laag-eis.
3. De onafhankelijke verwachtingen uit het modellenoverzicht: aantallen, klassen, CRS en lijnpunten in meters.

Het IDS-bestand wordt zelf aan het meegeleverde XSD-schema getoetst. De W3C-import wordt naar de lokale kopie in `xmlschema` verwezen; externe schema's zijn verboden. Een regressietest voert deze route met geblokkeerde sockets uit. Andere negatieve tests verwijderen de projectnaam, het project of de laaggegevens en moeten worden afgekeurd.

De workflow bewaart de IFC-bestanden en `validatie.json` als artifact `ifc43-validatie-<commit>`, ook als de validatiestap faalt. Het rapport bevat de gebruikte validatorversies, SHA-256 van ieder IFC-bestand en het IDS-contract, de schema-exitcodes en de foutmeldingen. De STEP-header bevat het exporttijdstip: een nieuwe export kan daarom een andere bestandshash krijgen bij dezelfde geometrie.

De validatie gebruikt geen live GIS- of validatiedienst. Het installeren van de vastgelegde Python-pakketten en Rust-afhankelijkheden gebeurt wel via de gebruikelijke pakketbronnen van de CI.

## Zelf herhalen

Gebruik de Rust-versie uit `.github/workflows/test.yml` (momenteel 1.99.1) en Python 3.12. Voer vanaf de repositoryroot uit:

```sh
python3 -m venv /tmp/open-infra-ifc-venv
/tmp/open-infra-ifc-venv/bin/python -m pip install -r tests/requirements-ifc.txt
cargo test --lib --locked -p open-infra ifc::
cargo test --test ifc_examples --locked -p open-infra
/tmp/open-infra-ifc-venv/bin/python tests/ifc_suite.py target/ifc-test --report target/ifc-test/validatie.json
/tmp/open-infra-ifc-venv/bin/python -m unittest discover -s tests -p test_ifc_suite.py -v
```

Een afzonderlijke export kan worden gecontroleerd met:

```sh
/tmp/open-infra-ifc-venv/bin/python tests/ifc_validate.py /pad/naar/model.ifc
```

## Wat kunnen we claimen?

Na een geslaagde CI op de betrokken commit:

> Onze vaste IFC 4.3-exportvoorbeelden worden automatisch gecontroleerd aan IFC4X3_ADD2 met IfcOpenShell, inclusief EXPRESS-regels en controles van de asgeometrie. Daarnaast toetsen we eigen informatie-eisen met buildingSMART IDS.

Deze controle bewijst niet dat iedere mogelijke tekening een geldige export oplevert. Er is geen validator ingebouwd die iedere export bij de gebruiker blokkeert. De geteste informatie-eisen zijn ons eigen contract, geen IDS van een opdrachtgever. Een geslaagde IDS-toets bewijst geen algemene IFC-conformiteit.

Er wordt geen buildingSMART-certificering of geslaagde toets aan alle normatieve regels van de buildingSMART Validation Service geclaimd. De volledige dienst en zijn Gherkin-regelset draaien niet in deze CI. Ook volledige geometrische/topologische correctheid, levering zonder gegevensverlies, de acceptatie door ieder BIM-pakket en iedere gebruikshandeling in het pluginvenster zijn hiermee niet bewezen.

De grote opgeslagen `examples/dordrecht-3d/dordrecht-3d.ifczip` valt buiten deze vaste CI-set. De controle van de kleinere Dordrecht-NLCS-tekening zegt niets over de geldigheid van dat afzonderlijke archief.

Bronnen: [IfcOpenShell-validatie](https://docs.ifcopenshell.org/autoapi/ifcopenshell/validate/index.html), [IfcTester](https://docs.ifcopenshell.org/ifctester.html), [buildingSMART IDS](https://github.com/buildingSMART/IDS), [buildingSMART Validation Service en normatieve regels](https://github.com/buildingSMART/validate).
