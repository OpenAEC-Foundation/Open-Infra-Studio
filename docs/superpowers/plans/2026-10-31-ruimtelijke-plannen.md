# Ruimtelijke plannen — uitvoering

Doel: issue #15 oplossen met een planzoeker op locatie, volledige IMRO-planobjecten, herkenbare thematische weergave en lokale GML-import zonder API-sleutel.

Architectuur: eigen hostvrije IMRO/GML- en API-v4-lezer, bestaande GIS-transformaties, CAD-adapter en apart werkbenchpaneel. De QGIS-plugin dient als functionele referentie; het bestaande overheidscontract bepaalt HTTP-verzoeken. Nieuwe DSO-omgevingsdocumenten en legacy IMRO-plannen blijven onderscheiden.

- [x] Reproduceer het verlies van IMRO-attributen/geometriereferenties in de generieke GML-route met synthetische data.
- [x] Bouw en test model, lokale referentieresolutie, polygonen met gaten, thematische stijlen en API-responsen/paginering.
- [x] Voeg API-sleutel uitsluitend als invoer in geheugen of lokale omgevingsvariabele toe; geen sleutels in tekeningen, URL's of logging.
- [x] Voeg locatiezoeken, planselectie, downloaden/inladen, lokale GML-keuze, thematische stijlen en metadata toe aan UI/commando's/MCP.
- [x] Controleer openbare WMS als sleutelvrije kaartreferentie en documenteer actualiteit en reikwijdte.
- [x] Voer tests/builds uit en laat code beoordelen. Publicatie koppelt de oplossing met `Fixes #15`; sluiting volgt de integratie op main.

Aanvullende QGIS-mogelijkheden worden apart bijgehouden in #17; deze uitbreiding implementeert geen XML-import van serviceverbindingen.
