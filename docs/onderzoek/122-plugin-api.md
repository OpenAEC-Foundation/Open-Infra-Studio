# Plugin API compatibility (#122)

OCS latest release checked on 2026-11-13: `v2026.45`, published November 8,
commit `fb0a667e269b6c924233ecd450a11dd7cba40eea`.
Infra `v2026.46.0` was published November 9 but retained the OCS v2026.44 API:
manifest rev `537ab37095b681cab899c1cf48b2927d06e194b9` (tag object), resolved
commit `0e77cae652271d08c7619238e619f9b6668ae17c` in Cargo.lock.
Both GIS and Civiel used this pin.

The old API pulls opencaddocument `d657f945f9d39e00548c76a8f52424881feeed1d`
and acadrust `53e6420c4df4d207409798c2719d32fc6cf2811d` (the latter through
Infra's now-obsolete URL patch). The released host instead locks
opencaddocument `b9e1762969f3897e67e1801a2199445b6cf83b5d` and acadrust
`eababc03c625eaf9cfc5169953d7ee88f2015338`.

API 14 runs the plugin's own runner. `version_info::wire_schema()` hashes the
embedded serde type registry with FNV-1a. `runner.rs` compares it with the
host's OCS_WIRE_SCHEMA before accepting requests. The user's report records
old plugin `08f0b6fd7d9a870e`, released host `c47b6ee4d4051440`.
Compiler equality and a successful plugin build do not prove schema equality.

The previous release workflow checked main ancestry and package versions, ran
library tests, then built, signed and published each platform. No test loaded
the plugins in the released host or compared the wire schema. Old API + old
codec was internally consistent and could pass all those checks.

Both pins now target the released host commit. Cargo update changed only the
API, document and codec sources. The obsolete acadrust URL patch is removed.
No host storage functionality is introduced.

The new pre-build release gate queries GitHub releases/latest (excluding drafts
and prereleases), resolves the tag commit, and requires both manifest pins, the
resolved API source and the three schema-generating package identities to match
the host lockfile. API/network errors stop publication. This is deliberately
stricter than schema equality: even a schema-compatible host release requires
an explicit Infra update. The same gate runs on pull requests. A Rust library
test additionally asserts the actual generated v2026.45 schema, in offline CI
and every release platform. Updating the host pin also requires reviewing that
assertion. Neither check establishes an interactive Windows plugin install.

#103 is related only through the need to select a released host. Its storage
conflict fix requires OCS #2768, merged November 10 as
`1f18cbbc11d8049bd42fdac4b08798cefdac1053`, after v2026.45. This API update does
not install that host fix and cannot unblock #103. A later host release and its
runtime gate remain required there.

Local validation: five Python gate tests pass; the live gate rejects the
unchanged origin/main manifests and accepts these pins and lockfile. No local
Rust/Tauri build or full test suite. CI must establish compile compatibility
and the actual generated schema. No release is created by this change.
