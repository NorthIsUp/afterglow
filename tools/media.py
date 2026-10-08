# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy", "pillow>=11.3"]
# ///
"""Render every saver to docs/media/<name>.webp, plus docs/media/tour.webp.

Run as `mise run media [--compare] [savers...]`. Frames come from dump mode at
1920x1080 with SAVER_PIXEL_ASPECT=180, resampled to the 3.2:1 shape pine's glass
shows. SAVER_HTTP=off: nothing binds a port, and the dump runs unpaced.
`--compare` also writes docs/media/formats/: the tour and two clips as WebP,
GIF (gifski), APNG and AVIF, from identical frames.
"""

import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image

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
    "reaction-diffusion-wide": 10,
    "fractal-tree": 4,
    "fractal-tree-wide": 4,
}
DEFAULT_WARMUP = 2
# Every cell changes every frame; a shorter loop beats a muddier one.
CLIP = {"ascii": 2}


def savers() -> list[str]:
    """Every saver, in README order. docs_check guarantees the index is complete."""
    readme = (ROOT / "README.md").read_text()
    names = re.findall(r"\[`([a-z0-9-]+)`\]\(docs/savers/", readme)
    return list(dict.fromkeys(names))


def tour_order(names: list[str]) -> list[str]:
    """One half per pair, the `-wide` one, as rotation picks it by default."""
    s = set(names)
    return [n for n in names if f"{n}-wide" not in s]


def capture(binary: Path, name: str, scratch: Path) -> list[np.ndarray]:
    warm = WARMUP.get(name, DEFAULT_WARMUP)
    dump = scratch / name
    every = DUMP_FPS // FPS
    env = {
        **os.environ,
        "SAVER": name,
        "SAVER_HTTP": "off",
        "SAVER_DUMP": str(dump),
        "SAVER_DUMP_FRAMES": str((warm + CLIP.get(name, CLIP_SECS)) * DUMP_FPS),
        "SAVER_DUMP_EVERY": str(every),
        "SAVER_FPS": str(DUMP_FPS),
        "SAVER_PIXEL_ASPECT": "180",
        "SAVER_WIDTH": "1920",
        "SAVER_HEIGHT": "1080",
        "SAVER_ROTATE_SECS": "0",
    }
    subprocess.run([binary], env=env, check=True, stderr=subprocess.DEVNULL)
    first = warm * DUMP_FPS
    frames = []
    for ppm in sorted(dump.glob("frame-*.ppm")):
        if int(ppm.stem.split("-")[1]) >= first:
            with Image.open(ppm) as im:
                frames.append(np.asarray(im.convert("RGB").resize(SIZE, Image.LANCZOS)))
    shutil.rmtree(dump)
    return frames


def pngs(frames: list[np.ndarray], d: str) -> list[str]:
    paths = []
    for i, f in enumerate(frames):
        paths.append(str(Path(d) / f"{i:05}.png"))
        Image.fromarray(f).save(paths[-1])
    return paths


# Lossless first so the glyphs stay crisp; each later rung only if the one
# before it is over budget.
WEBP_LADDER = [
    ["-lossless"],
    ["-near_lossless", "40"],
    ["-mixed", "-q", "90"],
    ["-mixed", "-q", "70"],
    ["-lossy", "-q", "50"],
    ["-lossy", "-q", "30"],
]


def webp(frames: list[np.ndarray], out: Path, budget: float = CLIP_BUDGET) -> str:
    with tempfile.TemporaryDirectory() as d:
        paths = pngs(frames, d)
        for rung in WEBP_LADDER:
            subprocess.run(["img2webp", "-loop", "0", "-d", str(1000 // FPS), *rung, *paths,
                            "-o", str(out)], check=True, capture_output=True)
            if out.stat().st_size <= budget:
                break
    return " ".join(rung)


def apng(frames: list[np.ndarray], out: Path) -> None:
    ims = [Image.fromarray(f) for f in frames]
    ims[0].save(out, format="PNG", save_all=True, append_images=ims[1:],
                duration=1000 // FPS, loop=0, optimize=True)


def avif(frames: list[np.ndarray], out: Path) -> None:
    ims = [Image.fromarray(f) for f in frames]
    ims[0].save(out, save_all=True, append_images=ims[1:], duration=1000 // FPS,
                loop=0, quality=80, speed=4)


def gif(frames: list[np.ndarray], out: Path) -> None:
    with tempfile.TemporaryDirectory() as d:
        paths = pngs(frames, d)
        subprocess.run(["gifski", "--quiet", "--fps", str(FPS), "--quality", "70",
                        "--lossy-quality", "55", "--motion-quality", "55",
                        "-o", str(out), *paths], check=True)


FORMATS: dict[str, tuple[str, Callable[[list[np.ndarray], Path], object]]] = {
    "webp": ("webp", webp),
    "gif": ("gif", gif),
    "apng": ("png", apng),
    "avif": ("avif", avif),
}
COMPARE = ["night-coast-wide", "vinyl-wide"]


def static(n: int, size: tuple[int, int], rng: np.random.Generator) -> list[np.ndarray]:
    """Channel-change snow: coarse grey noise, a flash going in, a rolling hum
    bar, torn scanlines. Drawn at half resolution so the encoders have runs."""
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


@dataclass
class Args:
    names: list[str]
    compare: bool
    jobs: int
    bin: Path
    out: Path


def mb(p: Path) -> str:
    return f"{p.stat().st_size / 1e6:5.2f} MB"


def main() -> int:
    # Set by the `media` task's usage spec in mise.toml.
    names = shlex.split(os.environ.get("usage_savers", ""))
    args = Args(
        names=names,
        compare=os.environ.get("usage_compare") == "true",
        jobs=max(1, (os.cpu_count() or 2) // 2),
        bin=ROOT / "target/release/screensaver",
        out=ROOT / "docs/media",
    )
    if not args.bin.exists():
        sys.exit(f"{args.bin}: run `mise run build` first")
    every = savers()
    want = args.names or every
    unknown = set(want) - set(every)
    if unknown:
        sys.exit(f"not in the README's saver index: {sorted(unknown)}")
    tour = [] if args.names else tour_order(every)
    args.out.mkdir(parents=True, exist_ok=True)
    clips: dict[str, list[np.ndarray]] = {}

    with tempfile.TemporaryDirectory() as scratch:

        def one(name: str) -> None:
            frames = capture(args.bin, name, Path(scratch))
            mode = webp(frames, args.out / f"{name}.webp")
            if name in tour or name in COMPARE:
                clips[name] = frames
            print(f"{name:26} {mb(args.out / f'{name}.webp')}  {mode}", flush=True)

        with ThreadPoolExecutor(args.jobs) as pool:
            for f in [pool.submit(one, n) for n in want]:
                f.result()

    movies: dict[str, list[np.ndarray]] = {}
    if tour:
        rng = np.random.default_rng(1)
        frames: list[np.ndarray] = []
        for name in tour:
            frames += static(STATIC_FRAMES, TOUR_SIZE, rng)
            frames += [
                np.asarray(Image.fromarray(f).resize(TOUR_SIZE, Image.LANCZOS))
                for f in clips[name][: TOUR_SECS * FPS]
            ]
        mode = webp(frames, args.out / "tour.webp", TOUR_BUDGET)
        print(f"tour: {len(tour)} savers, {len(frames) / FPS:.0f} s, "
              f"{mb(args.out / 'tour.webp')}  {mode}")
        movies["tour"] = frames

    if args.compare:
        movies |= {n: clips[n] for n in COMPARE if n in clips}
        cmp = args.out / "formats"
        cmp.mkdir(exist_ok=True)
        jobs = [(n, fmt) for n in movies for fmt in FORMATS]
        with ThreadPoolExecutor(args.jobs) as pool:
            def enc(job: tuple[str, str]) -> None:
                n, fmt = job
                ext, fn = FORMATS[fmt]
                out = cmp / f"{n}.{fmt}.{ext}" if fmt == "apng" else cmp / f"{n}.{ext}"
                fn(movies[n], out)
                print(f"compare {n:18} {fmt:5} {mb(out)}", flush=True)
            list(pool.map(enc, jobs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
