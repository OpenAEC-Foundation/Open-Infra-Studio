# Eerste meting van de kaartcatalogus

Op 2 november 2026 tussen 19:07:10 en 19:08:43 UTC zijn alle 629 openbare catalogusvermeldingen vanaf aia09-mp50 gecontroleerd: de 600 wereldvermeldingen en 29 ingebouwde bronnen.

Uitkomst: **573 werkt, 17 traag, 39 kapot**. Er zijn 499 HTTP-verzoeken gedaan, inclusief redirects. Het volledige script eindigde met exitcode 0.

Deze meting gebruikte commit `6d18ae359083c942e03f6ee674194ed19c58d303`, Python 3.12.3 en curl 8.5.0, een time-out van acht seconden per verzoek en een grens van drie seconden voor traag. De latere aanvullingen aan de XML-foutafhandeling zijn offline gecontroleerd; de volledige live meting is niet herhaald.

| Bron | Werkt | Traag | Kapot |
|---|---:|---:|---:|
| Wereldcatalogus | 545 | 17 | 38 |
| Ingebouwde bronnen | 28 | 0 | 1 |

## Opvallende problemen

- Vijf vermeldingen geven HTTP 404: twee delen het Braziliaanse CADASTRO_TERRITORIAL-endpoint; daarnaast JRC Global Surface Water, LGRB GK50 en de ingebouwde HisGIS-kaart van 1832. Bij tegelbronnen gaat dit alleen over de gekozen proeftegel.
- Zes vermeldingen hebben een standaardlaag die niet in de ontvangen capabilities staat: twee Finse overstromingskaarten, twee Israëlische lagen en twee Uruguayaanse hoogtemodellen.
- De Brusselse Urbis-bronnen delen twee endpoints waarvan het antwoord niet als XML gelezen kon worden; dit raakt zes vermeldingen.
- Twaalf vermeldingen lopen tegen een time-out. Die uitkomst hangt ook af van de meetomgeving en de korte verbindingslimiet.
- Langzaamste geslaagde proef: USGS 3DEP, gedeeld door twee vermeldingen, 6,33 seconden.

‘Kapot’ betekent hier een mislukte proef. Dit is geen bewijs van een blijvende of wereldwijde storing. Er is niets uit de catalogus verwijderd. De precieze werkwijze en beperkingen staan in [de controlebeschrijving](../kaartcatalogus-gezondheid.md).

## Alle mislukte proeven

| ID | Land | Type | Seconden | Reden |
|---|---|---|---:|---|
| w-global-jrc-globalsurfacewater-occurrence-3857 | GLOBAL | xyz | 0.157 | HTTP 404 |
| w-at-t-tirol-orthofoto-31254 | AT | wms | 3.022 | time-out |
| w-at-t-tirol-dgm-hillshade-31254 | AT | wms | 3.022 | time-out |
| w-be-bru-urbis-ortho-31370 | BE | wms | 0.553 | Geen geldige capabilities-XML: not well-formed (invalid token): line 8, column 68 |
| w-be-bru-urbis-buildings-31370 | BE | wfs | 0.281 | Geen geldige capabilities-XML: not well-formed (invalid token): line 1, column 15626 |
| w-be-fpsfinance-cadgis-parcels-31370 | BE | wfs | 0.373 | Geen geldige capabilities-XML: syntax error: line 1, column 0 |
| w-be-fpsfinance-cadgis-plan-31370 | BE | wms | 0.266 | Geen geldige capabilities-XML: syntax error: line 1, column 0 |
| w-be-bru-urbis-cadastralparcels-31370 | BE | wfs | 0.281 | Geen geldige capabilities-XML: not well-formed (invalid token): line 1, column 15626 |
| w-be-bru-urbis-dtm-2019-31370 | BE | wms | 0.553 | Geen geldige capabilities-XML: not well-formed (invalid token): line 8, column 68 |
| w-be-bru-urbis-streetsurfaces-31370 | BE | wfs | 0.281 | Geen geldige capabilities-XML: not well-formed (invalid token): line 1, column 15626 |
| w-be-bru-urbis-bridges-31370 | BE | wfs | 0.281 | Geen geldige capabilities-XML: not well-formed (invalid token): line 1, column 15626 |
| w-br-df-edificacoes-31983 | BR | wms | 1.012 | HTTP 404 |
| w-br-df-lotesregistrados-31983 | BR | wms | 1.012 | HTTP 404 |
| w-br-sp-geosampa-benstombados-31983 | BR | wms | 8.021 | time-out |
| w-br-sp-geosampa-metrolines-31983 | BR | wms | 8.021 | time-out |
| w-ca-on-geo-imagery2023to2027-3857 | CA | wms | 8.02 | time-out |
| w-fi-syke-tulvavaara-vesistotulva-3067 | FI | wms | 0.256 | Standaardlaag ontbreekt: tulva_perus_peittama:tulvanpeittama_vesistotulva |
| w-fi-syke-tulvavaara-meritulva-3067 | FI | wms | 0.256 | Standaardlaag ontbreekt: tulva_perus_peittama:tulvanpeittama_meritulva |
| w-de-sh-lvermgeo-inspire-bu-wfs-25832 | DE | wfs | 8.028 | time-out |
| w-de-ni-lbeg-bohrungen-25832 | DE | wms | 0.185 | OGC-fout zonder exceptioncode in het antwoord |
| w-de-ni-lbeg-geologischekarte-gk50-25832 | DE | wms | 0.185 | OGC-fout zonder exceptioncode in het antwoord |
| w-de-bw-lgrb-gk50-25832 | DE | wms | 0.158 | HTTP 404 |
| w-in-nrsc-bhuvan-adminboundaries-3857 | IN | wms | 8.02 | time-out |
| w-in-nrsc-bhuvan-railways-3857 | IN | wms | 8.02 | time-out |
| w-il-soi-parcels-map-2039 | IL | wms | 0.889 | Standaardlaag ontbreekt: opendata:Parcels_ITM |
| w-il-govmap-streams-2039 | IL | wms | 0.889 | Standaardlaag ontbreekt: opendata:nechalim1 |
| w-no-ngu-berggrunn-25833 | NO | wms | 3.022 | time-out |
| w-no-ngu-nadag-borehull-25833 | NO | wms | 3.021 | time-out |
| w-ph-mgb-floodsusceptibility-3857 | PH | wms | 3.016 | time-out |
| w-pt-lneg-geologicalmap50k-wms-3857 | PT | wms | 1.196 | Capabilities bevat geen benoemde lagen of objecttypen |
| w-za-wc-capetown-aerial2026-3857 | ZA | wms | 3.016 | time-out |
| w-es-catastro-building-wfs-25830 | ES | wfs | 0.255 | Netwerkfout (curl 56) |
| w-es-catastro-building-wms-25830 | ES | wms | 0.222 | Netwerkfout (curl 56) |
| w-es-catastro-cadastralparcel-wms-25830 | ES | wms | 0.222 | Netwerkfout (curl 56) |
| w-es-catastro-cartografiacatastral-wms-25830 | ES | wms | 0.226 | Netwerkfout (curl 56) |
| w-tw-nlsc-nonurbanlandusezoning-3857 | TW | xyz | 0.781 | Geen herkenbare PNG/JPEG/WebP-proeftegel (mogelijk foutpagina of onvolledig beeld) |
| w-uy-ide-mdt-nacional-5382 | UY | wms | 1.824 | Standaardlaag ontbreekt: relieve:mdt_nacional |
| w-uy-ide-mds-urbano-5382 | UY | wms | 1.824 | Standaardlaag ontbreekt: relieve:mds_urbana |
| hisgis-kaart-1832 | NL | xyz | 0.114 | HTTP 404 |

## Alle trage proeven

| ID | Seconden |
|---|---:|
| w-us-usgs-3dep-contours-3857 | 6.33 |
| w-us-usgs-3dep-hillshade-3857 | 6.33 |
| w-au-wa-slip-cadastre-7850 | 6.078 |
| w-au-wa-slip-localplanningschemezones-7850 | 6.078 |
| w-au-wa-slip-regionschemezones-7850 | 6.078 |
| w-au-nsw-spatialservices-hydrography-7856 | 5.503 |
| w-au-nsw-spatialservices-topomap-7856 | 5.172 |
| w-lv-dap-aizsargajamasteritorijas-3059 | 5.045 |
| w-au-qld-qldmaptopo-7856 | 4.618 |
| w-br-rj-rio-ortho2025-31983 | 4.522 |
| w-au-dea-geomad-landsat-3857 | 3.938 |
| w-au-dea-landcover-3857 | 3.938 |
| w-au-qld-geologydetailed-7856 | 3.405 |
| w-us-usgs-naipimagery-3857 | 3.395 |
| w-ar-ign-ferrocarril-3857 | 3.36 |
| w-ph-mgb-landslidesusceptibility-3857 | 3.332 |
| w-nz-gns-activefaults-250k-2193 | 3.19 |

## Herkomst van de meting

| Bestand | SHA-256 |
|---|---|
| assets/servers/world.json | f3e669e8b08dc34bde4a9fdcfde9585098ae86ba26bace13f18bfe0f8b173dd3 |
| src/gis/servers.rs | a33642e4fcec74fe10d2f9ef33d3fa8edcb1ff1fdbc7e434488b10c905382f5f |
| tools/check_map_services.py | bdbd657015a89da6594ce83844d2aaf6c7b2bacb1cb0ec78e1e662e922e21136 |

De volledige JSON en Markdown worden bij volgende wekelijkse uitvoeringen als workflow-artefact bewaard. Deze eerste lokale meting staat in de oorspronkelijke werkmap onder `target/catalogue-health-eerste-meting/rapport.json` en `rapport.md`; de gegenereerde bestanden in `target/` zijn niet ingecheckt.
