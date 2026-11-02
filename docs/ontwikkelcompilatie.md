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

## Nulmeting

De bewaakte nulmeting van 2 november 2026, begonnen om 20:30:51 CET:

| Maat | Huidig ontwikkelprofiel |
| --- | ---: |
| Schone build | 374,40 s |
| Incrementeel, drie herhalingen | 5,35 / 7,17 / 6,41 s |
| Incrementeel, mediaan | 6,41 s |
| Ongewijzigd opnieuw bouwen | 2,83 s |
| Maximale proces-RSS, schoon | 2.634.568 KiB |
| Doelmap na drie wijzigingen | 3.966.204 KiB |

Alle vijf aanroepen eindigden met exitcode 0; de bron is exact hersteld.
Een eerdere verkennende meting had kort overlap met een elders gestarte test;
een eerste profielmeting is om dezelfde reden afgebroken. Deze beide reeksen
worden niet gebruikt voor de vergelijking.

## Profielvergelijking

De bewaakte vergelijking met `CARGO_PROFILE_DEV_DEBUG=line-tables-only` loopt
afzonderlijk. Dit is een tijdelijke omgevingsinstelling; de kandidaat wordt pas
in het manifest gezet na de meting. Definitieve cijfers volgen in deze PR.

De regressiecontrole faalt zonder de profielinstelling, lokaal met exit 1 en in
CI op commit `ca6272623a35b6caf404d7bfcc4e014256df08b3`:
[run 41510581417](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41510581417).
De fout is de ontbrekende `profile.dev.debug`; de voorafgaande Rust-tests slagen.

## Niet gewijzigd

Het releaseprofiel, de hostpin en compiler voor distributie blijven behouden.
De twee plug-ins blijven afzonderlijke builds met hun eigen entrypoints. Een
wissel naar Cargo-resolver 1 is onderzocht maar niet toegepast: de normale
dependencyfeatures veranderen dan ook voor Linux, Windows en macOS. Dat vereist
een afzonderlijke afweging van de gevolgen voor distributie.
