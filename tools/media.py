# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy>=2", "pillow>=11"]
# ///
"""Render every saver to docs/media/<name>.gif, plus docs/media/tour.gif.

Run as `mise run media [savers...]`. Frames come from dump mode at
1920x1080 with SAVER_PIXEL_ASPECT=180, resampled to the 3.2:1 shape pine's glass
shows. SAVER_HTTP=off: nothing binds a port, and the dump runs unpaced.
"""

import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
from PIL import Image

LANCZOS = Image.Resampling.LANCZOS

ROOT = Path(__file__).resolve().parent.parent
FPS = 12
DUMP_FPS = 24
CLIP_SECS = 4
TOUR_SECS = 2
SIZE = (640, 200)
TOUR_SIZE = (480, 150)
STATIC_FRAMES = 4
CLIP_BUDGET = 1.5e6
TOUR_BUDGET = 12e6

# Savers that build something up before they look like themselves.
WARMUP = {
    "confetti": 25,
    "doodles": 20,
    "sakura": 8,
    "life": 6,
    "worms": 6,
    "strings": 4,
    "lissajous": 6,
    "reaction-diffusion": 10,
    "fractal-tree": 4,
    "doom": 8,
}
DEFAULT_WARMUP = 2

# Savers only a `--features doom` build has. Rendered only when named
# (`mise run media doom`), never in the tour of the default image, and dumped
# paced: their engines run on the wall clock, so an unpaced dump would be a
# second of play stretched over the clip.
FEATURED = {"doom"}


def savers() -> list[str]:
    """Every saver, in README order. docs_check guarantees the index is complete."""
    readme = (ROOT / "README.md").read_text()
    names = re.findall(r"\[`([a-z0-9-]+)`\]\(docs/savers/", readme)
    return list(dict.fromkeys(names))


def doom_build() -> tuple[Path, dict[str, str]]:
    """The doom binary, in its own target dir so the default one stays MIT,
    and the Freedoom WAD it plays."""
    target = ROOT / "target/doom"
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--features", "doom",
         "--target-dir", str(target)],
        cwd=ROOT, check=True,
    )
    wad = subprocess.run([str(ROOT / "tools/freedoom.sh")], cwd=ROOT, check=True,
                         capture_output=True, text=True).stdout.strip()
    return target / "release/screensaver", {"DOOM_WAD": str(ROOT / wad), "SAVER_DUMP_PACED": "1"}


def capture(binary: Path, name: str, scratch: Path, extra: dict[str, str]) -> list[np.ndarray]:
    warm = WARMUP.get(name, DEFAULT_WARMUP)
    dump = scratch / name
    every = DUMP_FPS // FPS
    env = {
        **os.environ,
        **extra,
        "SAVER": name,
        "SAVER_HTTP": "off",
        "SAVER_DUMP": str(dump),
        "SAVER_DUMP_FRAMES": str((warm + CLIP_SECS) * DUMP_FPS),
        "SAVER_DUMP_EVERY": str(every),
        "SAVER_FPS": str(DUMP_FPS),
        "SAVER_PIXEL_ASPECT": "180",
        "SAVER_WIDTH": "1920",
        "SAVER_HEIGHT": "1080",
        "SAVER_ROTATE_SECS": "0",
    }
    run = subprocess.run([binary], env=env, capture_output=True, text=True, check=False)
    if run.returncode:
        sys.exit(f"{name}: dump exited {run.returncode}\n{run.stderr[-2000:]}")
    first = warm * DUMP_FPS
    frames = []
    for ppm in sorted(dump.glob("frame-*.ppm")):
        if int(ppm.stem.split("-")[1]) >= first:
            with Image.open(ppm) as im:
                frames.append(np.asarray(im.convert("RGB").resize(SIZE, LANCZOS)))
    shutil.rmtree(dump)
    return frames


# gifski (quality, lossy): the best that fits the budget wins.
GIF_LADDER = [(90, 80), (70, 55), (50, 40), (40, 25)]


def gif(frames: list[np.ndarray], out: Path, budget: float = CLIP_BUDGET) -> str:
    with tempfile.TemporaryDirectory() as d:
        paths = []
        for i, f in enumerate(frames):
            paths.append(str(Path(d) / f"{i:05}.png"))
            Image.fromarray(f).save(paths[-1])
        for q, lossy in GIF_LADDER:
            run = subprocess.run(
                ["gifski", "--quiet", "--fps", str(FPS), "--quality", str(q),
                 "--lossy-quality", str(lossy), "--motion-quality", str(lossy),
                 "-o", str(out), *paths],
                capture_output=True, text=True, check=False,
            )
            if run.returncode:
                sys.exit(f"{out.name}: gifski exited {run.returncode}\n{run.stderr[-2000:]}")
            if out.stat().st_size <= budget:
                return f"quality {q} lossy {lossy}"
    q, lossy = GIF_LADDER[-1]
    return f"quality {q} lossy {lossy}, OVER the {budget / 1e6:g} MB budget"


def static(n: int, size: tuple[int, int], rng: np.random.Generator) -> list[np.ndarray]:
    """Channel-change snow: coarse grey noise, a flash going in, a rolling hum
    bar, torn scanlines. Drawn at half resolution so LZW has runs to eat."""
    w, h = size[0] // 2, size[1] // 2
    out = []
    for i in range(n):
        snow = rng.integers(0, 6, (h, w)).astype(np.float32) * 51
        if i == 0:
            snow = 140 + snow * 0.45
        bar = (np.arange(h) / h * 2 + i / n) % 1.0
        snow *= 0.55 + 0.45 * np.clip(np.abs(bar - 0.5) * 4, 0, 1)[:, None]
        for y in rng.choice(h, h // 8, replace=False):
            snow[y] = np.roll(snow[y], rng.integers(-w // 6, w // 6))
        g = (np.round(snow / 32) * 32).clip(0, 255).astype(np.uint8)
        g = g.repeat(2, 0).repeat(2, 1)
        out.append(np.stack([g, g, g], -1))
    return out


def mb(p: Path) -> str:
    return f"{p.stat().st_size / 1e6:5.2f} MB"


def main() -> int:
    # Set by the `media` task's usage spec in mise.toml.
    names = shlex.split(os.environ.get("usage_savers", ""))
    binary = ROOT / "target/release/screensaver"
    out = ROOT / "docs/media"
    if not binary.exists():
        sys.exit(f"{binary}: run `mise run build` first")
    indexed = savers()
    want = names or [n for n in indexed if n not in FEATURED]
    unknown = set(want) - set(indexed)
    if unknown:
        sys.exit(f"not in the README's saver index: {sorted(unknown)}")
    tour = [] if names else want
    doom = doom_build() if FEATURED & set(want) else None
    out.mkdir(parents=True, exist_ok=True)
    clips: dict[str, list[np.ndarray]] = {}

    with tempfile.TemporaryDirectory() as scratch:

        def one(name: str) -> None:
            if name in FEATURED:
                assert doom
                frames = capture(doom[0], name, Path(scratch), doom[1])
            else:
                frames = capture(binary, name, Path(scratch), {})
            mode = gif(frames, out / f"{name}.gif")
            if name in tour:
                clips[name] = frames
            print(f"{name:26} {mb(out / f'{name}.gif')}  {mode}", flush=True)

        with ThreadPoolExecutor(max(1, (os.cpu_count() or 2) // 2)) as pool:
            for f in [pool.submit(one, n) for n in want]:
                f.result()

    if tour:
        rng = np.random.default_rng(1)
        frames: list[np.ndarray] = []
        for name in tour:
            frames += static(STATIC_FRAMES, TOUR_SIZE, rng)
            frames += [
                np.asarray(Image.fromarray(f).resize(TOUR_SIZE, LANCZOS))
                for f in clips[name][: TOUR_SECS * FPS]
            ]
        mode = gif(frames, out / "tour.gif", TOUR_BUDGET)
        print(f"tour: {len(tour)} savers, {len(frames) / FPS:.0f} s, "
              f"{mb(out / 'tour.gif')}  {mode}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
