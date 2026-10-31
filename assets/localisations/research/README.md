# Regional research profiles

`index.json` lists 20 additional countries researched on 2026-10-31. Each country has its own ISO-code JSON file. These are source-backed research records, not enabled runtime profiles or completed CAD-standard implementations.

Fields describe the official portal, potential data sources, horizontal and vertical references, units, drawing-standard evidence, integration steps and source URLs. `existing_catalogue_services` counts entries in the repository's worldwide service list; it does not certify their availability. `verified_on` is the research date.

`status: "researched"` and `service_tested: false` are intentional. A portal page or datum definition is not a tested import adapter. A null EPSG code means the exact variant was not confirmed from a primary source. Geographic CRS codes must not be mistaken for metric design projections. A null drawing-standard source means no applicable complete civil CAD standard was verified.

Before enabling a country, verify the selected project region, source access/license, projected CRS/axis order, units, datum transformations, real imports and contractual drawing requirements. Preserve regional and historical variants rather than replacing them with one country-wide default.

The Dutch overview is in `docs/localisations-20-landen.md`; the implemented NL/US/international foundation is documented in `docs/localisations.md`.
