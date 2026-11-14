"""Synthetic laser-line frames for the benchmarks.

The Python harness and the Rust benchmarks must use the same frames. This
module makes each frame from a small set of numbers, with a counter-based
random generator that is easy to copy in Rust. Only integer operations,
IEEE-754 double `+ - * /`, `sqrt`, `floor` and one `exp` per column are used,
so a Rust copy gives the same bytes. Each frame also has a SHA-256 to check
this.

Frame specification (version 1)
-------------------------------

Inputs: width W, height H, centre c (column, in pixels, the centre of column x
is at x), sigma s (pixels), background B, amplitude A, read noise r, shot gain
g, seed S, frame index n. The line is vertical: all rows have the same signal.
The tool averages the rows, so its profile runs along the columns.

For each column x in 0..W-1:

    signal(x) = B + A * exp(-0.5 * ((x - c) / s)^2)
    sd(x)     = sqrt(r*r + g * signal(x))

For each pixel p = y*W + x (u64 arithmetic wraps at 2^64):

    key = splitmix64((S << 32) ^ n)
    h   = splitmix64(key + p)
    u_j = (((h >> (16*j)) & 0xFFFF) + 0.5) / 65536        for j = 0, 1, 2, 3
    z   = ((((u_0 + u_1) + u_2) + u_3) - 2.0) * sqrt(3.0)  (mean 0, variance 1)
    v   = signal(x) + sd(x) * z
    Y   = clamp(floor(v + 0.5), 0, 255) as u8

    splitmix64(x):
        z = x + 0x9E3779B97F4A7C15
        z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9
        z = (z ^ (z >> 27)) * 0x94D049BB133111EB
        return z ^ (z >> 31)

The frame is the luma (Y) plane, row by row, with no padding. For a camera
frame in NV12, the chroma plane is all 128 (grey).

Limits: there is no speckle, no lens, no tilt and no motion of the line. The
noise changes from frame to frame; the line does not. Real frames from the
camera cover what this model leaves out.
"""

from __future__ import annotations

import hashlib
import math
from dataclasses import asdict
from dataclasses import dataclass
from dataclasses import replace

import numpy as np
import numpy.typing as npt

SPEC_VERSION = 1

_GOLDEN = np.uint64(0x9E3779B97F4A7C15)
_MIX1 = np.uint64(0xBF58476D1CE4E5B9)
_MIX2 = np.uint64(0x94D049BB133111EB)
_SQRT3 = math.sqrt(3.0)


@dataclass(frozen=True)
class Scene:
    """The numbers that make one frame. Defaults: a 1920x1080 sensor and a wide line."""

    width: int = 1920
    height: int = 1080
    centre: float = 960.0
    sigma: float = 80.0
    background: float = 16.0
    amplitude: float = 160.0
    read_noise: float = 2.0
    shot_gain: float = 0.5
    seed: int = 1

    def at(self, centre: float) -> Scene:
        return replace(self, centre=centre)

    def to_dict(self) -> dict[str, float | int]:
        return asdict(self)


def splitmix64(x: npt.NDArray[np.uint64]) -> npt.NDArray[np.uint64]:
    """SplitMix64 on each element. Wraps at 2^64."""
    z = x + _GOLDEN
    z = (z ^ (z >> np.uint64(30))) * _MIX1
    z = (z ^ (z >> np.uint64(27))) * _MIX2
    return z ^ (z >> np.uint64(31))


def signal(scene: Scene) -> npt.NDArray[np.float64]:
    """The noise-free signal of each column."""
    x = np.arange(scene.width, dtype=np.float64)
    d = (x - scene.centre) / scene.sigma
    return scene.background + scene.amplitude * np.exp(-0.5 * (d * d))


def frame(scene: Scene, index: int) -> npt.NDArray[np.uint8]:
    """The luma plane of frame `index` of `scene`, shape (height, width)."""
    w, h = scene.width, scene.height
    key = splitmix64(np.array([(scene.seed << 32) ^ index], dtype=np.uint64))[0]
    p = np.arange(w * h, dtype=np.uint64)
    hsh = splitmix64(p + key)

    mask = np.uint64(0xFFFF)
    z = ((hsh & mask).astype(np.float64) + 0.5) / 65536.0
    for j in (1, 2, 3):
        z += (((hsh >> np.uint64(16 * j)) & mask).astype(np.float64) + 0.5) / 65536.0
    z = (z - 2.0) * _SQRT3
    z = z.reshape(h, w)

    s = signal(scene)
    sd = np.sqrt(scene.read_noise * scene.read_noise + scene.shot_gain * s)
    v = s[None, :] + sd[None, :] * z
    return np.clip(np.floor(v + 0.5), 0, 255).astype(np.uint8)


def sha256(y: npt.NDArray[np.uint8]) -> str:
    return hashlib.sha256(np.ascontiguousarray(y).tobytes()).hexdigest()
