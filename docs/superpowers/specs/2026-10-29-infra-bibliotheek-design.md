# Infra-bibliotheek en civiele tekenvaardigheden

Datum: 29 oktober 2026. Status: ontwerp ter beoordeling; implementatie nog niet uitgevoerd.

## Doel en afbakening

Breid Open Infra Studio uit met een geïntegreerde, doorzoekbare infra-bibliotheek en herbruikbare civiele tekenvaardigheden. De bibliotheek levert plaatsbare symbolen, lijn- en vlakobjecten, materiaalopbouwen en samengestelde profielen/details. Objecten krijgen een expliciete koppeling met NLCS 5.0.2, eenheden, bronvermelding en controleerbare parameters. Het openbaargebiedontwerp Oosterhoutse Baai dient als inhoudelijke referentie; projectspecifieke keuzes worden geen algemene ontwerpnormen.

De eerste oplevering is een bruikbare uitbreiding van de bestaande Rust-plugin, met een catalogus, preview, plaatsing, automatisering, voorbeeldtekening en skills. Geotechnische, hydraulische en constructieve dimensionering vallen buiten deze bibliotheek. Een getekend detail bewijst geen technische geschiktheid.

## Onderzochte projectonderdelen

- `README.md`, `Cargo.toml` en `plugin.toml`: Rust-workspace met GIS-plugin en afzonderlijke Civiel-plugin, hostcontract API 13.
- `src/library/mod.rs`, `draw.rs`, `place.rs` en `src/ui/library.rs`: databestand voor voertuigen, geometriegeneratie, blokdefinitie, plaatsing en catalogusweergave. Dit is een bestaande uitbreidingsbasis; er is nog geen algemene infra-objectbibliotheek in deze module.
- `src/nlcs.rs`, `assets/nlcs/*.tsv` en `tools/nlcs_tables.py`: 3.659 objectdefinities, groepen, disciplines, statussen en representatiegegevens uit de officiële publicatie.
- `src/nlcs_check.rs`: controle van laagnamen, objecten, elementtypen, representatie en een deel van de tekeneigenschappen. Dit is geen volledige certificering van een tekenpakket.
- `src/hoeveelheden.rs`: bestaande hoeveelheden per laag. Bij nieuwe objecten is een expliciete tel-/meetregel nodig om geometrie en arcering niet dubbel te tellen.
- `src/civiel/api.rs` en `mcp/README.md`: bestaande patronen voor commandoargumenten, projectcoördinaten in meters en JSON-resultaten.

## Bronanalyse

Bronmap: `DO Openbaar gebied - VEN`, onder het Definitief Ontwerp van Oosterhoutse Baai.

De bestandsinventaris telt 24 DWG's, 1 DXF, 377 PDF's en 6 DOCX-bestanden. De telling omvat meerdere standen en vervallen stukken; deze aantallen zijn geen aantallen unieke ontwerpen. Niet alle 377 PDF's zijn inhoudelijk beoordeeld.

Primaire referenties:

1. `TO Openbaar gebied 2026-10-07.docx`: actuele technische omschrijving met materialen, infrastructuur, uitvoeringsfasen en scopegrenzen.
2. `Stand 2026-07-24/Ontwerpnota Oosterhoutseplas-2.0.pdf`: 14 pagina's; documentdatum 16 juli 2026; ontwerpprincipes, thema's en verwijzing naar de verificatiematrix.
3. `Stand 2026-07-24/02 Ontwerpdocumenten/08 DO tekeningen`: situaties, ondergrondse infra, dwarsprofielen, groenplan, nutsprofielen, hoogteplan, bebording, rijcurves en verlichting. De mapdatum is niet automatisch de documentdatum: blad 02 versie 6.0 vermeldt 28 juli 2026.
4. De overige thematische ontwerpdocumenten en gemeentelijke richtlijnen zijn aanvullende bronnen. Verwijzingen ernaar in een nota zijn geen bewijs dat alle inhoud daarvan al getoetst is.

De nutsprofieltekening is ook als afbeelding beoordeeld: boringsprofielen, maaiveld-/leidinglijnen, nutsdoorsneden, peilreferenties, profielbanden en waarschuwing dat een boorplan verder moet worden uitgewerkt. Dit moet in de bibliotheek als configureerbaar profiel met bronstatus worden behandeld.

### Kwaliteit van de beschikbare CAD-bron

De DXF van Nieuwe situatie definitief v4.0 heeft `$INSUNITS=6` (meters) en 18 lagen. De lagen heten grotendeels `PDF_LIJN_*`, `PDF_VLAK_*` en `PDF_TEKST`; geen laag begint met een NLCS-statusprefix. De entities-sectie bevat 117.433 LWPOLYLINE's, 5.880 SPLINE's, 3.958 HATCH's en 2.168 TEXT-objecten, samen 129.439 tekenobjecten. Er zijn geen INSERT-objecten in die sectie.

Dit wijst op uit een PDF gereconstrueerde geometrie. De DXF is bruikbaar voor visuele referentie, maar levert geen betrouwbare objectclassificatie of direct overneembare symbolenbibliotheek. Een kleurlaag is geen NLCS-mapping. De 24 DWG's zijn geïnventariseerd; hun interne lagen, blokken en referenties zijn nog niet onderzocht. Een oudere DWG kan aanvullende geometrie leveren, maar mag niet zonder versiecontrole de actuele ontwerpinhoud bepalen.

### Concrete inhoud voor de bibliotheek

| Thema | Onderdelen uit de referenties | Te modelleren als |
|---|---|---|
| Verhardingen | hergebruikte straatbaksteen keiformaat en dikformaat, rood asfalt, grasbeton, halfverharding, betontegels 300×300×50 mm, houtsnipperpad | vlak met materiaalvariant, verband, laagopbouw en hoeveelheidsregel |
| Kantopsluitingen | betonband 150×250 mm, overige banden volgens tekening | lijn met profielvariant en doorsnede |
| Riolering | PE/HDPE Ø63 en Ø75 SDR17, inspectieput, afsluiter, balkeerklep, PIG-stuk, ontluchter en aansluiting op vrijvervalstelsel | leiding, puntsymbool en samengesteld aansluitdetail |
| Hemelwater | wadi, drainage PP160, betonduiker Ø300, slokop, uitstroombak | lijn/vlak/symbool met afzonderlijke functionele kenmerken |
| Nuts | gereserveerde sleuf, mantelbuizen, watermeterput, meterkast, trafostation, hydrant | leiding, reserveringsvlak, symbool en dwarsprofiel |
| Verlichting en laden | lichtmast, bolder, voedingskast, kabel, aftakmof, laadpaal | puntsymbool of kabeltracé met type en status |
| Groen, water en oevers | boom, plantvak, kruidenrijk gras, oeverzone, waterlijn, bestaand/ontwerp maaiveld | symbool, vlak, lijn en profielcomponent |
| Inrichting en verkeer | bank, fietsnietje, hekwerk, poort, containeropstelling, bebording, faunapassage | maatvast object of symbool met expliciete schaalregel |
| Annotatie | hoogte, afschot, materiaalcode, profielmarkering, maatvoering, legenda | schaalafhankelijke annotatie gekoppeld aan het object |

### Tegenstrijdigheden en onvolledigheden

- Bladen 05 (nutsprofielen) en 11 (lichtmasten) dragen in de bestandsnaam `def`, maar het titelblok vermeldt CONCEPT. De documentstatus in het titelblok moet afzonderlijk worden vastgelegd; een bestandsnaam bewijst geen goedkeuring.

- De technische omschrijving noemt voor de persleiding circa 5 mm/m afschot; blad 02 versie 6.0 noemt voor HDPE Ø75 3 mm/m. Dit zijn alternatieve bronwaarden totdat hun toepassingsgebied of voorrang vastgesteld is.
- De technische omschrijving noemt grasbeton van 120 mm dik; blad 03 bevat een opbouw met 100 mm. Materiaalvarianten bewaren beide bronnen en krijgen geen stilzwijgend samengevoegde maat.
- De technische omschrijving noemt PVC 300 mm; de legenda op blad 02 vermeldt Ø315, naast andere aanduidingen Ø300. Materiaal, nominale maat en buiten-/binnendiameter worden afzonderlijk opgeslagen; zonder productspecificatie is niet bewezen dat dit een fout is.
- Mantelbuizen hebben verschillende functies en diameters: onder meer Ø75/110 uit de technische omschrijving en PVC Ø160 per waterhof op blad 02. Varianten moeten op functie worden onderscheiden.
- De technische omschrijving bevat open opmerkingen over de borging van twee landart-elementen; het groenplan toont beide wel in de legenda. Ook verschillen hoogten: voor de betonnen palen staat in de omschrijving circa 2,6 m boven maaiveld en in de groenplanlegenda 1,5 m. Daarnaast bevat de omschrijving een verwijzing naar een datum na 29 oktober 2026. Dit zijn te verifiëren brongegevens.
- De technische omschrijving benoemt de engineering en levering rond landhoofden en steigers deels bij andere partijen. Details mogen deze raakvlakken tonen, maar geen ontbrekende engineering invullen.

## Normbasis en conformiteit

De bestaande objecttabellen blijven de normbasis: NLCS 5.0.2. De officiële repository noemt 5.0.2 en verwijst naar versie 5.0 voor de actuele specificaties. Documentatie met label 5.1 is afzonderlijk beschikbaar; die wordt niet met de huidige tabellen vermengd.

Bronnen:

- https://github.com/nl-digigo/NLCS — officiële publicatie, versies en CC BY 4.0.
- https://www.forumstandaardisatie.nl/open-standaarden/nlcs — geregistreerde standaard en toepassingsgebied.
- https://nl-digigo.github.io/NLCS/functionalspecification/5-1/ — afzonderlijke documentatie voor versie 5.1.

Een item verwijst naar een bestaand object-ID en hoofdgroep; representatie wordt met de bestaande NLCS-functies afgeleid. Status, discipline en elementtype blijven expliciete invoer. Geen fallback naar een willekeurige laag als de mapping ontbreekt. Namen van eigen blokken volgen de symbolensystematiek van de vastgelegde normversie; de bestaande voertuigprefix `OI_` wordt hiervoor niet automatisch overgenomen.

Voorbeelden van beschikbare mappings, gecontroleerd in de lokale tabel:

| Bibliotheektype | NLCS-object-ID | Hoofdgroep en object | Toegestaan |
|---|---|---|---|
| Opsluitband 150×250 | 2405 | VH / KANTOPSLUITING_OPSLUITBAND_150X250 | G |
| Grasbeton | 2508 | VH / HALFVERHARDING_GRASBETONSTEEN | G/S/A |
| Zitbank | 3413 | IE / MEUBILAIR_WEGMEUBILAIR_ZITBANK | G/S |
| Fietsenrek | 3406 | IE / MEUBILAIR_WEGMEUBILAIR_FIETSENREK | G/S |
| Lichtmast openbare verlichting | 911 | OV / LICHTMAST | G/S |
| Wadi | 3830 | RI / ITR_RESERVOIR_INFILTRATIERESERVOIR_WADI | G |
| Drainageput | 1656 | RI / DRAINAGE_PUT | G/S |
| Watermantelbuis | 1358 | KL / WATER_MANTELBUIS | G |

Een geometrische weergave, een symbool en een arcering zijn verschillende representaties. Bijvoorbeeld: een opsluitband met uitsluitend G wordt niet op een fictieve S-laag geplaatst om hem als catalogusitem te kunnen gebruiken. Materiaalmaat, type, fabrikant en functie zijn metadata; die worden niet willekeurig aan officiële objectnamen toegevoegd.

Officiële symbolen en patronen worden waar beschikbaar gebruikt met vastgelegde herkomst, versie en licentie. Ontbrekende vormen worden als eigen bibliotheekonderdeel herkenbaar beschreven. Projectgeometrie en leverancierstekeningen worden niet als algemeen vrij beschikbare content gepubliceerd zonder passende rechten. Generieke onderdelen worden opnieuw parametrisch opgebouwd.

## Gekozen opzet en afweging

De geïntegreerde uitbreiding past bij de bestaande plugin: één catalogus en gedeelde NLCS-mapping voor UI, commando's en skills. Losse DWG-blokken zijn makkelijker afzonderlijk te verspreiden, maar dekken vlakobjecten, laagopbouwen en parametercontrole onvoldoende. Een afzonderlijke plugin zou cataloguslogica, representatie en plaatsing dupliceren. Daarom wordt de bestaande bibliotheek uitgebreid; voertuigen blijven beschikbaar.

## Technisch ontwerp

### Catalogus en gegevens

Voeg een apart bestand `assets/library/infra.json` en een bijbehorend Rust-datamodel toe. Houd voertuigdata en infra-data afzonderlijk; deel alleen gemeenschappelijke weergave- en plaatsingshulpen waar dat nuttig is.

Een verplaatsing van bijvoorbeeld een bestaande lichtmast wordt als handeling vastgelegd met herkomst- en doelpositie; deze handeling wordt niet verward met een extra NLCS-tekenstatus. De NLCS-laagstatus volgt de representatie per toestand.

Een infra-item bevat minimaal:

- stabiel ID, Nederlandse/Engelse naam, categorie, zoekwoorden en revisie;
- objectsoort: punt, lijn, vlak, profiel of samengesteld detail;
- NLCS-versie, object-ID, discipline en toegestane representaties;
- geometriesjabloon, lokaal invoegpunt, oriëntatie en schaalgedrag;
- parameters met expliciete eenheid, geldige grenzen en status van de waarde;
- materiaal, maatvariant, opbouwlagen en meetregel;
- bronnen met document, versie/datum en pagina, blad, legenda of paragraaf;
- onderscheid tussen standaardwaarde, projectwaarde, aanname en onopgelost bronconflict.

Geometrie rekent intern in meters. Materiaalmaten kunnen in millimeters worden getoond. Omrekening naar tekeningeenheden gebeurt precies één keer bij plaatsing via de bestaande hostcontext. Papiergebonden symbolen en tekst worden expliciet onderscheiden van maatvaste objecten.

Samengestelde details bevatten componenten met hun eigen NLCS-classificatie. Geen enkel blok met meerdere materialen/disciplineonderdelen op één verzonnen laag. Bouw componenten waar mogelijk op met bestaande geometrieprimitieven en bestaande profiel-/terreinfuncties.

### Gebruikersinterface

Breid de bibliotheekpagina uit met de keuze Voertuigen/Infra, thematische filters, zoeken en previews voor het relevante aanzicht. Toon maten, materiaal, normversie, bronstatus en eventuele conflicten bij het gekozen onderdeel. Plaatsing ondersteunt punt+richting, lijn of gesloten vlak passend bij het item. Profielen/details krijgen configureerbare parameters en een expliciete schaal.

Leg geen technische implementatievelden aan de eindgebruiker voor. Laat een onopgelost bronconflict wel zien voordat een projectspecifieke waarde wordt gebruikt.

### Commando's en automatisering

Voorgestelde commandonamen: `GIS_INFRA_LIBRARY`, `GIS_INFRA_CATALOG` en `GIS_INFRA_PLACE`, met Nederlandstalige aliassen waar passend. De definitieve argumenten volgen het bestaande commando- en resultaatpatroon. Een plaatsingsresultaat meldt object-ID, parameters, bronstatus, NLCS-lagen en handles. Een fout levert geen halve plaatsing op; plaatsing vormt één herstelbare bewerking.

Voeg zoek-/catalogus- en plaatsingsfuncties toe aan de bestaande Open Infra MCP-server. Skills verwijzen uitsluitend naar daadwerkelijk geïmplementeerde commando's en kunnen ontbrekende mogelijkheden aangeven.

## Civiele teken-skills

Gebruik `skill-creator` en `writing-skills` voor de uitwerking. Bewaar overdraagbare projectskills onder `.agents/skills/`; laat bronverwijzingen en gedeelde bibliotheekinformatie in onderhoudbare referenties staan. Geen chatverslagen of projectspecifieke ontwerpwaarden als universele eisen.

Voorgestelde skills:

1. `civiel-situatietekening`: bronstand kiezen, coördinaten/eenheden vaststellen, bestaande en nieuwe toestand, verhardingen, banden en inrichting plaatsen.
2. `civiel-riolering-en-nuts`: tracés, putten, mantelbuizen, labels en aansluitdetails met stelsel, diameter, peilen en bronstatus.
3. `civiel-hoogteplan-en-profielen`: maatgevende punten, afschot, NAP, terreinbron, doorsnedes en consistente horizontale/verticale schalen.
4. `civiel-materialen-en-details`: materiaalvarianten, laagopbouwen, detailcomponenten, maatvoering en legenda vanuit catalogusgegevens.
5. `civiel-groen-en-inrichting`: plantvakken, bomen, oevers, meubilair, bebording en raakvlakken met kabels/leidingen.
6. `civiel-nlcs-oplevercontrole`: NLCS-versie, lagen/representatie, tekst- en symboolschaal, legenda, bronconflicten, dubbeltelling en visuele controle van de uitvoer.

Een skill controleert de beschikbare sessie/tekening en leest de toepasselijke toolmogelijkheden vóór mutaties. Hij bewaakt RD/NAP en eenheden, behoudt oorspronkelijke bronbestanden en schrijft een controleerbare afgeleide tekening. Leidingschetsen, boringen en civiele details worden alleen als uitgewerkt ontwerp aangemerkt voor zover invoer en beoordeling dat dragen.

## Oplevering en verificatie

1. Catalogus met een eerste bruikbare set onderdelen voor alle genoemde hoofdthema's. Definitieve geometriekeuze en aantal varianten worden tijdens implementatie bepaald op basis van aantoonbare bronnen.
2. Geïntegreerde zoek-/preview-/plaatsingsfunctie en automatiseringsinterface.
3. Zes gevalideerde civiele skills met referenties naar de implementatie.
4. Een catalogus-/voorbeeldtekening en visueel gecontroleerde weergave, met onderscheid tussen symbolen, maatvaste objecten en details.
5. Een korte gebruiks- en uitbreidingsbeschrijving met bron-/licentievermelding.

Gerichte controles: unieke IDs, geldige NLCS-object/elementcombinaties, numerieke parametergrenzen, eenheden m/mm, alle vier tekenstatussen, geometrieomvang/invoegpunt, plaatsing en terugdraaien, componentlagen van samengestelde details, geen dubbeltelling en behoud van bestaande voertuigfuncties. De catalogus moet fouten expliciet melden en mag geen ongeldige records stilzwijgend overslaan.

Voer passende Rust-tests en de build uit. Controleer plaatsing en uitvoer ook in een echte Open CAD Studio-sessie wanneer die beschikbaar is; rapporteer ontbrekende runtimecontrole als beperking. Valideer de skills met de beschikbare skill-validator en realistische scenario's voor bronconflicten, ontbrekende peilen en eenhedenomrekening.

## Nog te besluiten vóór uitvoering

Goedkeuring van deze concrete opzet. Er is geen extra keuze nodig over een nieuwe app of framework: Rust, de bestaande plugin en de bestaande NLCS-tabellen blijven het uitgangspunt. Onopgeloste technische bronconflicten blokkeren alleen het als definitief toepassen van die specifieke projectwaarde, niet het bouwen van de generieke bibliotheek.
