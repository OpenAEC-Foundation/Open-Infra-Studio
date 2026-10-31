# Opnamescripts (referentie)

De scripts waarmee alle beelden zijn opgenomen, zoals ze op de eerste pc draaiden (paden daar:
`~/Documenten/Open-Infra-video/work`). Ze sturen een eigen Open CAD Studio aan via de automatisering
(`mcp/open_infra_mcp.py` uit de Open-Infra-repository):
- `rec.py`: starten, venster, `capture`, muis.
- `shots.py`, `record5.py` t/m `record7.py`: camerabewegingen frame voor frame.
- `showcase7.py`: bouwt `dordrecht-showcase.dwg` op.
- `citytest.py`: CityGML/CityJSON-test.
- `stills.py`: scherpe 2×-schermafbeeldingen.

Pas de paden aan voor een andere pc. De montage van de eigen versies (`compose*.py`) hoort niet bij de OpenAEC-stijl
en staat er daarom niet bij.
