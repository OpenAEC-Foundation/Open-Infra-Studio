# Ontwikkelcompilatie

## Uitgangspunt en werkwijze

Gemeten op aia09-MP50: AMD Ryzen 7 3700U (4 kernen / 8 threads), 13 GiB RAM,
Linux Mint, kernel 7.0.0-38-generic, x86_64. Rust 1.99.1, Cargo 1.99.1,
LLVM 23.1.1. De gebouwde bibliotheek bevestigt LLD 23.1.1 als linker.

De productbronnen en Cargo-manifesten voor de vergelijking zijn gelijk aan
`e7f6fdc2abf4f85de675e08365ae751f07b436c5`. De Git-tree van `src` is
`ac0afe30c3e213d6a1a03a8e6e9cbcbc7ef49b2a`; het lockfile heeft SHA-256
`1ef21ba025c176380573e65be944ad1277bf4fcda4068888075ff9198a669ffe`.
De meetcommits voegen alleen een controle of documentatie toe.

Een schone build heeft een lege doelmap; bronnen zijn vooraf opgehaald.
De OS-bestandscache is niet geleegd. Er zijn twee Cargo-jobs toegestaan binnen
één build. Andere Rust-builds worden vóór en tijdens de meting gecontroleerd;
de meting begint pas na een minuut zonder andere Rust-build. Een nieuwe externe
Rust-build breekt de lopende meting af. De machine blijft gedeeld met andere
processen. De load aan begin en einde staat bij ieder meetpunt in
[`metingen/ontwikkelcompilatie-2026-11-02.json`](metingen/ontwikkelcompilatie-2026-11-02.json).

Commando, met voor iedere variant een andere lege doelmap:

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=<doelmap> \
  /usr/bin/time -f 'elapsed_s=%e user_s=%U system_s=%S max_rss_kib=%M exit=%x' \
  cargo build --locked --offline -p open-infra --timings
```

Na de schone build volgt één ongewijzigde aanroep. Voor drie incrementele
herhalingen wordt de regel `let s = format!("{:+.*}", decimals, z);` in
`src/hoogte.rs` achtereenvolgens gewijzigd naar dezelfde regel met
`z + 0.001`, `z + 0.002` en `z + 0.003`. Iedere wijziging wordt gebouwd. Daarna
wordt het bronbestand byte voor byte hersteld en de SHA-256 gecontroleerd.
De mutaties worden niet gecommit. De grootte van de doelmap wordt na die drie
herhalingen gemeten. RSS is de maximale RSS van het zwaarste afzonderlijke
proces, niet de som van twee gelijktijdige compilerprocessen.

## Resultaten

De bewaakte nulmeting van 2 november 2026 begon om 20:30:51 CET; de
profielvergelijking begon om 20:38:35 CET. De bronnen, manifesthash en lockhash
zijn in deze twee meetreeksen gelijk. Alleen de tijdelijke omgevingsinstelling
`CARGO_PROFILE_DEV_DEBUG=line-tables-only` verschilt.

| Maat | Volledige debug-informatie | Alleen bronregels |
| --- | ---: | ---: |
| Schone build | 374,40 s | 282,76 s |
| Incrementeel, drie herhalingen | 5,35 / 7,17 / 6,41 s | 4,23 / 4,98 / 4,31 s |
| Incrementeel, mediaan | 6,41 s | 4,31 s |
| Ongewijzigd opnieuw bouwen | 2,83 s | 1,91 s |
| Maximale proces-RSS, schoon | 2.634.568 KiB | 2.499.480 KiB |
| Maximale proces-RSS, eerste kleine wijziging | 2.169.816 KiB | 1.112.004 KiB |
| Doelmap na drie wijzigingen | 3.966.204 KiB | 2.688.508 KiB |
| GIS-ontwikkelbibliotheek, schoon | 503.980.552 bytes | 190.887.256 bytes |

In deze vergelijking is de schone build 24,5% korter en de mediaan van de drie
incrementele builds 32,8% korter. De doelmap is 32,2% kleiner. De ongewijzigde
aanroep is één waarneming per variant en geen onderbouwde algemene winstclaim.
Er is één schone build per variant gemeten, op een gedeelde machine; dit is geen
garantie voor alle machines, workloads of belastingniveaus.

Alle tien aanroepen eindigden met exitcode 0; de bron is na beide reeksen exact hersteld.
Een eerdere verkennende meting had kort overlap met een elders gestarte test;
een eerste profielmeting is om dezelfde reden afgebroken. Deze beide reeksen
worden niet gebruikt voor de vergelijking.

## Profiel en verificatie

Het manifest kiest nu `debug = "line-tables-only"` voor `profile.dev`.
De normale testprofielen erven die instelling. Optimalisatie, incrementele
compilatie, assertions, overflowchecks en unwind behouden Cargo's standaard.
Het releaseprofiel behoudt exact `opt-level = 3`.

Bronbestand en regelnummer blijven beschikbaar in backtraces. Types, lokale
variabelen en functieparameters zijn standaard niet volledig beschikbaar in de
debugger. Zie de [Cargo-profielen](https://doc.rust-lang.org/cargo/reference/profiles.html#debug).
Volledige informatie kan per aanroep terug:

```sh
CARGO_PROFILE_DEV_DEBUG=2 cargo build --locked -p open-infra
CARGO_PROFILE_TEST_DEBUG=2 cargo test --lib --locked -p open-infra
```

De gemeten ontwikkelbibliotheken exporteren beide uitsluitend
`ocs_plugin_register` en `ocs_plugin_api_version`. `readelf` vindt de
`.debug_line`-sectie in de kandidaat en `addr2line` vertaalt het adres van
`open_infra::hoogte::nap` naar `src/hoogte.rs:76`. De releasebibliotheken zijn
geen onderdeel van deze groottevergelijking.

De regressiecontrole faalt zonder de profielinstelling, lokaal met exit 1 en in
CI op commit `ca6272623a35b6caf404d7bfcc4e014256df08b3`:
[run 41510581417](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41510581417).
De fout is de ontbrekende `profile.dev.debug`; de voorafgaande Rust-tests slagen.
Na de manifestwijziging slagen deze controle en alle zes lokale Python-tests
met exit 0. `actionlint` en `git diff --check` slagen ook.

De CI voert nu `cargo test --lib --tests --locked -p open-infra` uit zodat het
gewijzigde ontwikkelprofiel voor alle offline Rust-testtargets wordt gevalideerd,
inclusief eerder niet geselecteerde integratietests. Genegeerde netwerktests
blijven genegeerd. IFC-validatie en de afzonderlijke Civiel-check blijven staan.
De CI-uitslag op de uiteindelijke PR-head is het bewijs voor de volledige
Rust-testsuite; de lokale volledige Rust-testsuite is niet uitgevoerd. De PR
vermeldt de concrete run, head en uitslag naast deze lokale compilatiemetingen.

## Niet gewijzigd

Het releaseprofiel, de hostpin en compiler voor distributie blijven behouden.
De twee plug-ins blijven afzonderlijke builds met hun eigen entrypoints. Een
wissel naar Cargo-resolver 1 is onderzocht maar niet toegepast: de normale
dependencyfeatures veranderen dan ook voor Linux, Windows en macOS. Dat vereist
een afzonderlijke afweging van de gevolgen voor distributie.
