# Infra-bibliotheek Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Een geïntegreerde NLCS-bibliotheek met aansluitbare objecten, configureerbare details, automatisering en zes repository-skills.

**Architecture:** Getypeerde JSON-catalogus, zuivere geometrie- en aansluitengine, hostadapter en bibliotheekpagina. Dezelfde gevalideerde gegevens sturen UI, commando's en MCP aan.

**Tech Stack:** Rust, serde, bestaande NLCS-tabellen en host-API, egui, Python-standaardbibliotheek.

**Spec:** `docs/superpowers/specs/2026-10-29-infra-bibliotheek-design.md`

## Global Constraints

- NLCS 5.0.2; geen verzonnen objectnamen of stilzwijgende fallback.
- Geometrie in meters, materiaalmaten expliciet; projectwaarden behouden hun bronstatus.
- Skills inclusief referenties onder `.agents/skills/` in deze repository.
- Geen kopieën van ontwerpbestanden; generieke parametrische geometrie met bronvermelding.
- Gebruik een beschikbare compiler die de dependency-MS​​RV ondersteunt; geen dependency-downgrades.

## Review Focus

- Diameter-/systeemverschil weigeren vóór mutatie; expliciete reducer kan verschillende maten verbinden.
- Eindpunten en aansluitrichtingen blijven gelijk bij rotatie, lengtewijziging en eenhedenomrekening.
- NaN, ongeldige polygonen en onbekende parameters leveren een fout, geen half resultaat.
- Samengestelde details classificeren ieder component; decoratie telt niet mee als extra hoeveelheid.
- Tekeninggegevens bewaren objectparameters en verbindingen, ook na opslaan/heropenen.

### Task 1: Catalogus en aansluitengine

**Files:** `assets/library/infra.json`, `tools/build_infra.py`, `src/library/infra/{mod,geometry,connect}.rs`, `tests/infra.rs`.
**Interfaces:** `catalog() -> Result<&'static [Item], String>`, `build(&Item, &Placement) -> Result<Built, String>`, `attach(...) -> Result<Placement, String>`.

- [x] Test catalogusdekking, NLCS-koppelingen, parameters, meters/mm, polygonen en aansluitingen vóór implementatie.
- [x] Implementeer alle objectfamilies uit de specificatie met getypeerde parameters, aansluitpunten en meetregels.
- [x] Verifieer rotatie, afschot, systeem-/maatcontrole, reducer en aangesloten objecten.

### Task 2: Plaatsing, hergeneratie en UI

**Files:** `src/library/infra/{host,commands}.rs`, `src/ui/infra.rs`, `src/ui/library.rs`, `src/lib.rs`, `src/hoeveelheden.rs`.
**Interfaces:** catalogus-, plaatsings-, update- en inspectiecommando's met JSON-resultaten; preview gebruikt dezelfde geometrie als plaatsing.

- [x] Test metadata, meetregels en commandoparsing vóór implementatie.
- [x] Integreer punt-, lijn-, vlak- en detailplaatsing, aansluitkeuze en hergeneratie; behoud voertuigbibliotheek.
- [x] Toon bronconflicten, parametergrenzen, aansluitpunten en resultaatmeldingen.
- [x] Voer Rust-tests en afzonderlijke pluginbuilds uit.

### Task 3: MCP, skills en voorbeelden

**Files:** `mcp/open_infra_mcp.py`, `.agents/skills/civiel-*/`, `docs/infra-bibliotheek.md`, `examples/infra-bibliotheek/`.

- [x] Test MCP-argumenten en foutafhandeling vóór uitbreiding.
- [x] Voeg catalogus, plaatsing, inspectie en update toe; documenteer alle werkelijke argumenten.
- [x] Schrijf zes skills met concrete commando's, bron-/eenhedencontrole en begrensde toepasselijkheid.
- [x] Genereer voorbeeldcatalogus, controleer visueel en valideer skills.

### Task 4: Controle en publicatie

- [x] Laat de volledige wijziging onafhankelijk beoordelen; herstel wezenlijke bevindingen.
- [x] Voer Rust-/Python-tests, build, catalogusvalidatie en contentcontrole uit.
- [x] Commit uitsluitend taakbestanden, push de featurebranch en maak een reviewbare pull request.
- [x] Rapporteer inhoud, controles, GitHub-link en eventuele beperkingen van runtimeverificatie.
