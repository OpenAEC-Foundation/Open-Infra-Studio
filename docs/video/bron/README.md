# Bronmateriaal voor een promovideo van Open Infra Studio

Alles wat nodig is om een nieuwe video te maken, bijvoorbeeld in de OpenAEC-stijl met de skill `openaec-video` uit
`OpenAEC-Foundation/product-videos`:

| Map / bestand | Inhoud |
|---|---|
| `VIDEO.md` | Knoppen, wensen van de eigenaar, besluiten |
| `BRIEF.md` | Wat Open Infra Studio uniek maakt, feedback uit de versies v1–v8, bronvermelding |
| `TEKSTEN.md` | Koppen en teksten uit de eerdere versies, de feature-lijst |
| `schermafbeeldingen/2x/` | 20 echte schermafbeeldingen op 3840 × 2160 (kaarten, steden, Nederland, NLCS-tekenblad) |
| `schermafbeeldingen/1x/` | 22 beelden op 1920 × 1080 (3D, KLIC, sonderingen, het programma, de werkvensters) |
| `schermafbeeldingen/website/` | De schermafbeeldingen van open-aec.com/open-infra-studio |
| `data/` | Het 3D-model van Dordrecht (`dordrecht-showcase.dwg`) en alle locaties en diensten |
| `opname/` | De opnamescripts |
| `assets/app-icoon.png` | Het logo (beeldmerk) voor de eindkaart |

**Video's en bewegend beeld** staan niet in git, maar bij de GitHub-release `video/2026-10-promo-bron` van deze
repository:
- `test.mp4`: versie 4, met muziek;
- `open-infra-v8.mp4`: versie 8;
- `opnames-1080p.zip`: alle 31 opgenomen bewegingen als clips.

```bash
gh release download video/2026-10-promo-bron -R OpenAEC-Foundation/Open-Infra
```

De lichte versie van de video van één minuut staat in `../open-infra.mp4`.
