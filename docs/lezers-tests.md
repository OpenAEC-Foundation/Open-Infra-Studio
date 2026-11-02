# Invoerlezers en hun tests

Inventaris bij `e7f6fdc` (2 november 2026). Dit overzicht beschrijft de
aangetroffen tests; aanwezigheid van een test bewijst geen volledige dekking.

| Lezer | Productiecode | Aangetroffen offline tests bij aanvang | Nog te onderzoeken |
| --- | --- | --- | --- |
| CityGML / IMGeo | `src/gis/citygml.rs` | Twee inline voorbeelden: LoD/semantiek, 2D/CRS | Kapotte XML, numerieke velden, streamgrenzen |
| CityJSON / CityJSONSeq | `src/gis/cityjson.rs` | Twee inline voorbeelden: ouder/kind/LoD, sequence | Ongeldige regels, transformatie, indices |
| LandXML | `src/civiel/landxml.rs` | Drie tests: punten/TIN/assen, spiraal, eenheden/fouten | Ontbrekende velden, ongeldige getallen, referenties |
| Algemene GML 2/3 | `src/gis/files.rs` | Eén inline WFS-collectie | Dimensies, kapotte coördinaten, ontbrekende geometrie |
| IMRO-GML | `src/gis/planning/gml.rs` | `tests/planning.rs`, `tests/planning_render.rs`, vaste synthetische GML | Aanvullende grensgevallen; recente code uit PR #18 |
| BRO CPT/BHR/zoekresultaten | `src/gis/bro.rs` | Zes tests, zes vaste XML-bestanden, onder meer een afwijzing | Ongeldige getallen en ontbrekende verplichte velden |
| KLIC / IMKL | `src/gis/klic.rs`, afhankelijkheid `klic2geo` | Optionele lokale levering; test wordt standaard overgeslagen | Kleine zelfstandige fixture zonder lokale levering |
| GeoJSON, Shapefile, GeoPackage | `src/gis/files.rs`, `src/gis/vector.rs` | CRS/WKT/WKB en zelfgemaakte Zürich-bestanden | Kapotte binaire gegevens, geometriegrenzen |
| KML/KMZ, GPX, CSV | `src/gis/files.rs` | Inline KML/GPX/CSV en Zürich-bestanden | Onvolledige coördinaten, niet-eindige getallen, archieven |
| Landmeetpunten PNEZD/PENZD/CSV | `src/civiel/landmeten.rs` | Eén test met meerdere varianten | Ongeldige en zeer grote getallen |
| GeoTIFF, ASC, world files | `src/gis/rasterbestand.rs`, `src/gis/ahn.rs` | ASC/world-file en hoogte-/TIFF-tests | Afmetingen, overflow, ontbrekende rasterwaarden |
| LAS/LAZ, COPC | `src/gis/pointcloud.rs`, `src/gis/rasterbestand.rs` | Onder meer classificatie/kleuren; live COPC apart overgeslagen | Kleine kapotte bestanden en CRS-afhandeling |
| WMS/WFS-capabilities en servicereacties | `src/gis/caps.rs`, `wfs.rs`, `gwsw.rs`, `bag3d.rs`, `vector.rs` | Inline voorbeelden voor enkele XML-/JSON-reacties | Scheid dienstbereikbaarheid van leesfouten |

De reguliere `offline`-baan in `.github/workflows/test.yml` voert de library-
tests en de planningtests uit bij pull requests. De aparte `online`-baan is
voor geplande of handmatige dienstcontroles. Nieuwe regressietests horen in
de offline baan en gebruiken uitsluitend vaste of deterministisch veranderde
invoer. Bron en licentie staan bij de nieuwe fixtures.

Elke reparatie blijft bij één lezer of één foutsoort. Bewijs van de falende
test vóór de reparatie en de geslaagde test erna hoort in de bijbehorende PR.
