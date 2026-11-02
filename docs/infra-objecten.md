# Infra-objecten

Eigen parametrische geometrie, NLCS-classificatie 5.0.2. Alle maten zijn configureerbare referentiewaarden.

## annotatie

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `hoogtepunt` | Hoogtepunt NAP | GW / HOOGTE_AFWERKHOOGTE | annotation | 0 |
| `afschotpijl` | Afschotpijl | GW / HOOGTE_AFWERKHOOGTE | annotation | 0 |
| `profielmarkering` | Profielmarkering | AL / PROFIELBALK | annotation | 0 |

## banden

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `opsluitband-150x250` | Opsluitband 150×250 | VH / KANTOPSLUITING_OPSLUITBAND_150X250 | line | 2 |
| `opsluitband-100x200` | Opsluitband 100×200 | VH / KANTOPSLUITING_OPSLUITBAND_100X200 | line | 2 |
| `trottoirband` | Trottoirband | VH / KANTOPSLUITING_TROTTOIRBAND | line | 2 |
| `inritband` | Inritband | VH / KANTOPSLUITING_INRITBAND | line | 2 |
| `gazonband` | Gazonband | VH / KANTOPSLUITING_GAZONBAND | line | 2 |
| `verloopband` | Verloopband | VH / KANTOPSLUITING_ELEMENT_VERLOOPBAND | line | 2 |

## groen

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `boom` | Boom kroon en stam | GR / BOOM | point | 0 |
| `boom-bestaand` | Bestaande boom | GR / BOOM | point | 0 |
| `plantvak` | Plantvak heesters | GR / BEPLANTING_CULTUUR_HEESTERS | area | 0 |
| `kruidenrijk-gras` | Kruidenrijk gras | GR / GRAS_KRUIDENRIJK | area | 0 |
| `bosplantsoen` | Bosplantsoen | GR / BEPLANTING_NATUUR_BOSPLANTSOEN | area | 0 |
| `riet-oever` | Riet en waterplanten | GR / BEPLANTING_CULTUUR_WATERPLANT | area | 0 |
| `oeverzone` | Natuurlijke oeverzone | GR / GRAS_NATUURLIJKE OEVER | area | 0 |
| `waterlijn` | Waterlijn | GW / WATERGANG_WATERLIJN | line | 0 |

## hemelwater

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `hwa-pvc315` | HWA PVC Ø315 | RI / HWA_RIOOLLEIDING | line | 2 |
| `drain-pp160` | Drain PP Ø160 | RI / DRAINAGE_DRAIN | line | 2 |
| `duiker-beton300` | Duiker beton Ø300 | KW / KUNSTWERK_DUIKER | line | 2 |
| `inspectieput-hwa` | Inspectieput HWA | RI / HWA_RIOOLPUT_INSPECTIEPUT | point | 4 |
| `drainageput` | Drainageput | RI / DRAINAGE_PUT | point | 4 |
| `slokop` | Slokop roosterput | RI / HWA_RIOOLPUT | point | 4 |
| `uitstroombak` | Uitstroombak beton | RI / DRAINAGE_UITLAATCONSTRUCTIE | point | 4 |
| `wadi` | Wadi | RI / ITR_RESERVOIR_INFILTRATIERESERVOIR_WADI | area | 0 |

## inrichting

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `zitbank` | Zitbank 2 m | IE / MEUBILAIR_WEGMEUBILAIR_ZITBANK | point | 0 |
| `fietsnietje` | Fietsnietje | IE / MEUBILAIR_WEGMEUBILAIR_FIETSENREK | point | 0 |
| `afvalbak` | Afvalbak | IE / MEUBILAIR_WEGMEUBILAIR_AFVALBAK | point | 0 |
| `containeropstelling` | Containeropstelplaats | IE / AFVAL APART PLAATS | point | 0 |
| `poort` | Poort 1200 mm | IE / TERREINAFSCHEIDING_KUNSTMATIG_HEKWERK | point | 0 |
| `landhoofd` | Landhoofd schematisch | KW / KUNSTWERK_LANDHOOFD | point | 0 |
| `faunapassage` | Faunapassage schematisch | FV / FAUNAPASSAGE | point | 0 |
| `landart-palen` | Landart palen schematisch | IE / KUNSTOBJECT | point | 0 |
| `klimboom` | Klimboom schematisch | IE / MEUBILAIR_WEGMEUBILAIR_SPEELTOESTEL | point | 0 |
| `hekwerk` | Hekwerk dubbelstaafmat | IE / TERREINAFSCHEIDING_KUNSTMATIG_HEKWERK | line | 2 |

## nuts

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `waterleiding` | Waterleiding | KL / WATER_TRANSPORTLEIDING | line | 2 |
| `glasvezel` | Glasvezel | KL / DATA_GLASVEZEL | line | 2 |
| `laagspanning` | Laagspanningskabel | KL / ET_LS | line | 2 |
| `mantelbuis-water` | Mantelbuis water Ø160 | KL / WATER_MANTELBUIS | line | 2 |
| `mantelbuis-data75` | Mantelbuis data Ø75 | KL / DATA_MANTELBUIS | line | 2 |
| `mantelbuis-laad75` | Mantelbuis laden Ø75 | KL / ET_MANTELBUIS | line | 2 |
| `mantelbuis-laad110` | Mantelbuis laden Ø110 | KL / ET_MANTELBUIS | line | 2 |
| `watermeterput` | Watermeterput 1160×910 | KL / WATER_PUT | point | 2 |
| `meterkast` | Meterkast | KL / ET_LS_SR_KAST | point | 2 |
| `trafostation` | Trafostation 3600×2000 | KL / ET_MS_TRANSFORMATORSTATION | point | 4 |
| `hydrant` | Brandhydrant | KL / WATER_BRANDLEIDING_BRANDHYDRANT | point | 1 |

## profielen

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `profiel-fietsstraat` | Opbouw fietsstraat | VH / GESLOTENVERHARDING_ASFALT_ROOD | profile | 0 |
| `profiel-straatbaksteen` | Opbouw straatbaksteen | VH / OPENVERHARDING_STRAATBAKSTEEN | profile | 0 |
| `profiel-halfverharding` | Opbouw halfverharding | VH / HALFVERHARDING | profile | 0 |
| `profiel-wadi` | Wadi bodemopbouw | RI / ITR_RESERVOIR_INFILTRATIERESERVOIR_WADI | profile | 0 |
| `profiel-nutssleuf` | Nutsprofiel 0,8×1,2 m | GW / MAAIVELD | profile | 0 |
| `detail-waterhof` | Aansluitdetail waterhof | RI / PERSLEIDING_RIOOLPUT | profile | 0 |
| `profiel-oever` | Oeverprofiel schematisch | GW / TALUD | profile | 0 |

## riolering

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `persleiding-pe63` | Persleiding HDPE Ø63 SDR17 | RI / PERSLEIDING | line | 2 |
| `afsluiter-pe63` | Afsluiter Ø63 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `balkeerklep-pe63` | Balkeerklep Ø63 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `pig-stuk-pe63` | PIG-stuk Ø63 | RI / PERSLEIDING_HULPSTUK | point | 3 |
| `ontluchter-pe63` | Ontluchter Ø63 | RI / PERSLEIDING_HULPSTUK | point | 1 |
| `bocht-pe63` | Bocht Ø63 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `t-stuk-pe63` | T-stuk Ø63 | RI / PERSLEIDING_HULPSTUK | point | 3 |
| `persleiding-pe75` | Persleiding HDPE Ø75 SDR17 | RI / PERSLEIDING | line | 2 |
| `afsluiter-pe75` | Afsluiter Ø75 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `balkeerklep-pe75` | Balkeerklep Ø75 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `pig-stuk-pe75` | PIG-stuk Ø75 | RI / PERSLEIDING_HULPSTUK | point | 3 |
| `ontluchter-pe75` | Ontluchter Ø75 | RI / PERSLEIDING_HULPSTUK | point | 1 |
| `bocht-pe75` | Bocht Ø75 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `t-stuk-pe75` | T-stuk Ø75 | RI / PERSLEIDING_HULPSTUK | point | 3 |
| `reductie-75-63` | Verloop Ø75 naar Ø63 | RI / PERSLEIDING_HULPSTUK | point | 2 |
| `dwa-pvc315` | DWA PVC Ø315 | RI / DWA_RIOOLLEIDING | line | 2 |
| `inspectieput-pers` | Inspectieput persriool 1250×1250 | RI / PERSLEIDING_RIOOLPUT | point | 4 |
| `inspectieput-dwa` | Inspectieput DWA | RI / DWA_RIOOLPUT_INSPECTIEPUT | point | 4 |

## verharding

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `straatbaksteen-kf` | Straatbaksteen keiformaat | VH / OPENVERHARDING_STRAATBAKSTEEN | area | 0 |
| `straatbaksteen-df` | Straatbaksteen dikformaat | VH / OPENVERHARDING_STRAATBAKSTEEN | area | 0 |
| `asfalt-rood` | Rood asfalt deklaag 30 mm | VH / GESLOTENVERHARDING_ASFALT_ROOD | area | 0 |
| `asfalt-zwart` | Asfalt zwart | VH / GESLOTENVERHARDING_ASFALT | area | 0 |
| `grasbeton-120` | Grasbeton 120 mm | VH / HALFVERHARDING_GRASBETONSTEEN | area | 0 |
| `grasbeton-100` | Grasbeton 100 mm | VH / HALFVERHARDING_GRASBETONSTEEN | area | 0 |
| `betontegel-300` | Betontegel 300×300×50 | VH / OPENVERHARDING_TEGEL | area | 0 |
| `halfverharding` | Halfverharding gestabiliseerd | VH / HALFVERHARDING | area | 0 |
| `houtsnipperpad` | Houtsnipperpad | VH / HALFVERHARDING | area | 0 |
| `grind` | Grindverharding | VH / HALFVERHARDING_GRIND | area | 0 |
| `betonplaat` | Betonplaat | VH / GESLOTENVERHARDING_BETON | area | 0 |
| `betonstraatsteen` | Betonstraatsteen | VH / OPENVERHARDING_BETONSTRAATSTEEN | area | 0 |

## verkeer

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `verkeersbord` | Verkeersbord generiek | VW / VERKEERSTEKEN_BORD | point | 0 |
| `fietsstraat-bord` | Bord fietsstraat | VW / VERKEERSTEKEN_BORD | point | 0 |

## verlichting

| ID | Onderdeel | NLCS-object | Soort | Poorten |
|---|---|---|---|---|
| `lichtmast` | Lichtmast | OV / LICHTMAST | point | 1 |
| `bolderlamp` | Bolderverlichting | OV / LICHTMAST | point | 1 |
| `voedingskast-ov` | Voedingskast OV | OV / KAST_VOEDINGSPUNT | point | 1 |
| `aftakmof-ov` | Aftakmof OV | KL / ET_LS_HULPSTUK | point | 1 |
| `laadpaal` | Laadpaal | KL / ET_LS_SR_KAST | point | 1 |
| `ov-kabel-2-5` | OV kabel 4×2.5 mm² | OV / KABEL_VOEDINGSKABEL | line | 2 |
| `ov-kabel-10` | OV kabel 4×10 mm² | OV / KABEL_VOEDINGSKABEL | line | 2 |
| `ov-kabel-16` | OV kabel 4×16 mm² | OV / KABEL_VOEDINGSKABEL | line | 2 |
| `ov-kabel-25` | OV kabel 4×25 mm² | OV / KABEL_VOEDINGSKABEL | line | 2 |
