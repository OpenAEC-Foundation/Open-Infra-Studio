# Localisations Implementation Plan

**Goal:** Separate interface language, regional data/tool availability and project geography; deliver Netherlands, United States and international profiles.

**Architecture:** A host-free regional model owns settings migration, capabilities and official references. The drawing's Site persists its project region; application settings choose the default/ribbon region. Generic GIS/CAD tools remain available in every region. A country filter in the service catalogue offers regional and global sources without relying on translated search terms. Region selection never rewrites geometry, drawing units or datums.

**Tech Stack:** Rust, serde, egui, existing WMS/WFS/XYZ engine.

- [x] Add profiles and tests for legacy settings migration, language independence, service scope and exact foot conversions.
- [x] Persist project region, expose default/project selection and regional guidance, and migrate the Dutch module switch.
- [x] Add country filters and US catalogue entry points; translate shared navigation labels.
- [x] Verify official US services and document federal/state/county coverage, geodetic transition and CAD-standard boundaries.
- [x] Test, compile both plugins and update the existing grouping PR with the resulting localisation implementation.
