# Dependencies hergebruiken bij native compilatie

De native workflow bouwt GIS en Civiel voor Linux, Windows en macOS. Op
2 november 2026 zijn de compilatiestappen op dezelfde commit tweemaal uitgevoerd:
eerst met een lege cache, daarna met de zojuist gemaakte dependencycache.
Ondertekening, notarisatie, distributiearchieven en publicatie zijn hierbij niet
uitgevoerd. Dit onderzoek heeft geen release, versie-tag of productinstallatie
gemaakt.

## Resultaat

Tijden komen uit de jobs-API, van `started_at` tot `completed_at`, inclusief
runneropstart, toolchainkeuze, cacheherstel en cacheopslag, zonder wachtrij.
Eén koude en één warme run per platform; dit zijn waarnemingen, geen garantie
voor iedere runner of wijziging.

| Native compilatiejob | Lege cache | Gevulde cache | Korter |
| --- | ---: | ---: | ---: |
| Ubuntu 22.04, x86_64, 4 logische processors | 657 s | 170 s | 74,1% |
| Windows Server 2025, x86_64, 4 logische processors | 868 s | 305 s | 64,9% |
| macOS 14.8.9, ARM64, 3 logische processors | 767 s | 193 s | 74,8% |

| Stap | Linux koud / warm | Windows koud / warm | macOS koud / warm |
| --- | ---: | ---: | ---: |
| `cargo test --lib --locked -p open-infra` | 199 / 20 s | 249 / 44 s | 267 / 31 s |
| `cargo build --release --locked -p open-infra` | 365 / 49 s | 461 / 92 s | 363 / 54 s |
| `cargo build --release --locked -p open-infra-civiel` | 52 / 52 s | 76 / 92 s | 73 / 55 s |
| Cacheherstel | 1 / 23 s | 5 / 41 s | 1 / 21 s |
| Cacheopslag na afloop | 19 / 0 s | 42 / 0 s | 38 / 0 s |

De cache versnelt vooral de eerste twee Cargo-stappen. Civiel bouwt daarna de
engine met andere features opnieuw; een cache van externe dependencies neemt
dat werk niet weg. In de warme logs worden uitsluitend vier workspacecompilaties
vermeld: de GIS-tests, de GIS-plugin, de engine zonder pluginfeature en Civiel.
`acadrust` en `ocs_plugin_api` worden op alle drie platforms hergebruikt.

## Opzet en herhaling

- [Run 41521579207, poging 1](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41521579207/attempts/1)
  en [poging 2](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41521579207/attempts/2).
- Beide pogingen horen bij PR-head `c5b144d17e76df9258ae2f2e673f144a8c2691fa`.
  De feitelijk uitgecheckte PR-mergecommit is in beide gevallen
  `6566016ac75cffcf6ba22e6dde9b06ec9e00cf03`.
- Rust/Cargo 1.99.1, rustc-commit
  `58cf8f9c35b355199727627d07ae34a4e2846d67`, LLVM 23.1.1.
- Bronboom `src`: `bad293aefc432aae8b1849525c61659c7e32ecbe`;
  `civiel/src`: `2466d705f2b39487b966046e8e13cbb5ae170541`.
- Per platform zijn de uitgecheckte bron-, manifest-, lockfile- en toolchainhashes
  tussen beide pogingen gelijk. Windows checkt tekstbestanden met CRLF uit;
  vergelijk daarom de bestandsdigests binnen hetzelfde platform.
- Vóór poging 1 zijn uitsluitend de twee eerder door deze PR gemaakte native
  meetcaches verwijderd: cache-IDs 9900527258 en 9900499141, beide onder
  `refs/pull/41/merge`. Windows had door een eerdere testfout nog geen cache.
  Caches van main, andere PR's en de testworkflow zijn niet aangepast.
- Logs bevestigen in poging 1 een lege cache en in poging 2 daadwerkelijk
  herstel. Alle zes jobs zijn geslaagd. Een eerdere Windows-run die door een
  bestaande CRLF-fout in een ASC-test faalde, telt niet mee. Die testfout is
  afzonderlijk gerepareerd in PR #42.
- De tijdelijke meetworkflow bevatte precies de drie Cargo-aanroepen uit de
  native workflow en gebruikte dezelfde cacheconfiguratie. De GIS-library is
  vóór de Civiel-build apart bewaard, omdat die tweede build de enginevariant
  zonder GIS-entrypoints schrijft.

Ruwe job- en staptijden, machinegegevens, samenvattingen van de compilatielogs en SHA-256 van
beide libraries staan in
[`metingen/native-compilatie-cache-2026-11-02.json`](metingen/native-compilatie-cache-2026-11-02.json).
De tijdelijke meetworkflow staat in de meetcommit, maar blijft buiten de finale
wijziging.

```sh
gh api repos/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41521579207/attempts/1/jobs
gh api repos/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41521579207/attempts/2/jobs
gh run download 41521579207 --repo OpenAEC-Foundation/Open-Infra-Studio
```

## Cache op main en bij tags

De actie staat na de vastgepinde toolchain en vóór de drie Cargo-stappen.
`shared-key: release` staat los van de bestaande testsleutel. Rustc, architectuur,
manifesten, lockfile en compileromgeving begrenzen het hergebruik. Alleen externe
dependencies worden bewaard; workspaceplugins worden opnieuw
gemaakt. De actie zet `CARGO_INCREMENTAL=0` voor CI. Het releaseprofiel was al
niet incrementeel; lokale instellingen veranderen niet.

[GitHub isoleert caches per branch of tag](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache).
Een nieuwe tag kan de cache van main lezen, maar niet die van een eerdere tag.
Daarom vult `native-cache.yml` de native cache op main wanneer manifesten,
lockfile of de native workflows veranderen. Dit draait op drie platforms,
zonder publicatierechten of organisatiecredentials. Bij een exacte cachehit
worden de drie voorbereidingsbuilds overgeslagen. Gewone wijzigingen in Rust-
bronbestanden starten deze voorbereiding niet: de eigen plugin wordt bij een
release toch opnieuw gebouwd.

Deze voorbereiding vraagt bij een lege cache eenmalig de koude bouwtijd uit de
tabel. De besparing wordt bij volgende native builds gerealiseerd. Voor de
allereerste tag na gewijzigde dependencies moet de voorbereiding eerst klaar
zijn; een cachemiss blijft correct bouwen. PR-caches kunnen main niet vullen.
Caches kunnen verlopen of uit de opslag worden verwijderd. De tabel toont het
hergebruik op dezelfde PR-ref; het vullen op main en herstellen vanuit een
nieuwe tag zijn vóór integratie nog niet als echte release uitgevoerd.

## Controle op productuitvoer

Op Linux zijn GIS en Civiel tussen koud en warm byte voor byte gelijk. Op macOS
zijn beide libraries eveneens byte voor byte gelijk. De ruwe JSON bevat alle
vier gelijke SHA-256-paren en bestandslengtes.

De Windows-libraries zijn even groot, maar hun volledige SHA-256 verschilt.
De eerste meetproef bewaarde uitsluitend hashes. Daarom is dezelfde vaste
broncommit aanvullend tweemaal op één Windows-runner gebouwd: eerst met een
lege targetmap, daarna na `cargo clean -p open-infra -p open-infra-civiel`, met
behoud van de dependencies. Beide keren zijn alle drie Cargo-stappen uitgevoerd.
De [vergelijking is geslaagd](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41530176291).

Voor beide DLL's zijn sectie-indeling, bestandslengte en alle bytes buiten
herkende PE-linkmetadata gelijk. De feitelijke verschillen zijn de COFF- en
debugtijdstempels en de CodeView-PDB-identificatie. Code, data, imports, exports
en relocaties zijn gelijk. De vergelijking wist uitsluitend begrensde metadata-
velden vóór het vergelijken van de volledige bestanden; code en algemene
sectie-inhoud worden niet weggelaten. De interpretatie van PE-tijdstempels en
debugdirectories volgt de [Microsoft PE-specificatie](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format).

De bron van deze aanvullende controle staat in
[meetcommit 0289b6d](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/blob/0289b6d63e38c69798e0b8314f4201249c762526/.github/workflows/compilation-measurement.yml).
De exacte offsets, waarden en genormaliseerde hashes staan in
[`metingen/windows-linkmetadata-2026-11-02.json`](metingen/windows-linkmetadata-2026-11-02.json).
Dit is een afzonderlijke controle op dezelfde broncommit; de oorspronkelijke
Windows-bestanden van de snelheidsmeting zijn niet achteraf opnieuw vergeleken.
De vergelijkingsfunctie is lokaal aanvullend met vaste synthetische PE-data
getoetst: gewijzigde linkidentiteit wordt herkend, een byte buiten metadata en
een gewijzigde sectie-indeling worden afgewezen. Alle drie controles: exit 0.

Compiler, hostpin, features, broncode en releaseprofiel zijn niet veranderd.
De bestaande stagingvolgorde en alle ondertekenings- en publicatiestappen staan
ongewijzigd in de native workflow. De tijdelijke metingen raken die stappen niet.

## Regressiecontrole

```sh
python3 -m unittest discover -s tests -p test_ci_release_cache.py -v
actionlint .github/workflows/release.yml .github/workflows/native-cache.yml
```

Met de oude native workflow faalt de cachecontrole: exit 1, nul cachestappen.
Na herstel van de nieuwe workflow slaagt die controle: exit 0. Het bestand is
met `cp` hersteld en de SHA-256 vóór en na de proef is gelijk:
`4408daab3da91a4d01218975f0b68365619a39eb7564fdeeec833bb59eda5956`.

De tweede controle faalt eerst met exit 1 omdat de voorbereiding op main
ontbreekt. Na toevoeging slagen beide controles, exit 0. Ze bewaken de
platformmatrix, gelijke toolchain en Cargo-stappen, cachevolgorde, de beperkte
triggers en het ontbreken van ondertekening/publicatie in de voorbereiding.
`actionlint` controleert de workflows afzonderlijk.

Aanvullend zijn zowel in de native workflow als in de voorbereiding twee defecten
tijdelijk aangebracht: `if: false` op de cachestap en `cache-targets: false`. Alle
vier proeven worden afgewezen met exit 1. Na elke proef is het bestand met `cp`
teruggezet en is de SHA-256 gecontroleerd; beide gewone controles slagen daarna
weer met exit 0. De ruwe JSON bevat de herstelhashes en exitcodes.
