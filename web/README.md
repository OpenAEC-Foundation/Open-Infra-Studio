# Open Infra in de browser (`open-infra-web`)

De rekenkern van de Open Infra GIS-workbench als WebAssembly, voor de
webversie van Open CAD Studio. Die webversie laadt geen native plugins; deze
crate compileert daarom **dezelfde code** als de desktop-plugin
(`src/nlcs.rs`, `src/gis/bgt.rs`, `src/gis/vector.rs`, letterlijk via
`#[path]`) naar wasm en levert het resultaat als definities voor
`entities_create` van de besturingskoppeling van Open CAD Studio.

Wat in de browser niet kan (bestanden, SQLite, threads, het netwerk vanuit
Rust) zit er niet in. `crs` en `site` zijn hier een RD-versie (EPSG:28992,
RD-coördinaten in de tekening). Het ophalen bij PDOK doet de host-pagina.

## Bouwen

```
cd web
cargo test                                   # ook de eigen toetsen van nlcs/vector/bgt
wasm-pack build --target web --release --out-dir pkg --out-name open_infra_web
```

## Gebruik (host-pagina)

```js
import init, { bgtGroepen, bgtVerzoeken, bgtTekenen } from './pkg/open_infra_web.js';
await init();
const bbox = [166281, 419260, 166481, 419460];               // RD
const collecties = JSON.parse(bgtGroepen()).filter((g) => g.standaard).flatMap((g) => g.collecties);
const data = {};
for (const v of JSON.parse(bgtVerzoeken(JSON.stringify(collecties), JSON.stringify(bbox)))) {
  let url = v.url; data[v.collectie] = [];
  while (url) { const j = await (await fetch(url)).json(); data[v.collectie].push(...j.features); url = j.links?.find((l) => l.rel === 'next')?.href; }
}
const { entities, lagen } = JSON.parse(bgtTekenen(JSON.stringify(data), JSON.stringify({ bbox, status: 'B', schaal: 500 })));
// → in de geopende tekening:
// {"protocol":1,"op":"entities_create","document_id":…,"request_id":…,"entities": entities}
```

Opties van `bgtTekenen`: `bbox` (verplicht), `status` (`B`/`N`/`T`/`V`),
`discipline` (standaard `XX`), `schaal` (standaard 500), `teksten`
(straatnamen en huisnummers), `knippen` (op het gebied).

Licentie: GPL-3.0-only, zoals de rest van Open Infra.
