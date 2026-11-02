# Een compiler-cache voor herhaalde schone ontwikkelbuilds

`scripts/build.sh` bouwt vanuit deze checkout en gebruikt een beschikbare
`sccache` als `RUSTC_WRAPPER` nog niet in de omgeving is ingesteld. Een bestaande
waarde, ook een lege waarde, houdt voorrang. Alle argumenten gaan ongewijzigd
naar `cargo build` en de exitcode van Cargo blijft de exitcode van het script.

```sh
./scripts/build.sh --locked -p open-infra
./scripts/build.sh --locked -p open-infra-civiel
RUSTC_WRAPPER= ./scripts/build.sh --locked -p open-infra
```

De derde aanroep kiest expliciet de gewone compilerroute. Zonder `sccache` op
`PATH` gebruikt het script eveneens gewoon Cargo. De gemeten versie is 0.19.0.
Er wordt geen programma geïnstalleerd en geen globale Cargo-configuratie
gewijzigd. Bij automatisch gekozen sccache gebruikt het script standaard de
clientmodus en de fallback bij server-I/O-fouten; bestaande waarden voor die
twee opties blijven behouden. Een wrapper uit een lokale Cargo-configuratie
kan via `RUSTC_WRAPPER` expliciet worden gekozen.

Incrementele compilatie van de eigen crates blijft ingeschakeld. De cache
hergebruikt vooral externe dependencies na het legen van de Cargo-doelmap.
Een bestaande, actuele doelmap blijft de snelste route. Het script verandert
de bestaande installatiescripts en CI-workflows niet.

## Meting op aia09-MP50

Dezelfde Ryzen 7 3700U, 13 GiB RAM, Linux Mint en Rust/Cargo 1.99.1 als in de
[profielmeting](ontwikkelcompilatie.md). Twee Cargo-jobs; hoogstens één
Rust-build tegelijk, met controle op andere compilerprocessen. Downloads waren
vooraf opgehaald, de doelmap was vóór elke schone build leeg en had steeds
hetzelfde absolute pad. De machine wordt gedeeld.

Broncommit: `e0192d71de944f6ffdd18ac0500d0fa715e33d3e`, met het inmiddels
geïntegreerde `line-tables-only`-ontwikkelprofiel. Manifest-SHA-256:
`3f771920817cf25ffd18469405d51ac0dc56425184d7298f1a472f2aaa064257`;
lockfile-SHA-256:
`1ef21ba025c176380573e65be944ad1277bf4fcda4068888075ff9198a669ffe`.
Alle drie builds gebruiken dezelfde broncode en manifesten.

| Schone GIS-ontwikkelbuild | Tijd | Verschil tegenover gewone Cargo |
| --- | ---: | ---: |
| Zonder compiler-cache | 279,10 s | — |
| Compiler-cache aanvankelijk leeg | 317,45 s | 13,7% langer |
| Compiler-cache gevuld | 198,59 s | 28,8% korter |

Alle drie Cargo-aanroepen eindigden met exitcode 0. De bibliotheek was in alle
gevallen exact 190.888.632 bytes, met dezelfde SHA-256:
`17ff9188efc8674e7cc2a65a85690fe257b8901595ea2a46c7b6e607707b37e5`.
Dit betreft de ontwikkelbuild op Linux; voor releasebuilds of andere platforms
is met deze wrapper geen afzonderlijke snelheidsclaim gemaakt.

De gevulde cache gaf 330 hits: 298 Rust-, 18 C/C++- en 14 assemblercompilaties.
Er waren geen cachemissers of cachelees-/schrijffouten. Die hits gaan over de
cachebare compilerverzoeken, niet over al het werk van Cargo. De detailstatistiek
bevat ook vier niet-geslaagde interne compilerverzoeken; de drie uiteindelijke
Cargo-builds zijn geslaagd. De specifieke interne verzoeken zijn niet verder
geanalyseerd.

Sccache bewaart volgens zijn [Rust-documentatie](https://github.com/mozilla/sccache/blob/v0.19.0/docs/Rust.md)
geen compilaties die de systeemlinker aanroepen, waaronder `cdylib` en
`proc-macro`. De eigen plugin en de incrementele eigen crate houden daardoor
compilatiewerk. De eerste vulling is hier aantoonbaar duurder. Eén meting per
variant, op één gedeelde machine: dit is geen belofte dat iedere build versnelt.
Hergebruik tussen verschillende absolute checkoutpaden is niet gemeten.

## Herhaling

De commandoregel was:

```sh
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/ois-sccache-probe/target \
  cargo build --locked --offline -p open-infra --timings
```

De twee cachevarianten voegden `RUSTC_WRAPPER=<sccache-0.19.0>` en
`SCCACHE_CLIENT_SIDE=1` toe. De cache stond in een afzonderlijke lokale map,
met een limiet van 10 GiB en een eigen Unix-socket. Vóór beide cachevarianten
zijn de statistieken op nul gezet. Na iedere build is de doelmap apart bewaard;
daardoor startte de volgende build op exact hetzelfde pad zonder builduitvoer.
De server is na de proef gestopt. `/usr/bin/time` registreerde verstreken tijd,
CPU-tijd, maximale proces-RSS en exitcode.

Ruwe tijden, belasting, manifesthashes, bibliotheekhashes en volledige
sccache-statistieken staan in
[`metingen/lokale-compiler-cache-2026-11-02.json`](metingen/lokale-compiler-cache-2026-11-02.json).
De officiële download van sccache 0.19.0 is vóór gebruik tegen de gepubliceerde
SHA-256 gecontroleerd:
`aa73c7270c997894038c0d8f0d297865f57569e1687883322bc431713d4bb9dd`.

## Regressiecontrole

```sh
python3 -m unittest discover -s tests -p test_build_cache.py -v
shellcheck scripts/build.sh
```

Met een script dat uitsluitend `cargo build` uitvoert faalt de cachekeuzetest,
exit 1: de beschikbare cache wordt niet doorgegeven. Na de cachekeuze slagen
alle vijf tests, exit 0. De tests starten het echte shellscript vanuit een andere
map met gecontroleerde programma's op `PATH`; er wordt geen Rust-compiler gestart.
Ze controleren het ontbreken van sccache, aanwezige sccache, expliciete wrappers
en opt-out, bestaande cacheopties, argumenten met spaties en een Cargo-fout met
exitcode 42. Deze shellcontrole vraagt POSIX met Bash en dirname; op andere
omgevingen wordt hij expliciet overgeslagen.
