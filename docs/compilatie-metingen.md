# Compilatie meten

Uitgangspunt op 2 november 2026: `e7f6fdc2abf4f85de675e08365ae751f07b436c5`,
gelijk aan de toen opgehaalde `origin/main`. Dit onderzoek verandert geen
releaseprofiel, plug-incontract of productcode.

## Machine en lokale meting

- Machine: aia09-MP50, Linux Mint, kernel `7.0.0-38-generic`, x86_64.
- Processor: AMD Ryzen 7 3700U, 4 kernen / 8 threads; 13 GiB RAM.
- `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, Cargo 1.99.1, LLVM 23.1.1.
- De machine wordt gedeeld. Er draait tijdens een meting hoogstens één
  Rust-build. Andere processen en de belasting worden bij de meting vastgelegd.
- Een schone build betekent een lege doelmap, met vooraf opgehaalde bronnen;
  downloadtijd en netwerkvariatie tellen zo niet als compilerwinst.
- Het commando voor de GIS-plug-in is
  `CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=<lege map> cargo build --locked --offline -p open-infra --timings`.
- Voor de incrementele meting wordt in `src/hoogte.rs` tijdelijk
  `decimals, z` vervangen door `decimals, z + 0.0`, hetzelfde commando opnieuw
  uitgevoerd en de bron daarna exact teruggezet. Dit is een wijziging in een
  bestaande functie; de tijdelijke wijziging wordt niet gecommit.
- Noteer naast de verstreken tijd ook de exitcode, maximale proces-RSS en
  doelmapgrootte. Een afgebroken of mislukte build telt niet als versnelling.

De lokale nulmeting volgt zodra de al actieve Rust-build op aia klaar is.

## Bestaande CI

Tijden uit de GitHub Actions jobs-API, dus inclusief de stappen binnen de job,
maar zonder de wachtrij vóór de job. De testjobs hebben geen Cargo-cache.

| Run | Job / stap | Tijd |
| --- | --- | ---: |
| [41242678829](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41242678829), main `e7f6fdc` | offline, Ubuntu 22.04 | 345 s |
| dezelfde run | `cargo test --lib --locked -p open-infra` | 202 s |
| dezelfde run | `cargo test --test locaties --locked -p open-infra` | 17 s |
| dezelfde run | `cargo test --test planning --test planning_render --locked -p open-infra` | 3 s |
| dezelfde run | `cargo check --locked -p open-infra-civiel` | 86 s |
| [41392686417](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41392686417), main `e7f6fdc` | online, Ubuntu 22.04 | 998 s |
| [40974256968](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/40974256968), oudere release `5786534` | Linux | 413 s |
| dezelfde run | Windows | 643 s |
| dezelfde run | macOS | 720 s |

De online job bevat aanvragen aan openbare diensten; zijn totale duur is geen
zuivere compilatiemeting. De releasejobs zijn historische meetgegevens. Voor dit
onderzoek is geen release gestart en zijn geen assets gepubliceerd.

De offline log van run 41242678829 vermeldt 140 geslaagde en 8 genegeerde
unittests, 9 geslaagde en 1 genegeerde locatietest, en 9 geslaagde planningtests.
De compiler meldt respectievelijk 3m21s, 16,51s, 2,41s en 1m25s. Die tijden zijn
iets korter dan de afgeronde staptijden in de tabel.

Opvragen:

```sh
gh run list --repo OpenAEC-Foundation/Open-Infra-Studio --limit 25
gh api repos/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/41242678829/jobs
gh api repos/OpenAEC-Foundation/Open-Infra-Studio/actions/jobs/124659627064/logs
```

## Afbakening

De twee plug-ins worden afzonderlijk gebouwd: de GIS-plug-in gebruikt de feature
`plugin`, Civiel gebruikt de engine zonder die entrypoints. Een gezamenlijke
workspacebuild kan features verenigen en is daarom geen geldige vervanging.
Het vastgepinde hostcontract, de compiler voor distributie en het releaseprofiel
blijven behouden. Het ondertekenen en notariseren uit PR #14 valt buiten dit werk.
