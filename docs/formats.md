# Animation formats

The same frames, 12 fps, encoded four ways by `mise run media --compare`
([`tools/media.py`](../tools/media.py)). Open this page in each browser you care about.

| clip               | G1 WebP | G2 GIF  | G3 APNG  | G4 AVIF |
| ------------------ | ------- | ------- | -------- | ------- |
| `tour`             | 7.92 MB | 9.04 MB | 64.10 MB | 8.79 MB |
| `night-coast-wide` | 1.16 MB | 0.17 MB | 5.86 MB  | 0.19 MB |
| `vinyl-wide`       | 0.14 MB | 0.07 MB | 1.00 MB  | 0.03 MB |

Encoders: G1 WebP: img2webp; lossless, stepping to near-lossless, then mixed, only when over budget; G2 GIF: gifski, quality 70, lossy 55; G3 APNG: Pillow, lossless; G4 AVIF: Pillow/libavif, quality 80. The WebP tour is mixed, q 70: lossless, near-lossless and mixed q 90 all came out over the 12 MB budget.

## tour

The tour, 480x150, 51 savers each after a channel change, 119 s.

**G1 WebP** — 7.92 MB

![G1 WebP: tour](media/formats/tour.webp)

**G2 GIF** — 9.04 MB

![G2 GIF: tour](media/formats/tour.gif)

**G3 APNG** — 64.10 MB

![G3 APNG: tour](media/formats/tour.apng.png)

**G4 AVIF** — 8.79 MB

![G4 AVIF: tour](media/formats/tour.avif)

## night-coast-wide

`night-coast-wide`, a halftone scene, 640x200, 4 s.

**G1 WebP** — 1.16 MB

![G1 WebP: night-coast-wide](media/formats/night-coast-wide.webp)

**G2 GIF** — 0.17 MB

![G2 GIF: night-coast-wide](media/formats/night-coast-wide.gif)

**G3 APNG** — 5.86 MB

![G3 APNG: night-coast-wide](media/formats/night-coast-wide.apng.png)

**G4 AVIF** — 0.19 MB

![G4 AVIF: night-coast-wide](media/formats/night-coast-wide.avif)

## vinyl-wide

`vinyl-wide`, a text piece, 640x200, 4 s.

**G1 WebP** — 0.14 MB

![G1 WebP: vinyl-wide](media/formats/vinyl-wide.webp)

**G2 GIF** — 0.07 MB

![G2 GIF: vinyl-wide](media/formats/vinyl-wide.gif)

**G3 APNG** — 1.00 MB

![G3 APNG: vinyl-wide](media/formats/vinyl-wide.apng.png)

**G4 AVIF** — 0.03 MB

![G4 AVIF: vinyl-wide](media/formats/vinyl-wide.avif)
