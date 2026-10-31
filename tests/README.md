# Testset Open Infra

Drie lagen tests. De locaties en hun verwachtingen staan in [`data/locaties.json`](data/locaties.json):

- Nederland: Dordrecht, Amsterdam, Utrecht, de Maasvlakte, een polder bij Lelystad, de Vaalserberg, Terschelling en Loppersum.
- Over de grens: Antwerpen, Aken, Londen, Zürich, Parijs, New York en Sydney.
- De Noordzee: overal leeg.
- 30 grenspunten rond Nederland.

## 1. Offline (ook in CI)

```bash
cargo test --lib            # eenheidstests van de engine
cargo test --test locaties  # locatieset zonder netwerk
```

| Test | Wat |
|---|---|
| `stelsel_per_locatie` | Het voorgestelde coördinatenstelsel per locatie, op land (zoeken) en op positie (klik op de kaart). |
| `stelselkeuze_rond_de_grens` | 30 plaatsen rond de Nederlandse grens: Kerkrade RD, Aken UTM 32, Hulst RD, Antwerpen Lambert 72, Baarle-Nassau, Lille, enzovoort. |
| `transformatie_heen_en_terug` | Heen en terug binnen 1 cm, en de schaal van het stelsel. |
| `projectlocatie_in_de_tekening` | Het importgebied in de tekening: absoluut en relatief, meters en mm. |
| `bgt_kenmerken_naar_nlcs` | 121 echte BGT-kenmerkcombinaties ([`data/bgt-kenmerken.json`](data/bgt-kenmerken.json)). Elk object moet op een geldige NLCS-laag landen. |
| `klic_levering_naar_nlcs_en_imkl` | De KLIC-voorbeeldlevering van GIS2BIM: NLCS-objecten, IMKL-kleuren, ligging en labels. Wordt overgeslagen als de levering er niet is (`OPEN_INFRA_KLIC=<zip>`). |
| `hoogtelijnen_en_profiel_op_een_dijk` | Een kunstmatige dijk met een gat. Gecontroleerd: de ligging van de hoogtelijnen en de profielstations. |
| `tekenblad_kiest_het_formaat` | A3–A0 bij 1:500 en 1:1000. |
| `nap_notatie` | `+2,43`, `-6,10`. |

## 2. Online: data ophalen per locatie

```bash
cargo test --test locaties -- --ignored --nocapture
LOCATIES=dordrecht,vaals cargo test --test locaties -- --ignored --nocapture
```

Per locatie worden deze bronnen getest:
- zoeken (PDOK en Nominatim);
- OSM-kaartbeeld en PDOK-luchtfoto;
- BGT: actueel, binnen het gebied, alles NLCS;
- BRK;
- OpenStreetMap;
- AHN-hoogtes, binnen het verwachte NAP-bereik, en hoogtelijnen;
- AHN-profiel;
- AHN-puntenwolk, met een `.rcs`-bestand;
- het formaat van het tekenblad.

Buiten Nederland moeten BGT, BRK en AHN leeg blijven of netjes weigeren, zonder te crashen.

Het rapport staat in `target/locatietest/rapport.md`, met een matrix locatie × bron, en in `rapport.json`. Een overbelaste openbare dienst (Overpass, Nominatim) telt als waarschuwing ⚠, niet als fout ✘.

In GitHub Actions draait deze test elke maandag. Hij is ook op verzoek te starten (*Actions → test → Run workflow*); het rapport is een artifact.

## 3. End-to-end in Open CAD Studio

```bash
./scripts/install.sh                       # plugin installeren
python3 tests/e2e/ocs_e2e.py               # Dordrecht, Vaals, Lelystad, Londen
python3 tests/e2e/ocs_e2e.py --snel        # alleen Dordrecht
```

Het script start Open CAD Studio met een lege tekening en de REST-koppeling (`OpenCADStudio start.dwg --http <poort>`), met de geïnstalleerde plugin. Pad naar de executable: `--ocs` of `OCS_BIN`.

Er verschijnt dus een venster. Zonder venster verwerkt Open CAD Studio de achtergrondverzoeken van een plugin alleen tijdens een plugin-commando, waardoor imports stil blijven staan. Daarom draait deze test niet in CI.

| Scenario | Wat |
|---|---|
| importeren | Projectlocatie (`GIS_SITE`) en de imports: BGT, kadaster, AHN-hoogtelijnen en -punten, of OSM buiten Nederland. Daarna een tekenblad. Gecontroleerd: NLCS-lagen, elementen binnen het importgebied en XDATA. |
| bewerken | Een BGT-object verplaatsen (de XDATA blijft), het naar een andere laag zetten en een laag leegmaken. Ongedaan maken en opnieuw. Een hele import in één stap ongedaan maken. Opnieuw importeren. |
| opslaan | Opslaan en een kopie heropenen: gelijke elementen, lagen en XDATA. |
| eenheden | Een mm-tekening (`data/leeg-mm.dwg`): alles is 1000 × zo groot. |
| relatief | `GIS_SITE … RELATIEF`: het basispunt ligt op 0,0. |
| verhuizen | Een tweede locatie in dezelfde tekening. |

Het rapport staat in `target/e2e/rapport.md`; de tekeningen staan in `target/e2e/`.

## 4. Groot model via de MCP-server

```bash
python3 tests/e2e/mcp_model.py --venster <pid of tekeningnaam>
```

Dit bouwt in een lege metrische tekening hetzelfde als een AI-agent via de MCP-server zou doen. Het bevat:

- **Dordrecht-binnenstad in 3D (1,2 × 1 km):**
  - AHN-terrein met de luchtfoto (299.000 cellen);
  - de BGT in 3D (18.560 objecten);
  - de 3D BAG in LoD 2.2 (2.864 panden).
- **Civiel:**
  - een bestaand terrein (TIN);
  - een rondweg met lengteprofiel, ontwerpprofiel en corridor met 127 dwarsprofielen;
  - een dijk als corridor op kruinhoogte;
  - grondverzet.
- **Controle:** modelinfo, hoeveelheden en NLCS-controle.
- **Beelden:** in `target/e2e/mcp/`.

Start Open CAD Studio voor deze test met `OCS_V4_SNAPSHOT_SEGMENT_SIZE=268435456`, anders kunnen hoeveelheden en NLCS-controle de tekening van ~80 MB niet lezen. Laatste run: 20/20 stappen in ongeveer 110 s. Het grondverzet uit de dwarsprofielen en uit TIN tegen TIN verschilt minder dan 1 %.

## 5. Alle knoppen van de vensters

`tests/e2e/ui_knoppen.py` klikt in een echte Open CAD Studio elke knop van de vensters GIS en Civiel aan en meldt wat er gebeurde: een melding, een andere pagina, een gesloten venster, een veranderd beeld, of niets. In keuzelijsten kiest het een andere optie; houdt die geen knoppen aan, dan zet het de oude terug. Vinkjes gaan aan en weer uit.

De plugins worden daarvoor gebouwd met de feature `ui-test`: die zet de egui-vensters op de toegankelijkheidsbus (AT-SPI). Open CAD Studio draait in een eigen D-Bus-sessie met een eigen toegankelijkheidsbus en een eigen configuratiemap. Je bureaubladinstellingen en je geïnstalleerde plugins blijven dus zoals ze zijn.

```sh
CARGO_TARGET_DIR=target/ui-test cargo build --release -p open-infra --features ui-test
CARGO_TARGET_DIR=target/ui-test cargo build --release -p open-infra-civiel --features ui-test
python3 tests/e2e/ui_knoppen.py                 # beide vensters; --venster gis|civiel, --pagina <naam>
```

Het rapport staat in `target/e2e/ui/knoppen.md`. Nodig: `wmctrl`, `python3-xlib` en de AT-SPI-bindings (`gir1.2-atspi-2.0`).

## 6. BGT-mutatie

`tests/e2e/bgt_mutatie_e2e.py` importeert de BGT rond de Grote Kerk en maakt een ontwerp: een wegdeel krijgt nieuw asfalt (oud object op V, nieuw vlak op R), een tweede wegdeel is gewijzigd (R) en een fietspad is gepland (N). `GIS_BGT_MUTATIE` moet dan 2 nieuwe, 1 gewijzigd en 1 beëindigd object geven. Met `--xsd` wordt de GML ook getoetst aan het IMGeo 2.1.1-schema (lxml).

```sh
cargo build --release -p open-infra && cargo build --release -p open-infra-civiel
python3 tests/e2e/bgt_mutatie_e2e.py --xsd
```

## 7. IFC 4.3-export

De unit-tests van `src/ifc` exporteren een voorbeeldtekening met van alles één:
- een as met ontwerpprofiel, een TIN, een fototerrein en een 3D BAG-pand;
- KLIC, een sondering met twee lagen, verharding met markering, een ontgraving en een corridor;
- teksten en een kaartbeeld.

Ze schrijven `target/ifc-test/voorbeeld.ifc`. `tests/ifc_validate.py` toetst dat bestand met IfcOpenShell:
- aan het schema IFC4X3_ADD2 en de EXPRESS-regels;
- de as-geometrie: de geometriekernel rekent de horizontale en verticale curve uit, en de punten moeten op de segmenten liggen.

In CI gebeurt dat bij elke push.

`tests/ifc_dwg.rs` exporteert een echte tekening rechtstreeks uit de DWG, zonder Open CAD Studio. De projectlocatie komt uit de tekening.

```sh
cargo test --lib -p open-infra ifc
pip install ifcopenshell pytest
python3 tests/ifc_validate.py target/ifc-test/voorbeeld.ifc
cargo test --release --test ifc_dwg -- --ignored --nocapture
OI_IFC_DWG=docs/video/bron/data/dordrecht-showcase.dwg cargo test --release --test ifc_dwg -- --ignored --nocapture
python3 tests/ifc_validate.py target/ifc-test/dordrecht-showcase.ifc
```

## Wat de testset tot nu toe vond

In de plugin, opgelost:
- **Coördinatenstelsel op positie:** Antwerpen, Gent, Brugge, Aken, Kleve, Emmerich en Gronau kregen RD, Lille Lambert 72 en de Noordzee RD. Nu worden grove landsgrenzen voor NL en BE gebruikt.
- **Zoeken:** "Grote Kerk Dordrecht" gaf een straat in Hoorn, "Utrecht Centraal" een busstation in Breda. Nu staan treffers met alle zoekwoorden voorop, en als die er niet zijn wordt in de laatst genoemde plaats gezocht.
- **mm- en cm-tekeningen:** de plugin las INSUNITS niet, want Open CAD Studio geeft die niet als systeemvariabele. Daardoor kwam alles in meters. Nu komt de eenheid uit de tekeningkop.
- **AHN-puntenwolk:** AHN5 is nog niet compleet (bijvoorbeeld bij Loppersum). Een ontbrekende tegel komt nu uit AHN4.
- **AHN buiten Nederland:** gaf een kale WCS-fout. Nu volgt vooraf een duidelijke melding.
- **IMKL-kleuren:** "buisleiding gevaarlijke inhoud" had de kleur van petrochemie; "wees" en "overig" ontbraken. Ze volgen nu de PMKL-handreiking 3.0.

Met de knoppentest gevonden en opgelost:
- **Annuleren** op het locatiescherm deed niets zichtbaars. Nu draait het de wijzigingen terug en sluit het venster, zoals in de FreeCAD-dialogen.
- **Zoeken** met een leeg zoekveld deed niets. De knop staat nu uit tot er iets getypt is.
- **GIS_SITE** nam RD-coördinaten zonder het woord `RD` aan als lengte- en breedtegraad. Nu volgt een melding.

Met het grote model gevonden en opgelost in de plugin:
- **Berichtgrens van 64 MB:** elementen gaan nu in delen van hoogstens ~12 MB naar de host. 3D-objecten zijn lichte MESH-elementen; een POLYFACE-hoekpunt is een volledig record.
- **Hele tekening opvragen:** een snapshot boven 64 MB legt de plugin stil. De civiele commando's lezen nu via een objectindex in de tekening (`OPENINFRA/INDEX`) en per handle. De eenheid staat in `OPENINFRA/UNITS`, zodat de plugin bij het openen van een grote tekening niets groots opvraagt.
- **Gedeelde weergave:** die wordt na het lezen weer gesloten; anders publiceert de host hem na elke wijziging opnieuw.
- **Strepen op water:** de gaten in AHN worden per aaneengesloten gebied vlak gevuld, onder de laagste oever.

In Open CAD Studio (host), nog open. Gemeld bij Open CAD Studio, samengevat onder [RFC #2331](https://github.com/HakanSeven12/OpenCADStudio/issues/2331#issuecomment-6505337895):
- **v2026.43:** de gedeelde documentweergave voor plugins is standaard 16 MB (`OCS_V4_SNAPSHOT_SEGMENT_SIZE`). Groter dan 256 MB lukt niet: de plugin-kant mapt hoogstens 1 GiB voor twee segmenten. ([#2334](https://github.com/HakanSeven12/OpenCADStudio/issues/2334))
- **v2026.43:** plugins krijgen geen INSUNITS en geen tekeningextents (alleen CLAYER, SNAPANG en CTAB) ([#2333](https://github.com/HakanSeven12/OpenCADStudio/issues/2333)). Ook is er geen opvraging per laag of type ([#2335](https://github.com/HakanSeven12/OpenCADStudio/issues/2335)).
- **Ongedaan maken:** een import van de plugin is niet ongedaan te maken. Elementen die een plugin op de achtergrond toevoegt komen niet in de ongedaan-maakstap. ([#2336](https://github.com/HakanSeven12/OpenCADStudio/issues/2336))
- **Zonder venster** (`--http` zonder tekening) verwerkt Open CAD Studio de achtergrondverzoeken van een plugin alleen tijdens een plugin-commando. ([#2337](https://github.com/HakanSeven12/OpenCADStudio/issues/2337))
- **REST:** de routes `/save` en `/undo` nemen geen `document_id` mee in de GUI-modus. De test gebruikt daarom `QSAVE` en `U`. ([#2338](https://github.com/HakanSeven12/OpenCADStudio/issues/2338))
- **Plugin Manager:** installeert per repository één plugin; de Civiel-plugin moet met de hand uit de zip. ([#2339](https://github.com/HakanSeven12/OpenCADStudio/issues/2339))

Gedrag om te beslissen:
- **Opnieuw importeren** in hetzelfde gebied voegt alles nogmaals toe (dubbele elementen).

