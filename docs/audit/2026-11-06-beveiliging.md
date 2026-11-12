# Security audit of main, 6 November 2026

Commit: `841f5fa` (main, merge of #100). Read and probed only; nothing in this document changes code.

## Scope and method

Covered: the MCP server (`mcp/`), every place where a path or file name comes in, the file readers (KLIC/IMKL, KML/KMZ, GPX, GML, CityGML, CityJSON, LandXML, GeoJSON, GeoPackage, Shapefile, raster files, point clouds, QGIS projects, NLCS++ XML), network calls, the bridge to Open CAD Studio, `unsafe`, and process calls.

Checked for: path traversal, XML entities (XXE, entity expansion), unbounded recursion and memory, zip bombs, integer overflow in indexes, `unsafe`, shell and process calls, SQL built from input, and network behaviour (TLS, redirects, server-supplied URLs, body limits).

Not repeated here, because other work fixes them today: MCP over HTTP without a token and without an Origin check, MCP paths outside the work folder, and the seven blocking points from the review of #61 (`graaf.rs`, `nlcspp_host.rs`, `layer.rs`, `hydx.rs`, the cross-section sources, `geoexport.rs` heights). The nesting depth of `nlcspp/src/xml.rs` was already listed as a follow-up in that review; item 3 below adds the other readers with the same cause.

Labels:

- **CONFIRMED**: shown with a probe that was run; the input and the literal output are below.
- **SUSPECTED**: follows from the code, but was not run; the reason is given.
- **BLOCKING**: a user meets it in normal work, data is lost, or it is a security hole.
- **FOLLOW-UP**: rare, needs a crafted file to crash the plug-in, text only, a missing test, or clean-up.

Item 1 is shown by the tests of its fix PR, run in CI on the unchanged code first. Items 3–5 use a small separate program that makes the same library calls as the lines named (roxmltree 0.21.1 and zip 2.6.1, the versions in `Cargo.lock`); it is not part of this PR.

## Summary

| # | Finding | Label |
|---|---|---|
| 1 | KLIC attachment "Openen" hands any file from a delivery to the system, including programs and scripts; on Windows through `cmd /C start` | BLOCKING, CONFIRMED (Windows step SUSPECTED) |
| 2 | Sheet PDFs: layout names read back from the drawing become the PDF path and part of the `-PLOT` command line | BLOCKING, SUSPECTED (no fix PR today) |
| 3 | Deeply nested XML overflows the stack in readers without the GML/CityGML precaution (KML, GPX, WMS/WFS capabilities, KLIC extras, LandXML, BRO, IMRO, QGIS) | FOLLOW-UP, CONFIRMED |
| 4 | Entity expansion where `allow_dtd: true` (capabilities, KLIC extras, CityGML, QGIS, NLCS++) | FOLLOW-UP, CONFIRMED |
| 5 | Zip entries are read into memory without a size limit (KMZ, KLIC delivery, KLIC attachments) | FOLLOW-UP, CONFIRMED |
| 6 | `GIS_KLIC_MAP VOLGEN=ja` reads every `Levering*.zip` in the Downloads folder by default and draws it | FOLLOW-UP, SUSPECTED |
| 7 | The meldnummer from a zip entry name becomes a folder name for attachments | FOLLOW-UP, SUSPECTED |
| 8 | Redirects from https to http are followed; server-advertised URLs are used without a host check | FOLLOW-UP, SUSPECTED |
| 9 | `klic2geo` is a git dependency without `rev` | FOLLOW-UP, CONFIRMED |
| 10 | `tools/nlcspp_schematron.py` downloads and runs code without a pinned version or hash | FOLLOW-UP, CONFIRMED |

Checked and found in order: see the last section.

## Findings

### 1. KLIC attachments: "Openen" starts any file from a delivery

BLOCKING. CONFIRMED for the path from the delivery to `open_file`; the Windows step is SUSPECTED (not run on Windows).

The KLIC panel lists every `Bijlage`/`ExtraDetailinfo` of a delivery with an "Openen" button (`src/ui/app.rs:1840–1856`). The button calls `klic_extra::attachment_file`, which extracts the file named in `bestandLocatie` from the zip, and then `klic_extra::open_file` (`src/gis/klic_extra.rs:311–320`):

```rust
std::process::Command::new("cmd").args(["/C", "start", ""]).arg(path).spawn()   // Windows
std::process::Command::new("open").arg(path).spawn()                            // macOS
std::process::Command::new("xdg-open").arg(path).spawn()                        // Linux
```

`attachment_file` refuses `..`, `\`, `:` and links, but not the file type and not shell characters. So:

- An attachment `run.bat`, `setup.exe`, `x.js`, `x.hta` or `x.lnk` is started, not shown. Because the plug-in extracts the file itself, it carries no "downloaded from the internet" mark, so Windows SmartScreen and Office Protected View do not step in.
- `cmd /C` reads `&`, `|`, `<`, `>`, `^` and `%…%` as syntax. Rust quotes an argument only when it holds a space or tab, so in a path without spaces a name with `&` is split into more than one command. The meldnummer is part of the folder (`klic-bijlagen/<meldnummer>/…`) and comes from the zip entry name, so it can carry the same characters.

Who can do this: anyone whose delivery zip the user opens: a received or forwarded `Levering_*.zip`, or (with item 6) a file that lands in the Downloads folder.

Proof: the tests of PR #113, run in CI on the unchanged code (commit `f3c66f0`, [run 42196967522](https://github.com/OpenAEC-Foundation/Open-Infra-Studio/actions/runs/42196967522)). The extracted-delivery test writes `run.bat`, `setup.exe`, a name with `&`, a name with `%PATH%`, `macro.docm` and a file without a type next to a GI-xml; the zip test puts the same names in a delivery zip. Result on main: `484 passed; 3 failed`, the three new tests:

```
gis::klic_extra::tests::attachments_open_only_as_documents_and_images ... FAILED
  bronnen/KL1081/run.bat must not be opened
gis::klic_extra::tests::unsafe_attachments_are_not_extracted_from_the_zip ... FAILED
  bronnen/KL1081/run.bat must not be extracted
gis::klic_extra::tests::open_file_refuses_programs_and_scripts ... FAILED
  run.bat
```

Fix: PR #113 (`fix/klic-attachment-open`): only PDF and image types are extracted and opened, names with shell characters are refused, and `open_file` checks the same before it starts anything.

### 2. Sheet PDFs: layout names from the drawing become a path and command text

BLOCKING. SUSPECTED: follows from the code; not run (it needs a drawing with a changed record in Open CAD Studio).

`CIV_BLADEN` writes the sheets of an axis to the drawing's record `OPENINFRA/BLADEN` as `axis\tlayout\tpaper`. The layout name is cleaned when it is made (`layout_name`, `src/civiel/bladen.rs:458`), but `CIV_BLADEN_PDF` reads the record back (`sheets_read`, line 484) and uses the name without a check (lines 1180–1192):

```rust
let path = dir.join(format!("{layout}.pdf"));
let _ = std::fs::remove_file(&path);
let line = format!("-PLOT Y {layout} PDF {paper} Millimeters Landscape N Layout 1:1 Center N Y N N N {} N Y", path.display());
bridge::execute(&line)?;
```

The record is part of the DWG/DXF, so a drawing from someone else decides these names. The paper name from the same record goes on the command line too. Two consequences follow from the code:

- A name that is an absolute path, or holds `../`, makes `dir.join` point outside the chosen folder, so a PDF elsewhere is deleted and replaced by the plot.
- A name with spaces or line breaks changes the words on the `-PLOT` command line. What the host then does was not checked.

Direction (no fix PR today): before plotting, accept only names that `layout_name` can make (letters, digits, `-`, `.`, `_`, no leading dot) and only paper names from `PAPERS`; skip other records. `CIV_BLADEN` itself deletes old layouts through `TableOperation::LayoutDelete`, which takes the name as data, so only `CIV_BLADEN_PDF` needs the check.

### 3. Deep XML nesting overflows the stack

FOLLOW-UP (a crafted file or server answer crashes the plug-in; no data is lost). CONFIRMED for the parser call itself.

roxmltree tokenises nested elements recursively, as the comment in `src/gis/files.rs:637` says. `files::gml` and `citygml::document` measure the depth first and give the parser enough stack. The other readers call `Document::parse` directly; a stack overflow aborts the process, and `catch_unwind` does not catch it:

- `src/gis/files.rs:434` KML (and KMZ), `:477` GPX; `kml_geometry` (`:413`) also recurses on `MultiGeometry` itself.
- `src/gis/caps.rs:63` WMS/WFS capabilities (a server answer); `walk` (`:140`) also recurses on `Layer`.
- `src/gis/klic_extra.rs:143` the KLIC extras (every delivery, also in the followed folder).
- `src/civiel/landxml.rs:102` LandXML, `src/gis/bro.rs:170, 280, 625` BRO answers, `src/gis/qgis.rs:45` QGIS projects.

Probe (`xmlprobe depth <n>`): `roxmltree::Document::parse` as in `files.rs:434`, on `n` nested `MultiGeometry` elements, in a thread with 8 MiB of stack (more than a worker thread gets):

```
input 310011 bytes, depth 10000, parsing on a thread with 8 MiB stack
parse -> Ok(Ok(10002))

input 620011 bytes, depth 20000, parsing on a thread with 8 MiB stack
thread '<unknown>' (77026) has overflowed its stack
fatal runtime error: stack overflow, aborting
Command terminated by signal 6
```

A file of 620 kB is enough, even with 8 MiB of stack.

Direction: one shared `parse_xml` that does what `files::gml` does (measure depth, then parse on a thread with enough stack, or refuse beyond a limit), used by every reader.

### 4. Entity expansion where DTDs are allowed

FOLLOW-UP. CONFIRMED.

Five readers parse with `allow_dtd: true`: `src/gis/caps.rs:62`, `src/gis/klic_extra.rs:142`, `src/gis/citygml.rs:346`, `src/gis/qgis.rs:44` and `nlcspp/src/xml.rs:123`. roxmltree has no external entities (no XXE: nothing is read from disk or the network), and it limits nesting to 10 levels and 255 references inside one entity. It does not limit the total size: each top-level reference can expand to 255 times the largest entity again.

Probe (`xmlprobe dtd 100 <refs>`): `parse_with_options` with `allow_dtd: true` as in `klic_extra.rs:142`, on a document with a 100 kB entity, a second entity of 255 references to it, and 1 or 8 references to the second one. Each further reference (3 bytes) adds 25.5 MB.

```
input 100818 bytes
parse -> ok, text of <r>: 25500000 bytes, 0.1 s          (1 reference, max RSS 52 MB)

input 100839 bytes
parse -> ok, text of <r>: 204000000 bytes, 1.6 s         (8 references, max RSS 401 MB)
```

Direction: refuse a DTD with `<!ENTITY` where it is not needed (KLIC, NLCS++), or check the size of the internal subset before parsing; for WMS 1.1.1 and QGIS the DOCTYPE is needed, but internal entities are not.

### 5. Zip entries are read without a size limit

FOLLOW-UP. CONFIRMED for KMZ.

`files::kmz` (`src/gis/files.rs:470`), `klic_extra::gi_xml` (`src/gis/klic_extra.rs:211`), `klic_extra::attachment_file` (`:303`) and `klic2geo::convert_zip_bytes` (crates-warehouse) read a whole entry with `read_to_string`/`read_to_end`. Deflate compresses runs of the same byte about 1000 to 1; an allocation failure aborts the process.

Probe (`xmlprobe kmz 256`): `by_name(..).read_to_string` as in `files.rs:470`, on a KMZ holding 256 MiB of spaces (kept small; the machine had 2 GB free):

```
kmz on disk: 260888 bytes, kml inside: 268435456 bytes
read_to_string -> 268435456 bytes in memory               (max RSS 274 MB)
```

Direction: `.take(limit)` on each entry (for example 1 GiB for the GI-xml, 100 MiB for an attachment) and a clear message when it is reached.

### 6. The followed KLIC folder is the Downloads folder by default

FOLLOW-UP (a design point). SUSPECTED: follows from the code, not run in Open CAD Studio.

`GIS_KLIC_MAP` without `MAP=` uses the folder of last time, else `~/Downloads` (`src/klic_map.rs:118`). Every `Levering*.zip` and every `GI_gebiedsinformatielevering*.xml` there (and one folder down) that the drawing has not read is drawn into the drawing; with `VOLGEN=ja` every minute while Open CAD Studio runs. A browser can save a file there without a question. Such a file is then parsed (items 3–5) and its cables and pipes are drawn as if the Kadaster delivered them.

Direction: follow only a folder the user chose, and check that a delivery's meldnummer matches a request the user made, or ask before drawing a new one.

### 7. The meldnummer becomes a folder name

FOLLOW-UP. SUSPECTED (Windows paths, not run).

The meldnummer comes from the zip entry name `GI_gebiedsinformatielevering_<nr>_…` (klic2geo) and is used as `config_dir()/klic-bijlagen/<meldnummer>` (`src/ui/app.rs:1849`). On Windows `..\..\x` or `C:\x` are a valid meldnummer there, so the attachment folder can be made anywhere. The file inside still goes into a hashed subfolder, so the name of a file cannot be chosen. Item 1's fix refuses shell characters in the whole path before opening; the folder itself is not cleaned.

Direction: use only `[A-Za-z0-9-]` of the meldnummer for the folder.

### 8. Redirects and server-advertised URLs

FOLLOW-UP. SUSPECTED.

The shared agent (`src/gis/http.rs:16`) keeps ureq's defaults: up to 10 redirects, also from https to http (`https_only` is false). The GetMap/GetFeature URL from capabilities (`src/gis/caps.rs:78`), the OGC API `next` links (`src/gis/ogcapi.rs:38`) and the 3D BAG `next` links (`src/gis/bag3d.rs:134`) are used without checking the host. Only the planning API turns redirects off and checks the base (`src/gis/planning.rs:450, 461`), which is right because it sends a key. Three catalogue services are plain http (`assets/servers/world.json`). The other requests carry no key, so the risk is a downgraded or redirected map image, not a leak.

Direction: `https_only(true)` for services that start on https.

### 9. `klic2geo` without `rev`

FOLLOW-UP. CONFIRMED.

`Cargo.toml:44`: `klic2geo = { git = "https://github.com/OpenAEC-Foundation/crates-warehouse" }`. CI builds with `--locked`, so `Cargo.lock` (`6e8738c`) holds it today, but any `cargo update` takes whatever is on that repository's default branch. The other git dependencies have a `rev`.

### 10. Development tools download code without a pin

FOLLOW-UP. CONFIRMED.

`tools/nlcspp_schematron.py:33–37` clones `mijnaansluiting/Inhoudelijke-validaties` without a commit and downloads a SchXslt zip without a hash, and then runs the XSLT. It is not shipped, but it runs on a developer's machine.

## Checked and found in order

- **MCP server** beyond the known items: `oi_command` runs any host command by design, so the HTTP token is the whole boundary; result files use a random ID; the plug-in accepts only `[A-Za-z0-9_-]{1,64}` as `ID=` (`src/civiel/api.rs:166`). The HTTP handler reads `Content-Length` bytes without a cap; that falls under the token fix.
- **`unsafe`**: one block, `prctl(PR_SET_PTRACER)` in test builds only (`src/ui/mod.rs:111`).
- **Processes**: only `open_file` (item 1). No shell in the plug-in or the MCP server.
- **SQL**: GeoPackages are opened read-only, the table name is quoted, statements are prepared one at a time (`src/gis/files.rs:344–358`). The export uses fixed table names and cleaned column names (`src/geoexport.rs:358, 405`).
- **Binary readers**: WKB and the GeoPackage header check counts against the remaining bytes (`src/gis/files.rs:221–339`); CityJSON and 3D BAG use `.get` for vertex indexes; the ASCII grid uses `checked_mul`; TIFF uses the crate's default limits; HTTP bodies stop at 256 MiB (`src/gis/http.rs:14`).
- **JSON**: serde_json's recursion limit of 128 bounds the recursive GeoJSON and planning walkers.
- **Paths from commands** (`PAD=`, `MAP=`, `BESTAND=`): the user's own input; outside MCP this is intended. Names from WMS layers and server IDs go through `slug` (`src/gis/raster.rs:160`).
- **KLIC attachment paths**: `..`, `\`, `:`, links and already extracted files are handled well (`attachment_target`); item 1 is about type and shell characters only.
- **Credentials**: `servers.json` is written with mode 0600; the planning key stays in memory and is sent to its own base only; QGIS imports drop passwords.
- **Point clouds**: a range answer shorter than asked panics in `RangeReader::read` (`src/gis/pointcloud.rs:154`); the command's `catch_unwind` turns it into an error, and the server is a fixed one.
- **Temporary files**: all `temp_dir()` use is in tests.
- **Text in exports**: IFC strings are escaped as STEP `\X2\`; the XML writers escape `& < > "`. `nlcspp/src/xml.rs` and `src/bgt_mutatie.rs` do not drop control characters, which XML 1.0 forbids (not a security point).

## Not checked

- Items 3–5 through the plug-in's own functions: the probe program makes the same library calls, but a full build of the plug-in did not finish on the shared machine today.
- Windows itself: items 1 and 7 follow from `cmd /C start` and Windows path rules, not from a run on Windows.
- Open CAD Studio's handling of extra tokens on a command line (item 2).
- The external crates' own readers (shapefile, las/laz, copc-rs, e57, pointcloud-core, tiff), beyond how this code calls them.
- The host's shared-memory view (`bincode` without a size limit in `src/bridge.rs:372`): the host writes it, so it is trusted here.
- `civiel/src/lib.rs` command parsing in depth.

This audit was done with the help of AI.
