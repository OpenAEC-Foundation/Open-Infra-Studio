# Localisaties en internationaal onderzoek

Onderzocht op 31 oktober 2026. Eerste regionale profielen: Nederland (`NL`), Verenigde Staten (`US`) en Internationaal (`international`).

## Vier afzonderlijke keuzes

| Keuze | Betekenis | Opslag |
| --- | --- | --- |
| Interfacetaal | Nederlands of Engels; onafhankelijk van het land van het project | `language` in instellingen |
| Standaardregio | Landgebonden lintgroepen en uitgangspunt voor nieuwe projecten | `region` in instellingen |
| Projectregio | Regionale navigatie en hulp bij databronnen; verandert geen geometrie | `region` in het Site-record van de tekening |
| Technische referentie | Werkelijk project-CRS, tekeneenheden en overeengekomen hoogtereferentie | Bestaande project-/tekeninggegevens; expliciet instellen |

Via **Localisatie** of `GIS_LOCALISATION` / `GIS_LOCALISATIE` kiest u de standaardregio en, na het opslaan van een projectlocatie, de regio van de tekening. Wijzigingen aan het lint gelden na herstart. De projectregio past de werkbanknavigatie direct aan. Een Nederlandstalige gebruiker kan dus een Amerikaans project hebben zonder de interfacetaal te veranderen.

Oude instellingen met `netherlands: false` worden Internationaal; anders blijft het oude Nederland-profiel behouden. Een expliciete `region` heeft voorrang. Oude projectrecords zonder regio blijven Nederland. Onbekende toekomstige regio's vallen terug op Internationaal en verliezen hun coördinaten niet. Geografie wordt nooit gebruikt als stilzwijgende vervanging van de projectkeuze.

## Wat de profielen aanbieden

| Profiel | Data en bediening | Standaarden |
| --- | --- | --- |
| Nederland | Eigen groep voor PDOK/BAG/NWB, BGT, BRK, KLIC, AHN, BRO, IMRO en NLCS | Bestaande NLCS-functionaliteit |
| Verenigde Staten | Eigen groep met USGS-topografie, 3DEP-hoogtekaart, Census-grenzen, NRCS-bodemkaart en ingang naar de Amerikaanse servicecatalogus | Projectspecifieke NCS-, A/E/C- of State DOT-eisen; geen geïmplementeerde Amerikaanse conformiteitscontrole |
| Internationaal | Wereldwijde/eigen WMS, WFS, XYZ, OpenStreetMap en open bestandsformaten | Projectspecifiek; geen automatische nationale laagmapping |

De algemene functies voor coördinaten, bestanden, IFC en geometrie blijven beschikbaar. Nederlandse datasets worden niet als Amerikaanse equivalenten gepresenteerd. Civiele ontwerptools en de infra-objectbibliotheek produceren momenteel NLCS-lagen, ook wanneer een ander landprofiel is gekozen; de werkbank vermeldt dit bij deze functies. Voor internationale oplevering is een afzonderlijke laag-/objectmapping nodig. De bestaande delen van de interface zijn nog niet allemaal vertaald; de gedeelde navigatie en nieuwe lokalisatiefuncties ondersteunen beide interfacetalen.

De servicebrowser heeft een expliciet regiofilter, naast categorie, protocol en geografische dekking. NL en US nemen ook bronnen met `GLOBAL` mee. Internationaal en Alle regio's tonen alle landen, niet uitsluitend wereldwijd dekkende bronnen. Een regionale ingang schakelt het locatie-filter uit, zodat bijvoorbeeld een Amerikaanse bron niet onzichtbaar wordt door een nog Nederlandse kaartpositie. Voor import moet u de projectlocatie en brondekking controleren.

Voor opdrachten/MCP: `GIS_SERVICES COUNTRY=US;GLOBAL`, `gis_services(country="US;GLOBAL")`. Directe rasteropdrachten: `GIS_US_TOPO`, `GIS_US_ELEVATION`, `GIS_US_BOUNDARIES`, `GIS_US_SOILS`. `GIS_US_SERVICES` opent de gefilterde catalogus.

## Amerikaans onderzoek

De bestaande catalogus bevat 24 Amerikaanse services, waaronder federale datasets en lokale bronnen uit Massachusetts, New York, North Carolina en Los Angeles County. Het landprofiel maakt deze bronnen bereikbaar; het veronderstelt geen landelijk equivalent voor ieder Nederlands register.

### Federale en lokale data

| Onderwerp | Primaire bron | Gebruik en begrenzing |
| --- | --- | --- |
| Hoogte en lidar | [USGS 3DEP](https://www.usgs.gov/3d-elevation-program), [The National Map-services](https://apps.nationalmap.gov/services/) | Hoogtekaarten en beschikbare terrein-/puntenwolkproducten. De eerste snelknop importeert een hillshade-raster, geen nieuw terreinmodel of lidar-downloadadapter. |
| Topografie en transport | [USGS The National Map](https://apps.nationalmap.gov/services/) | Federale kaartlagen. Beschikbare protocols en productdekking verschillen per service. |
| Bestuurlijke grenzen | [Census TIGERweb](https://tigerweb.geo.census.gov/) | Grenzen en Census-geografie via WMS/REST. Dit vervangt geen kadaster, perceeleigendomsregistratie of ontwerpmeting. |
| Bodem | [USDA NRCS Soil Data Access](https://sdmdataaccess.nrcs.usda.gov/WebServiceHelp.aspx) | Bodemkaart via WMS/WFS en tabulaire queryservices. De snelknop gebruikt de kaart; tabulaire interpretaties zijn nog geen eigen applicatie-adapter. |
| Percelen en gebouwen | Staats-/county-bronnen in de catalogus, bijvoorbeeld [MassGIS assessors' parcels](https://www.mass.gov/info-details/massgis-data-property-tax-parcels) | Bronhouder, actualiteit, attributen en gebruiksdoel per regio beoordelen. Een belastingperceelkaart is geen vervanging van een grensreconstructie. |
| Overstromingsinformatie | [FEMA NFHL](https://www.fema.gov/flood-maps/national-flood-hazard-layer) | Onderzocht als uitbreiding. De geteste dienst verbrak de verbinding; niet toegevoegd als nieuwe werkende snelkoppeling. |

Op de onderzoeksdatum zijn capabilities én de ingestelde standaardlaagnamen van deze vier WMS-diensten live gecontroleerd:

- USGS topografie: `USGSTopo/MapServer/WMSServer`, laag `0`.
- USGS 3DEP: `3DEPElevation/ImageServer/WMSServer`, laag `3DEPElevation:Hillshade Multidirectional`.
- Census TIGERweb Current: lagen `67,55,49,11`.
- NRCS SDM: laag `mapunitpoly`.

Alle vier leverden daarnaast een geldige 128 × 128 PNG via WMS GetMap bij een testlocatie in Washington. Dit is een controle op dienst-, laag- en kaartresponsniveau, geen garantie op actualiteit, complete dekking of beschikbaarheid op iedere locatie. De overige 20 Amerikaanse catalogusservices zijn in deze wijziging niet allemaal opnieuw live gecontroleerd.

### Coördinaten, hoogten en eenheden

[NOAA NGS](https://www.ngs.noaa.gov/TOOLS/spc.html) behandelt State Plane als zones met een specifieke referentie, niet als één CRS voor de hele VS. Kies het horizontale datum, de realisatie, de zone en het verticale datum op basis van de project-/bronmetadata. Het Nederlandse NAP wordt niet op Amerikaanse hoogten toegepast. Deze implementatie voegt geen verticale datumtransformatie toe.

Volgens de actuele [NGS-bètasite](https://beta.ngs.noaa.gov/) zijn de gemoderniseerde NSRS-tools nog beschikbaar voor publieke evaluatie tot ten minste maart 2027. De [aankondiging van oktober 2026](https://www.ngs.noaa.gov/web/science_edu/webinar_series/overview-beta-release-nsrs-mod.shtml) bevestigt de recente bètarelease. Daarom wordt SPCS2022/NAPGD2022 niet automatisch als operationele projectreferentie ingesteld.

[NIST](https://www.nist.gov/pml/us-surveyfoot) onderscheidt:

- International foot: exact `0.3048 m`.
- Historische US survey foot: exact `1200/3937 m`; sinds 2023 alleen voor historische/legacy-toepassingen.

DWG `INSUNITS=2` en `INSUNITS=21` worden nu afzonderlijk naar tekeneenheden per meter omgerekend. Een tekening in survey feet blijft survey feet; een andere regio verandert de eenheden niet. Broncoördinaten in feet moeten door hun CRS-definitie worden geïnterpreteerd, niet door het landprofiel.

De bestaande civiele engine gebruikt metrische projectafstanden. Gebruik daarom voorlopig een **metrisch project-CRS**, eventueel met een tekening in feet. Voor een nieuw US-profiel begint de locatiepagina bij Washington in metrisch WGS84/UTM 18N; dit is een startpositie, geen officiële keuze voor ieder Amerikaans project. Geprojecteerde State Plane-CRS'en in feet als intern project-CRS vereisen nog een bredere eenhedenaudit voordat daarvoor volledige ondersteuning wordt geclaimd.

### Tekenstandaarden

Het publiek beschikbare [A/E/C CAD Standard Release 6.1-rapport](https://stg.wbdg.org/FFC/AECCAD/ERDC_ITL_TR-19-7_2019.pdf) is een Amerikaanse DoD-uitwerking, mede gebaseerd op NCS. Dat maakt het niet automatisch de oplevereis voor elke Amerikaanse opdrachtgever. Project-/State DOT-eisen, versie, disciplinecodes, laagstructuur, symbolen, tekst, lijnstijlen en sheets moeten afzonderlijk worden vastgelegd. Hier zijn geen volledige normteksten of commerciële normbibliotheken overgenomen.

Verdere implementatie kan op deze regio-opzet aansluiten: eerst een expliciet overeengekomen Amerikaans laag-/objectprofiel, daarna een controleerbare mapping en validatie met echte oplevervoorbeelden. Alleen NLCS-laagnamen naar Engels vertalen levert geen Amerikaanse conformiteit op.

## Verdere internationale uitbreiding

De internationale catalogus blijft beschikbaar voor onder meer België, Duitsland, het Verenigd Koninkrijk en andere landen. Nieuwe lokale profielen moeten hun bronhouders, dekking, CRS/hoogtereferenties, eenheden, tekenstandaard en gebruiksvoorwaarden afzonderlijk onderbouwen. Een taalkeuze of een WMS-vermelding geldt niet als bewijs van volledige landspecifieke ondersteuning.

Het [onderzoek naar twintig aanvullende landen](localisations-20-landen.md) bevat officiële bronnen, referentiestelsels en integratievoorwaarden. De afzonderlijke JSON-profielen hebben status `researched`; ze zijn niet automatisch als werkende landmodules ingeschakeld.

## Verificatie van de implementatie

Gecontroleerd met Rust `1.99.1-x86_64-pc-windows-msvc`: 168 Rust-tests geslaagd (10 bestaande online/lokale tests overgeslagen), 5 Python MCP-tests geslaagd en beide plugins afzonderlijk gebouwd. De regressietests dekken instellingenmigratie, taal/regio-onafhankelijkheid, projectrecord-rondreis zonder coördinatenwijziging, onbekende toekomstige regio's, survey-foot-conversie, regionale servicefilters en het wissen van een oude Nederlandse selectie bij openen van de Amerikaanse catalogus. Alle vier Amerikaanse snelknoppen verwijzen naar bestaande US-WMS-bronnen. Onafhankelijke reviewbevindingen rond GEODATA-adoptie en oude service-selecties zijn opgelost. Het nieuwe werkbankpaneel is niet integraal in een live pluginsessie bediend.
