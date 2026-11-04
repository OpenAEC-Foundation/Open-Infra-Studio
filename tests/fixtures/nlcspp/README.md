# NLCS++ example deliveries

The official example deliveries of NLCS++ Netbeheer V11 (DWG + XML), from
https://github.com/mijnaansluiting/NLCSpp (`examples/`), Apache License 2.0:

- `alliander-demo.*`: *Alliander demodataset_622025_1712*
- `enexis-voorbeeld.*`: *Enexis voorbeeld_622025_1505*

`tests/nlcspp.rs` reads them as golden files: every object of the XML, and
the layer the grammar gives each one against the layer of the element with
the same handle in the DWG.

`xsd/` holds the NLCS++ V12.1 schemas (`schemas/V12.1` of the same
repository): `tests/test_nlcspp_xsd.py` validates the V12.1 files Open Infra
writes (made by `tests/nlcspp.rs` in `target/nlcspp/`) against them. The GML
3.2.1 schemas they import come from schemas.opengis.net.
