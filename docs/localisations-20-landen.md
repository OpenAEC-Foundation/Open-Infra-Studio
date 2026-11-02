# Twintig aanvullende lokalisaties

Onderzoeksdatum: **31 oktober 2026**. Dit zijn 20 landen naast Nederland en de Verenigde Staten, met 74 afzonderlijke primaire bronverwijzingen. De landen zijn als gestructureerde onderzoeksprofielen opgeslagen; ze zijn nog geen 20 geactiveerde landmodules. De huidige toepassing ondersteunt de basisprofielen Nederland, Verenigde Staten en Internationaal.

Per land zijn officiële data-ingangen, horizontale coördinatenreferenties, hoogtereferenties, eenheden, integratiestappen en onzekerheden vastgelegd. Alle nieuwe data-ingangen hebben `service_tested: false`: de bronnen zijn onderzocht, maar er zijn voor deze 20 landen geen volledige service- of CAD-importproeven uitgevoerd. De bestaande catalogustellingen hieronder zijn aantallen vermeldingen in `assets/servers/world.json`, geen aantallen opnieuw geteste diensten.

## Overzicht

| Land | Officiële ingang | Horizontale referentie, per project te kiezen | Hoogtereferentie | Catalogusvermeldingen |
| --- | --- | --- | --- | ---: |
| [Australië (AU)](../assets/localisations/research/AU.json) | [ELVIS — Elevation and Depth](https://elevation.fsdf.org.au/) | GDA2020, geografisch 2D; MGA-zone per project selecteren (EPSG:7844) | [Australian Height Datum (AHD); AVWS afzonderlijk behandelen](https://www.anzlic.gov.au/sites/default/files/files/GDA2020%20Technical%20Manual%20V1.8_published.pdf) | 49 |
| [België (BE)](../assets/localisations/research/BE.json) | [Geo.be — federaal geoportaal](https://www.geo.be/) | ETRS89 / Belgian Lambert 2008 (EPSG:3812); BD72 / Belgian Lambert 72 — bestaande gegevens (EPSG:31370) | [Tweede Algemene Waterpassing (TAW)](https://ngi.be/aanbod/dienstverlening/geodetische-diensten/belgische-coordinatensystemen/overgang-naar-lambert-2008/) | 34 |
| [Brazilië (BR)](../assets/localisations/research/BR.json) | [INDE — Infraestrutura Nacional de Dados Espaciais](https://inde.gov.br/) | SIRGAS2000, geografisch; UTM-zone en hemisfeer per project bepalen (EPSG:4674) | [Datum Vertical Brasileiro — Imbituba en Santana](https://www.ibge.gov.br/geociencias/informacoes-sobre-posicionamento-geodesico/rede-geodesica/10842-rmpg-rede-maregrafica-permanente-para-geodesia.html) | 18 |
| [Canada (CA)](../assets/localisations/research/CA.json) | [GEO.ca](https://geo.ca/getting-started/) | NAD83(CSRS), realisatie en epoch per bron; projectie per provincie en project | [Canadian Geodetic Vertical Datum of 2013 (CGVD2013)](https://natural-resources.canada.ca/science-data/science-research/geomatics/geodetic-reference-systems/canadian-spatial-reference-system-csrs) | 14 |
| [Denemarken (DK)](../assets/localisations/research/DK.json) | [Dataforsyningen — Danish Data and Map Supply](https://dataforsyningen.dk/) | ETRS89 / UTM zone 32N (EPSG:25832); ETRS89 / UTM zone 33N — onder meer Bornholm (EPSG:25833); ETRS89 / DKTM zone 1 (EPSG:4093); ETRS89 / DKTM zone 2 (EPSG:4094); ETRS89 / DKTM zone 3 (EPSG:4095); ETRS89 / DKTM zone 4 (EPSG:4096) | [DVR90 — Danish Vertical Reference 1990](https://www.eng.klimadatastyrelsen.dk/mapping/geodesy-and-coordinate-systems) | 8 |
| [Duitsland (DE)](../assets/localisations/research/DE.json) | [BKG — Geodatenzentrum](https://gdz.bkg.bund.de/) | ETRS89 / UTM zone 32N (EPSG:25832); ETRS89 / UTM zone 33N (EPSG:25833) | [DHHN2016 — Normalhöhennull (NHN)](https://geodatenportal.sachsen-anhalt.de/gfds/de/datei/download/id/4016%2C501/afis_broschuere.pdf) | 119 |
| [Finland (FI)](../assets/localisations/research/FI.json) | [National Land Survey of Finland - MapSite](https://www.maanmittauslaitos.fi/en/e-services/mapsite) | EUREF-FIN / ETRS-TM35FIN (E,N) (EPSG:3067) | [N2000](https://www.maanmittauslaitos.fi/en/maps-and-spatial-data/datasets-and-interfaces/product-descriptions/elevation-model-2-m) | 17 |
| [Frankrijk (FR)](../assets/localisations/research/FR.json) | [IGN Geoservices / Geoplateforme](https://geoservices.ign.fr/) | RGF93 v1 / Lambert-93; daadwerkelijke realisatie per dataset controleren (EPSG:2154) | [NGF-IGN69 op het vasteland; NGF-IGN78 op Corsica](https://geoservices.ign.fr/sites/default/files/2021-07/DC_Ref3DNat_1-0-2.pdf) | 25 |
| [Ierland (IE)](../assets/localisations/research/IE.json) | [GeoHive — nationale geodatahub](https://www.geohive.ie/) | ETRS89 / Irish Transverse Mercator (ITM) (EPSG:2157) | [Ordnance Datum Malin Head](https://data.gov.ie/dataset/open-topographic-lidar-data) | 3 |
| [Italië (IT)](../assets/localisations/research/IT.json) | [RNDT - Repertorio Nazionale dei Dati Territoriali](https://geodati.gov.it/geoportale/) | RDN2008 / ETRF2000, epoche 2008.0; projectie per regio/dataset kiezen | [Genova 1942 voor het beschreven IGMI VMAP-product; overige varianten nog te verifieren](https://igmi.esercito.difesa.it/en/product-description/digital-maps/vmap-level-1/) | 10 |
| [Japan (JP)](../assets/localisations/research/JP.json) | [GSI — Fundamental Geospatial Data](https://www.gsi.go.jp/kiban/) | Japanese Geodetic Datum 2024 (JGD2024); vlakke rechthoekige zone per locatie kiezen | [Geoid-based vertical datum, JPGEO2024; referentie gemiddeld zeeniveau Tokyo Bay](https://www.gsi.go.jp/common/000278468.pdf) | 5 |
| [Nieuw-Zeeland (NZ)](../assets/localisations/research/NZ.json) | [LINZ Data Service](https://data.linz.govt.nz/) | NZGD2000 / New Zealand Transverse Mercator 2000 (NZTM2000) (EPSG:2193); NZGD2000, geografisch (EPSG:4167) | [New Zealand Vertical Datum 2016 (NZVD2016)](https://www.linz.govt.nz/guidance/geodetic-system/coordinate-systems-used-new-zealand/vertical-datums/new-zealand-vertical-datum-2016-nzvd2016) | 4 |
| [Noorwegen (NO)](../assets/localisations/research/NO.json) | [Geonorge — officiële nationale geodatacatalogus](https://www.geonorge.no/) | EUREF89 / UTM zone 32N (EPSG:25832); EUREF89 / UTM zone 33N (EPSG:25833); EUREF89 / UTM zone 35N (EPSG:25835) | [NN2000 — Normal Null 2000](https://www.kartverket.no/til-lands/posisjon/ta-i-bruk-nn2000) | 15 |
| [Oostenrijk (AT)](../assets/localisations/research/AT.json) | [BEV Open Data](https://data.bev.gv.at/) | ETRS89 / Austria Lambert (EPSG:3416); MGI / Austria Lambert (EPSG:31287) | [MGI-gebruikshoogten, Adria 1875; EVRF2000 Austria height afzonderlijk](https://www.bev.gv.at/Services/Produkte/Grundlagenvermessung/Hoehen-Grid.html) | 27 |
| [Polen (PL)](../assets/localisations/research/PL.json) | [GUGiK Geoportal](https://www.geoportal.gov.pl/) | PUWG 1992 (EPSG:2180) | [PL-EVRF2007-NH; PL-KRON86-NH als afzonderlijke bronvariant](https://www.geoportal.gov.pl/pl/dane/numeryczny-model-terenu-nmt/) | 9 |
| [Singapore (SG)](../assets/localisations/research/SG.json) | [OneMap — Singapore Land Authority](https://www.onemap.gov.sg/) | SVY21 / Singapore TM (EPSG:3414) | [Singapore Height Datum (SHD)](https://app.sla.gov.sg/sirent/About/SGEOID09) | 3 |
| [Spanje (ES)](../assets/localisations/research/ES.json) | [CNIG Centro de Descargas](https://centrodedescargas.cnig.es/CentroDescargas/) | ETRS89 / UTM zone 29N (EPSG:25829); ETRS89 / UTM zone 30N (EPSG:25830); ETRS89 / UTM zone 31N (EPSG:25831); REGCAN95 / UTM zone 28N voor de Canarische Eilanden (EPSG:4083) | [REDNAP / datum Alicante; eiland- en datasetvariant controleren](https://www.ign.es/web/calculadora-geodesica) | 22 |
| [Verenigd Koninkrijk (GB)](../assets/localisations/research/GB.json) | [Ordnance Survey Data Hub — Groot-Brittannië](https://osdatahub.os.uk/) | OSGB36 / British National Grid — Engeland, Schotland en Wales (EPSG:27700); ETRS89 / Irish Transverse Mercator — Noord-Ierland (EPSG:2157) | [Ordnance Datum Newlyn (ODN); regionale en eilanddatums](https://www.gov.uk/government/publications/uk-geospatial-data-standards-register/national-geospatial-data-standards-register) | 14 |
| [Zweden (SE)](../assets/localisations/research/SE.json) | [Geotorget — Lantmäteriet](https://geotorget.lantmateriet.se/) | SWEREF 99 TM (EPSG:3006); SWEREF 99 12 00 — voorbeeld lokale zone (EPSG:3007); SWEREF 99 18 00 — voorbeeld lokale zone (EPSG:3011) | [RH 2000](https://www.lantmateriet.se/sv/geodata/gps-geodesi-och-swepos/Referenssystem/) | 9 |
| [Zwitserland (CH)](../assets/localisations/research/CH.json) | [geo.admin.ch](https://map.geo.admin.ch/) | CH1903+ / LV95 (EPSG:2056) | [LN02; LHN95 als afzonderlijk hoogtereferentiekader](https://www.swisstopo.admin.ch/de/lokale-bezugsrahmen-schweiz) | 24 |

## Wat nog nodig is voor implementatie

Een geverifieerd geografisch datum of een EPSG-code is geen volledige projectinstelling. Geografische CRS’en gebruiken graden; een civiele tekenprojectie vereist een geschikte metrische projectie, zone en asvolgorde. Houd horizontale en verticale referenties apart. Lokale varianten en oudere bronbestanden blijven expliciet herkenbaar.

Voor alle twintig landen zijn de volledige civiele CAD-laag-/symboolstandaarden nog niet geverifieerd. De profielen eisen daarom een projectspecifieke keuze en claimen geen nationale conformiteit. Een volgende implementatiestap moet per land ten minste een echte kaart-/vector-/terreinimport, transformatie met controlepunten, eenhedencontrole en een overeengekomen teken-/oplevervoorbeeld toetsen. Ook toegang, licentie en actualiteit worden per dataset vastgesteld.

## Aandachtspunten per land

### Australië — AU

EPSG:7844 is geografisch en niet de vlakke tekenprojectie. Deelstaatkadasters, infrastructuurdata en leveringsafspraken vergen aparte selectie. ELVIS-dekking en ouderdom verschillen per dataset.

**Volgende stap:** Selecteer deelstaat en MGA2020-zone, haal een ELVIS-dataset op en verifieer bronprojectie, hoogtedatum, resolutie en transformatie.

[Volledig profiel en bronnen](../assets/localisations/research/AU.json).

### België — BE

Federale en regionale bronnen afzonderlijk selecteren; Vlaanderen is geen dekking voor heel België. NGI meldt versieafhankelijke problemen met het Lambert72/Lambert2008-transformatierooster; gebruikte CRS-database en rooster verifiëren. Niet alle NGI-producten zijn open data; licentie per dataset controleren. Alleen bronnen onderzocht; services en import niet getest.

**Volgende stap:** Begin met een Lambert 2008-projectprofiel, behoud expliciete importkeuze voor Lambert 72 en controleer transformatieroosters met NGI-controlepunten.

[Volledig profiel en bronnen](../assets/localisations/research/BE.json).

### Brazilië — BR

EPSG:4674 is geografisch en geen UTM-projectie. INDE is een federatie: beschikbaarheid, schaal, toegang en actualiteit verschillen per beheerder. Gemeentelijke kaarten zijn geen bewijs van landelijke perceel- of gedetailleerde DEM-dekking.

**Volgende stap:** Selecteer gemeente/deelstaat, SIRGAS2000-UTM-zone en hoogtebron; haal concrete laagmetadata en service-URL uit INDE voordat een koppeling wordt ingericht.

[Volledig profiel en bronnen](../assets/localisations/research/BR.json).

### Canada — CA

EPSG-code niet vastgezet: bronrealisatie en regionale projectie moeten eerst worden gekozen. Hoogteresolutie, kadastertoegang en licentie verschillen per provincie; catalogusverwijzingen zijn geen geteste service-endpoints.

**Volgende stap:** Kies provincie, NAD83(CSRS)-realisatie en epoch; selecteer lokale projectie en controleer metadata van een CanElevation-tegel voor import.

[Volledig profiel en bronnen](../assets/localisations/research/CA.json).

### Denemarken — DK

UTM32 wordt ook oostelijk van de formele zonegrens gebruikt; Bornholm gebruikt vaak UTM33. Oude System34/System45-data vereisen specialistische conversie. Deze preset betreft Europees Denemarken; Groenland/Faeröer afzonderlijk onderzoeken. Registratie, token/API-sleutel en productvoorwaarden voor actuele diensten controleren. WMS alleen uit documentatie opgenomen, niet aangeroepen.

**Volgende stap:** Kies voor bouw- en infraprojecten een passende DKTM-zone volgens de officiële aanbeveling; gebruik UTM voor passende landelijke bronnen. Test DHM en autorisatie van Datafordeler.

[Volledig profiel en bronnen](../assets/localisations/research/DK.json).

### Duitsland — DE

BKG biedt federale producten; grootschalige Länder-producten hebben eigen productspecificaties en voorwaarden. Hessen is een regionale startbron. Gedetailleerde kadaster- en hoogtedata, toegangsvoorwaarden en services verschillen per deelstaat. DGM200 is een grof nationaal model en geen ontwerpsurvey. ETRS89-realisatie/epoch en eventuele zoneprefix uit bronmetadata behouden; oude Gauss-Krüger-data vereisen transformatie. Geen service- of importtest uitgevoerd.

**Volgende stap:** Selecteer eerst deelstaat en projectgebied; configureer UTM-zone en hoogtereferentie, en test een regionale DGM- en gebouwimport.

[Volledig profiel en bronnen](../assets/localisations/research/DE.json).

### Finland — FI

De primaire CRS-lijst vermeldt recente EUREF-FIN-codeactualisaties; bibliotheekondersteuning en asvolgorde controleren. De 2 m terreinresolutie garandeert geen centimeternauwkeurigheid en dekking kent uitzonderingen. Geen API-aanroep of download getest.

**Volgende stap:** Test de OGC API Processes-bestandsdienst, autorisatievoorwaarden en een 2 m terreinbestand; registreer N2000, kwaliteitsklasse en bronjaar.

[Volledig profiel en bronnen](../assets/localisations/research/FI.json).

### Frankrijk — FR

IGN beschrijft in mei/juni 2026 voorstellen voor wijziging van EPSG-gebruik. Code 2154 is volgens die bron strikt v1, terwijl veel datasets v2b gebruiken; geen uitwisselbare precisie aannemen. CC-zones en overzeese referenties vereisen aparte keuze. Geen serviceverzoeken getest.

**Volgende stap:** Kies een RGE-dataset en controleer actuele service, licentie, realisatie van RGF93, projectie en hoogtereferentie voor import.

[Volledig profiel en bronnen](../assets/localisations/research/FR.json).

### Ierland — IE

GeoHive meldt dat sommige gehoste hubs vanaf september 2026 verdwijnen of migreren. LiDAR-dekking en meetjaar per gebied controleren. Kadaster- en professionele topografische producten niet automatisch als open data behandelen. Bestaande Irish Grid-data vragen een bewuste conversie via officiële geodetische voorzieningen; services niet getest.

**Volgende stap:** Zoek per project beschikbare LiDAR-tegels en test ITM-import met expliciete Malin Head-hoogtereferentie. Controleer de actuele GeoHive-doorverwijzingen.

[Volledig profiel en bronnen](../assets/localisations/research/IE.json).

### Italië — IT

RNDT is een metadataregister: toegang en actualiteit hangen af van de gegevensbeheerder. IGMI beschrijft het referentiekader, maar een exacte projectiecode is hier niet op primaire bron bevestigd; daarom epsg null. IGMI-pagina's waren via webindex leesbaar, direct openen gaf deels 403. Geen services getest.

**Volgende stap:** Zoek in RNDT op regionale topografische databases, gebouwen, kadaster en terreinmodellen; verifieer leverende instantie, downloadrechten, UTM-zone, exact EPSG-nummer en hoogtevariant.

[Volledig profiel en bronnen](../assets/localisations/research/IT.json).

### Japan — JP

EPSG-code voor de gekozen JGD2024-variant nog niet met primaire bron bevestigd. Download-URL is in april 2025 gewijzigd; GSI vereist registratie en login. DEM-resolutie en actualiteit variëren per gebied.

**Volgende stap:** Kies projectgebied en vlakke zone; controleer datum, versie en hoogteherziening van een GSI-download, maak een importpad voor basisobjecten en DEM.

[Volledig profiel en bronnen](../assets/localisations/research/JP.json).

### Nieuw-Zeeland — NZ

LDS-assenvolgorde verschilt per WFS-versie. Offshore-eilanden hebben eigen projecties. Property Boundaries combineert verschillende bronsoorten en vervangt geen terreinmeting; toegangseisen en dekking per laag controleren.

**Volgende stap:** Begin met NZTM2000 en NZVD2016; selecteer LINZ-laag, API-toegang en assenvolgorde, en controleer een terrein- en perceelimport.

[Volledig profiel en bronnen](../assets/localisations/research/NZ.json).

### Noorwegen — NO

FKB gebruikt lokaal zone32 in zuidelijk/midden Noorwegen, zone33 in Nordland/Troms en zone35 in Finnmark; landelijke distributie vaak zone33. Productlicentie en GeoID-toegang kunnen verschillen. NN1954 niet zonder transformatie combineren. Svalbard/Jan Mayen apart onderzoeken. Geen download of service getest.

**Volgende stap:** Selecteer lokale UTM-zone en NN2000, lees SOSI/GML-metadata en test een FKB-terrein/gebouwimport. Beoordeel producttoegang in Geonorge.

[Volledig profiel en bronnen](../assets/localisations/research/NO.json).

### Oostenrijk — AT

Lambert-opties zijn voor landelijke kaartproducten bevestigd; kadaster gebruikt eigen landmeetkundige stroken M28/M31/M34. Stichtagsdata zijn momentopnamen; actuele eigendomsrelevante informatie vraagt andere levering. Geen download of serviceverzoek getest.

**Volgende stap:** Selecteer BEV-download en bron-CRS; verifieer lokale Gauß-Kruger-strook voor kadastrale gegevens en de vereiste hoogtetransformatie.

[Volledig profiel en bronnen](../assets/localisations/research/AT.json).

### Polen — PL

2180 is bevestigd voor de landelijke ULDK-uitwisseling; dat bewijst niet dat alle lokale ontwerp- en kadasterbestanden die projectie gebruiken. EGiB-WMS geeft kaartbeelden; beschikbare WFS- en downloadbronnen verschillen. Geen service of download getest.

**Volgende stap:** Importeer een NMT-tegel met expliciete hoogtevariant en onderzoek EGiB-vectorlevering per powiat; verifieer gedetailleerde bronprojectie voordat geometrie wordt gecombineerd.

[Volledig profiel en bronnen](../assets/localisations/research/PL.json).

### Singapore — SG

OneMap is een toegangspunt voor kaartdata; dit bewijst geen vrije toegang tot alle kadastrale grenzen, nutsdata of gedetailleerde hoogtegegevens. API-authenticatie, voorwaarden en bronprecisie per dataset beoordelen.

**Volgende stap:** Configureer SVY21 met expliciete Easting/Northing-volgorde, controleer OneMap-toegang en valideer een puntconversie en SHD-hoogtebron.

[Volledig profiel en bronnen](../assets/localisations/research/SG.json).

### Spanje — ES

ETRS89 geldt voor schiereiland/Balearen; Canarias gebruikt REGCAN95. IGN vermeldt ook REGCAN95 zone 27N; deze configuratie beperkt zich tot de bevestigde zone 28N-optie. De nieuwste terreinresolutie is niet overal dezelfde. Geen service of download getest.

**Volgende stap:** Kies UTM-zone en datum op projectlocatie, download een PNOA-tegel en controleer dekking, opnamejaar, hoogtevariant en productkwaliteit.

[Volledig profiel en bronnen](../assets/localisations/research/ES.json).

### Verenigd Koninkrijk — GB

OS Terrain 50 dekt Groot-Brittannië en heeft 50 m rasterafstand en 10 m hoogtelijnen; geschikt voor regionale context, geen gedetailleerde civiele ontwerpondergrond. Noord-Ierland vereist eigen bronkeuze. OSGB36/ETRS89-transformatie met officiële roosters beoordelen; overige datasetlicenties en datums apart controleren. Services niet getest.

**Volgende stap:** Maak aparte profielen voor Groot-Brittannië en Noord-Ierland; test eerst een Terrain 50-download met expliciete horizontale en verticale metadata.

[Volledig profiel en bronnen](../assets/localisations/research/GB.json).

### Zweden — SE

Er bestaan twaalf lokale projectiezones (EPSG:3007–3018); hier zijn alleen twee voorbeelden opgenomen. SWEREF99 niet op meetnauwkeurigheid gelijkstellen aan actuele WGS84-posities. Gratis producten kunnen een account, productautorisatie en specifieke voorwaarden vereisen; sommige vastgoedgegevens vragen doeltoetsing. Services en downloads niet getest.

**Volgende stap:** Start met SWEREF99 TM en RH2000, voeg de voor het project vereiste lokale SWEREF-zone toe en test een Geotorget-terrein- en gebouwproduct.

[Volledig profiel en bronnen](../assets/localisations/research/SE.json).

### Zwitserland — CH

Een WMS levert kaartbeelden. Het 10 m swissALTIRegio-model is een orientatiebron; resolutie is geen gegarandeerde ontwerpmeetnauwkeurigheid. Buitenlandse hoogten en oudere LV03-data vereisen expliciete transformatie. Geen serviceverzoeken getest.

**Volgende stap:** Test WMS-capabilities en kies een gedetailleerd terreinproduct en eventuele kantonale kadastrale bron. Bewaar LV95 en de hoogtevariant in projectmetadata.

[Volledig profiel en bronnen](../assets/localisations/research/CH.json).

## Bestandsopzet

Het [indexbestand](../assets/localisations/research/index.json) verwijst naar precies 20 landbestanden. Elk bestand bevat `portal`, `data`, `horizontal_crs`, `vertical_reference`, `units`, `drawing_standard`, `integration` en `sources`, plus de onderzoeksdatum en expliciete status. Een ontbrekende bevestigde EPSG-code of normbron blijft `null`. De bestaande serviceaantallen zijn uit de repositorycatalogus afgeleid.

De implementatie van de eerste regionale profielen en het afzonderlijke Amerikaanse onderzoek staan in [Localisaties](localisations.md).
