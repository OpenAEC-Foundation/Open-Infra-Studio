# Ruimtelijke plannen

Open Infra ondersteunt bestaande ruimtelijke plannen in het IMRO-domein via de Ruimtelijke Plannen API v4 en lokale IMRO-GML. Daaronder vallen ook TAM-IMRO-plannen wanneer de bron deze aanbiedt. Dit is geen volledige ontsluiting van moderne omgevingsdocumenten en regels in het DSO. Controleer voor actuele juridische informatie het officiële Omgevingsloket, Regels op de kaart, en de bijbehorende brondocumenten.

## API en sleutel

De API-basis is `https://ruimte.omgevingswet.overheid.nl/ruimtelijke-plannen/api/opvragen/v4`. Hiervoor is een eigen Ruimtelijke Plannen API-sleutel nodig. Geef deze mee via de omgevingsvariabele `OPENINFRA_RP_API_KEY` vóór het starten van de host, of vul hem tijdelijk in het sleutelveld van de gebruikersinterface in. Een via de interface ingevoerde sleutel blijft alleen tijdens de sessie in het geheugen; sla hem niet op in projectbestanden, instellingen of broncode.

Zoeken gebeurt op een RD-punt; de client vraagt geometrie in EPSG:28992 op. Een gekozen plan levert de plancontour en ondersteunde objectcollecties, waaronder bestemmingsvlakken, bouwvlakken en aanduidingen. Niet elk gepubliceerd plan is noodzakelijk beschikbaar via de API. Bewaar plan-ID en bronverwijzingen bij controle en export.

## Lokale GML

Een zelfstandig lokaal IMRO-GML-bestand kan zonder API-sleutel worden ingelezen. Interne geometrieverwijzingen worden binnen het bestand opgelost; externe verwijzingen worden niet automatisch gedownload. Lever bij ontbrekende verwijzingen een zelfstandig bestand aan. Controleer het opgegeven coördinatenstelsel en de plaatsing ten opzichte van bekende RD-punten.

Open **Ruimtelijke plannen** via de GIS-werkbank of `GIS_PLANNEN`. Stel een projectlocatie in, voer eventueel de sessiesleutel in en kies **Plannen zoeken**. Selecteer een resultaat, haal het op en plaats het daarna in de tekening. Voor lokale import: download de GML bij het plan onder **Overig** in Regels op de kaart, vul het volledige lokale pad in en kies **GML in tekening plaatsen**. De algemene bestandsimport herkent IMRO eveneens.

De import ondersteunt polygonen met binnenringen, meerdere geometrieën, punten en gesegmenteerde lijnen. Gebogen GML-segmenten en omgekeerd georiënteerde segmenten geven een expliciete fout: segmenteer die eerst in de bron. Ze worden niet stilzwijgend als rechte lijnen getekend. IMRO wordt als volledige planobjecten ingevoerd; afknippen via de algemene bestandsoptie is niet ondersteund.

Beschikbare opdrachten en MCP-tools:

| Opdracht | MCP-tool | Gebruik |
| --- | --- | --- |
| `GIS_PLAN_SEARCH X=100000 Y=400000` | `planning_search` | Zoekt op een RD-punt; zonder punt op de projectlocatie |
| `GIS_PLAN_IMPORT PLAN=<plan-ID>` | `planning_import` | Haalt een gekozen plan op en plaatst de geometrie |
| `GIS_PLAN_GML "C:/data/plan.gml"` | `planning_gml` | Importeert lokaal zonder sleutel |

Een sleutel hoort uitsluitend in het sessieveld of de lokale procesomgeving, nooit in een opdrachtargument.

## Kaartbeeld via PDOK

De ingebouwde WMS-server **PDOK Ruimtelijke plannen (enkelbestemming)** gebruikt `https://service.pdok.nl/kadaster/plu/wms/v1_0`, met standaardlaag `enkelbestemming`. Deze openbare kaartdienst vereist geen Ruimtelijke Plannen API-sleutel. Een WMS levert een kaartbeeld; gebruik de API of lokale GML wanneer objectgeometrie en eigenschappen nodig zijn.

PDOK vermeldt een halfjaarlijkse actualisatie van deze dataset. Het WMS-kaartbeeld kan daardoor achterlopen op de actuele bron. De WMS en de API zijn afzonderlijke diensten; beschikbaarheid van de ene bewijst niets over de actualiteit of volledigheid van de andere.

## Metadata en weergave

Planstatus, statusdatum, eventuele einddatum van rechtsgeldigheid en historische/TAM-markeringen zijn bronmetadata. Ze vormen op zichzelf geen uitspraak dat een plan op een locatie juridisch van toepassing is. Ook een status “vastgesteld” bewijst dat niet. Controleer de samenhang met andere documenten en de actuele officiële publicatie.

Kleuren, vullingen, arceringen en lijndikten zijn een indicatieve kaartweergave op basis van objecttype en bestemmingshoofdgroep. Ze zijn geen volledige officiële verbeelding of gecertificeerde stijl. Onbekende groepen krijgen een algemene stijl; raadpleeg legenda, planteksten en bronkaart voor de betekenis.

IMRO-kaartlagen houden een eigen `IMRO_`-naam: planologische kaartobjecten krijgen geen onjuiste NLCS-ontwerpclassificatie. Objectmetadata bevat identificaties, status, bron en de volledige samengestelde attributen, verdeeld over UTF-8-veilige Xdata-velden waar dat nodig is. Enkelbestemmingen krijgen een effen vulling, dubbelbestemmingen een arcering en bouwvlakken een zwaardere omtrek. Binnenringen blijven uitsparingen.

## Verificatie

Gecontroleerd op Windows met Rust `1.99.1-x86_64-pc-windows-msvc`:

- `cargo test -p open-infra`: 161 geslaagde tests; 10 bestaande online/lokale-bestandstests overgeslagen.
- `cargo test -p open-infra-civiel`, afzonderlijke builds van beide plugins en 4 Python MCP-tests geslaagd.
- API-contracttests met een lokale HTTP-server: RD-headers, POST-zoekgeometrie, paginering vanaf pagina 0, alle objectcollecties, foutafhandeling en sleutelafscherming.
- `cargo run -p open-infra --example planning_map -- target/planning-final`: synthetisch plan met 10 elementen, DXF- en DWG-rondreis gecontroleerd.
- Geopend en visueel gecontroleerd in Open CAD Studio 2026.43, build `dbd50b20091c`, bridge-schema `709b35b1fc3fea05b8041b07723e3abfd2eea6fea29030a33e0859db5ceef15b`. Native audit en geverifieerd opslaan naar DWG AC1032: 0 fouten, 0 waarschuwingen, 0 vervallen objecten, 10 elementen behouden.

De geauthenticeerde overheidsdienst en het nieuwe werkbankpaneel zijn niet integraal live getest. De publieke WMS-capabilities en standaardlaag zijn wel live gecontroleerd. De lokale GML-parser, renderer en sleutel-/HTTP-afhandeling hebben geautomatiseerde regressietests. Labelplaatsing gebruikt een begrensde ontwijking; bij zeer drukke kaarten kan handmatige verplaatsing nodig blijven.

## Bronnen

- [Ruimtelijke plannen opvragen, API v4 en sleutelvoorwaarden](https://developer.omgevingswet.overheid.nl/api-register/api/rp-opvragen/).
- [PDOK Ruimtelijke plannen: WMS en halfjaarlijkse actualisatie](https://www.pdok.nl/ogc-webservices/-/article/ruimtelijke-plannen).
- [GetCapabilities van de gebruikte WMS](https://service.pdok.nl/kadaster/plu/wms/v1_0?service=WMS&request=GetCapabilities).
