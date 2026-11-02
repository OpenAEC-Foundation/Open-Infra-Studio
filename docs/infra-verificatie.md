# Verificatie infra-bibliotheek

Datum: 30 oktober 2026. Normtabellen: NLCS 5.0.2. Toolchain: Rust 1.99.1, Windows MSVC.

## Uitgevoerde controles

| Controle | Resultaat |
|---|---|
| `cargo test -p open-infra` | 148 geslaagd, 10 bestaande omgevingsafhankelijke tests genegeerd |
| `cargo test -p open-infra-civiel` | geslaagd; deze crate heeft geen afzonderlijke unit-tests |
| `cargo build -p open-infra` | geslaagd |
| `cargo build -p open-infra-civiel` | geslaagd |
| Python-catalogus- en MCP-tests | 3 geslaagd |
| Skill-validator | alle 6 repository-skills geldig |
| Gedragsscenario's skills | bronconflict, PDF-DXF-classificatie, RD/m/mm, parameterupdate en band/vlak-aansluiting beoordeeld |
| Onafhankelijke codecontrole | aansluitingen, concurrente mutaties, parameters, metadata, native arceringen en hoeveelheden beoordeeld; wezenlijke bevindingen verwerkt |
| CAD-catalogus | 94 objecten als DXF en DWG geschreven en opnieuw ingelezen, met behoud van parameters |
| Aansluitvoorbeeld | 6 objecten als DXF en DWG geschreven en opnieuw ingelezen; herberekening bij lengtewijziging gecontroleerd |
| Native CAD-audit | OpenCADStudio 2026.43, build dbd50b20091c: AC1032, 13 tekenobjecten, 0 fouten, 0 waarschuwingen, geen verlies bij opslaan |
| Native save-verificatie | controlekopie opnieuw opgeslagen, heropend en semantisch vergeleken; verified=true |
| Visuele controle | catalogusvoorvertoning en aansluitvoorbeeld in het CAD-venster beoordeeld |

De tien gerichte Rust-integratietests controleren themadekking, geldige geometrie, aansluitpositie/-richting, diameter-/stelselverschillen, ongeldige parameters en polygonen, oppervlakten, afschot, afhankelijke hergeneratie, poortbezetting/kringen, metadata/eenzelfde hoeveelheid in meter- en millimetertekeningen, band/vlakrand-aansluiting, onbekende hoogten, alle vier statussen, hoek-/laagdikteparameters en een stabiel onderdelenaantal bij vlakvergroting.

## Afbakening van de verificatie

De nieuwe pluginbibliotheken zijn gebouwd. De native CAD-controle gebruikte de gegenereerde bestanden; de volledige nieuwe UI- en commando-interactie is niet opnieuw met de nieuwgebouwde plugin in een afzonderlijk geïnstalleerde CAD-sessie doorlopen. Deze codecontrole en bestandsverificatie vervangen dus geen volledige interactieve acceptatietest.

De CAD-audit controleert de bestandsstructuur en bewaarbaarheid, niet de volledige inhoudelijke NLCS-conformiteit. Geometrie en materiaalpatronen zijn eigen schematische definities. Complexe lijntypen kunnen vereenvoudigd zijn. Technische dimensionering, gemeentelijke acceptatie en de projectspecifieke bronconflicten blijven afzonderlijke beoordelingen.

De twee plugins worden afzonderlijk getest en gebouwd vanwege hun bestaande host-exports. Een gecombineerde workspace-test verenigt features en geeft dubbele exportsymbolen; daarvoor zijn geen afhankelijkheden of hostcontracten gewijzigd.
