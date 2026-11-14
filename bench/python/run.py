"""Benchmark harness for the Python tool (Phase 0 of PLAN.md).

It runs the tool's own code from `src/` and does not change it.

    python bench/python/run.py synthetic [--quick] [--memory-seconds 60]
    python bench/python/run.py live [--seconds 30] [--camera 0] [--save-frames 0]

`synthetic` puts frames from `synthetic.py` through the real `FrameWorker`,
`SampleWorker` and `Core.received_sample`. It measures the time of each step,
the highest frame rate, accuracy, repeatability, linearity, scale and memory.

`live` starts the real `Core` with a camera. It counts the frames that the
camera gives and the frames that the worker processes, and it measures CPU use.
It can also save raw frames. Close the GUI tool first: it holds the camera and
uses CPU.

Each run writes a JSON file to `bench/results/`.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
import sys
import time
from dataclasses import replace
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(HERE))

SENSOR_WIDTH_MM = 5.9  # the tool's default
UNITS = "mm"
ANALYSER_HEIGHT = 300  # height of the analyser widget, in pixels; any value works
DEFAULT_SMOOTHING = 50  # the tool's default
SUBSAMPLES = 10  # the tool's default
OUTLIERS = 30  # the tool's default, percent


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--platform", default="offscreen", help="Qt platform plugin (default: offscreen)")
    p.add_argument("--out", type=Path, help="JSON output file (default: bench/results/python-<mode>-<date>.json)")
    sub = p.add_subparsers(dest="mode", required=True)

    s = sub.add_parser("synthetic", help="synthetic frames through the real worker code")
    s.add_argument("--quick", action="store_true", help="fewer frames, for a smoke test")
    s.add_argument("--memory-seconds", type=float, default=60.0, help="length of the memory run (600 = 10 min)")

    live = sub.add_parser("live", help="the real Core with a real camera")
    live.add_argument("--camera", type=int, default=0, help="camera index, as in the tool's list")
    live.add_argument("--warmup", type=float, default=5.0, help="seconds before the measurement starts")
    live.add_argument("--seconds", type=float, default=30.0, help="length of the measurement")
    live.add_argument("--save-frames", type=int, default=0, help="save this many raw luma frames after the run")
    live.add_argument("--frames-dir", type=Path, default=ROOT / "bench" / "data" / "live", help="where to save frames")
    return p.parse_args()


ARGS = parse_args()
os.environ["QT_QPA_PLATFORM"] = ARGS.platform

import numpy as np  # noqa: E402
import psutil  # noqa: E402
from PySide6 import __version__ as pyside_version  # noqa: E402
from PySide6.QtCore import QObject  # noqa: E402
from PySide6.QtCore import QSize  # noqa: E402
from PySide6.QtCore import QTimer  # noqa: E402
from PySide6.QtCore import Slot  # noqa: E402
from PySide6.QtGui import QImage  # noqa: E402
from PySide6.QtGui import QPixmap  # noqa: E402
from PySide6.QtGui import QTransform  # noqa: E402
from PySide6.QtMultimedia import QVideoFrame  # noqa: E402
from PySide6.QtMultimedia import QVideoFrameFormat  # noqa: E402
from PySide6.QtWidgets import QApplication  # noqa: E402

import qimage2ndarray  # noqa: E402  (after PySide6: it uses the Qt binding that is already loaded)

import synthetic  # noqa: E402

APP = QApplication(sys.argv)

from src.Core import Core  # noqa: E402
from src.Core import samples_recalc  # noqa: E402
from src.curves import fit_gaussian  # noqa: E402
from src.DataClasses import FrameData  # noqa: E402
from src.DataClasses import Sample  # noqa: E402
from src.utils import get_units  # noqa: E402
from src.Workers import FrameWorker  # noqa: E402
from src.Workers import SampleWorker  # noqa: E402

UM_PER_PX = SENSOR_WIDTH_MM * 1000.0 / 1920.0
PROC = psutil.Process()


# --------------------------------------------------------------------------- helpers


def now_ns() -> int:
    return time.perf_counter_ns()


def stats(values: list[float] | np.ndarray) -> dict[str, float]:
    a = np.asarray(values, dtype=np.float64)
    if a.size == 0:
        return {"n": 0}
    return {
        "n": int(a.size),
        "mean": float(a.mean()),
        "sd": float(a.std(ddof=1)) if a.size > 1 else 0.0,
        "p50": float(np.percentile(a, 50)),
        "p99": float(np.percentile(a, 99)),
        "min": float(a.min()),
        "max": float(a.max()),
    }


def rss_mb() -> float:
    return PROC.memory_info().rss / 1e6


def nv12_frame(y: np.ndarray) -> QVideoFrame:
    """A new NV12 QVideoFrame with luma `y` and grey chroma, as a camera gives it.

    Each call makes a new frame: QVideoFrame keeps the image from toImage(), so a
    frame that is used again does not show the conversion cost.
    """
    h, w = y.shape
    f = QVideoFrame(QVideoFrameFormat(QSize(w, h), QVideoFrameFormat.PixelFormat.Format_NV12))
    if not f.map(QVideoFrame.MapMode.WriteOnly):
        raise RuntimeError("cannot map QVideoFrame")
    plane_y = np.frombuffer(f.bits(0), dtype=np.uint8, count=f.mappedBytes(0)).reshape(h, f.bytesPerLine(0))
    plane_y[:, :w] = y
    plane_uv = np.frombuffer(f.bits(1), dtype=np.uint8, count=f.mappedBytes(1))
    plane_uv[:] = 128
    del plane_y, plane_uv
    f.unmap()
    return f


class CoreStub:
    """The parts of `Core` that `FrameWorker` and `Core.received_sample` read."""

    def __init__(self, worker: FrameWorker | None = None) -> None:
        self.zero = 0.0
        self.sensor_width = SENSOR_WIDTH_MM
        self.units = UNITS
        self.frameWorker = worker
        self.setting_zero_sample = False
        self.replacing_sample = False
        self.replacing_sample_index = 0
        self.samples: list[Sample] = []
        self.OnSampleComplete = self  # received_sample calls self.OnSampleComplete.emit()

    def emit(self) -> None:
        pass


class Rig:
    """The real FrameWorker and SampleWorker, connected as `main.py` connects them."""

    def __init__(self) -> None:
        self.core = CoreStub()
        self.worker = FrameWorker(parent_obj=self.core)
        self.core.frameWorker = self.worker
        self.worker.analyser_smoothing = DEFAULT_SMOOTHING
        self.worker.analyser_widget_height = ANALYSER_HEIGHT
        self.sample_worker = SampleWorker()
        self.emitted: list[Any] = []
        self.samples: list[float] = []
        # main.py:258
        self.worker.OnCentreChanged.connect(self.sample_worker.sample_in)
        self.worker.OnCentreChanged.connect(self.emitted.append)
        self.sample_worker.OnSampleReady.connect(self.samples.append)

    @property
    def k(self) -> int:
        return int(self.worker.analyser_smoothing)

    def process(self, y: np.ndarray) -> tuple[float, Any]:
        """One frame through FrameWorker.setVideoFrame. Returns (float centre, emitted value)."""
        self.worker.setVideoFrame(nv12_frame(y))
        return self.worker.centre, self.emitted[-1]

    def take_sample(self, frames: list[np.ndarray]) -> tuple[float, list[float]]:
        """One sample (SUBSAMPLES frames) through SampleWorker. Returns (sample, float centres)."""
        n_before = len(self.samples)
        self.sample_worker.start(len(frames), OUTLIERS)
        floats = []
        for y in frames:
            c, _ = self.process(y)
            floats.append(c)
        assert len(self.samples) == n_before + 1, "SampleWorker did not give a sample"
        return self.samples[-1], floats


def trimmed_mean(values: list[float], outlier_percent: float) -> float:
    """SampleWorker.sample_in's outlier removal (Workers.py:66-75), on floats."""
    a = np.asarray(values, dtype=np.float64)
    n_outliers = int(len(a) * (outlier_percent / 100.0) / 2.0)
    first, last = n_outliers, -n_outliers if n_outliers > 0 else None
    return float(np.mean(a[a.argsort()][first:last]))


class FramePool:
    """Frames made once and used again, for the timing and memory runs."""

    def __init__(self, scene: synthetic.Scene, n: int) -> None:
        self.frames = [synthetic.frame(scene, i) for i in range(n)]
        self.i = 0

    def next(self) -> np.ndarray:
        y = self.frames[self.i % len(self.frames)]
        self.i += 1
        return y


# --------------------------------------------------------------------------- synthetic sections

STEP_NAMES = [
    "convert",  # Workers.py:140 toImage + Grayscale8
    "mean",  # :142
    "feed_pixmap",  # :147
    "smooth",  # :151-152
    "rescale",  # :155-158
    "scope_image",  # :162-169
    "scope_pixmap",  # :172-184
    "fit",  # :190
    "rest",  # :186-187, 192-202
]


def timed_steps(worker: FrameWorker, frame: QVideoFrame) -> list[int]:
    """A copy of FrameWorker.setVideoFrame (Workers.py:137-206) with a timer after each step.

    The signals are not emitted. Keep this copy the same as the original.
    """
    t = [now_ns()]
    image = frame.toImage().convertToFormat(QImage.Format_Grayscale8)
    t.append(now_ns())
    histo = np.mean(qimage2ndarray.raw_view(image), axis=0)
    t.append(now_ns())
    pixmap = QPixmap.fromImage(image).transformed(QTransform().rotate(-90))  # noqa: F841
    t.append(now_ns())
    kernel = np.ones(2 * worker.analyser_smoothing + 1) / (2 * worker.analyser_smoothing + 1)
    histo = np.convolve(histo, kernel, mode="valid")
    t.append(now_ns())
    min_value, max_value = histo.min(), histo.max()
    histo = ((histo - min_value) * (255.0 / (max_value - min_value))).clip(0, 255).astype(np.uint8)
    t.append(now_ns())
    scopeData = np.zeros((histo.shape[0], 256), dtype=np.uint8)
    histo_nn = np.nan_to_num(histo)
    for i, intensity in enumerate(histo):
        scopeData[i, : int(intensity)] = 128
    t.append(now_ns())
    qimage = QImage(scopeData.data, scopeData.shape[1], scopeData.shape[0], scopeData.strides[0], QImage.Format_Grayscale8)
    a_pix = QPixmap.fromImage(qimage)
    a_pix = a_pix.transformed(QTransform().scale(1, -1))
    t.append(now_ns())
    centre = fit_gaussian(histo_nn)
    t.append(now_ns())
    width = histo_nn.shape[0]
    p = worker.parent_obj
    a_sample = int(worker.analyser_widget_height - centre * worker.analyser_widget_height / width) if centre else 0
    a_zero, a_text = 0, ""
    if p.zero and centre:
        a_zero = int(worker.analyser_widget_height - p.zero * worker.analyser_widget_height / width)
        a_text = get_units(p.units, (p.sensor_width / width) * (centre - p.zero))
    FrameData(a_pix, a_sample, a_zero, a_text)
    t.append(now_ns())
    return [b - a for a, b in zip(t, t[1:])]


def section_timing(rig: Rig, pool: FramePool, n: int) -> dict[str, Any]:
    rig.core.zero = 900.0  # after Zero, the worker also makes the text; this is the normal case
    for _ in range(10):  # warm up
        rig.process(pool.next())

    per_step: dict[str, list[float]] = {s: [] for s in STEP_NAMES}
    for _ in range(n):
        frame = nv12_frame(pool.next())
        for name, ns in zip(STEP_NAMES, timed_steps(rig.worker, frame)):
            per_step[name].append(ns / 1e6)

    total_e2e = []
    for _ in range(n):
        frame = nv12_frame(pool.next())
        t0 = now_ns()
        rig.worker.setVideoFrame(frame)
        total_e2e.append((now_ns() - t0) / 1e6)

    steps = {k: stats(v) for k, v in per_step.items()}
    step_sum = float(sum(s["mean"] for s in steps.values()))
    e2e = stats(total_e2e)
    rig.core.zero = 0.0
    return {
        "smoothing": rig.k,
        "frames": n,
        "steps_ms": steps,
        "steps_sum_mean_ms": step_sum,
        "set_video_frame_ms": e2e,
        "max_fps": 1000.0 / e2e["mean"],
        "note": "One thread, no camera, no painting, no GIL contention. setVideoFrame is the real method; "
        "steps_ms comes from a copy of it with timers.",
    }


def section_fit_cases(rig: Rig, scene: synthetic.Scene, n: int) -> dict[str, Any]:
    """Time per step with the line at the centre, near the edge, and with no line (noise only)."""
    cases = {
        "centre": scene,
        "edge": scene.at(0.9 * scene.width),
        "no_line": replace(scene, amplitude=0.0),
    }
    out: dict[str, Any] = {"smoothing": rig.k, "frames": n}
    for name, sc in cases.items():
        frames = [synthetic.frame(sc, 50_000 + i) for i in range(min(n, 8))]
        per: dict[str, list[float]] = {s: [] for s in STEP_NAMES}
        for i in range(n):
            for step, ns in zip(STEP_NAMES, timed_steps(rig.worker, nv12_frame(frames[i % len(frames)]))):
                per[step].append(ns / 1e6)
        centres = [rig.process(y)[0] + rig.k for y in frames]
        out[name] = {
            "line_px": sc.centre,
            "amplitude": sc.amplitude,
            "fit_ms": stats(per["fit"]),
            "total_ms": stats(np.sum([per[s] for s in STEP_NAMES], axis=0)),
            "found_px": stats(centres),
        }
    return out


def errors_at(rig: Rig, scene: synthetic.Scene, centres: list[float], frames_per: int, seed0: int) -> list[dict[str, Any]]:
    """Per centre: errors of the float centre and of the emitted value, in input pixels."""
    out = []
    k = rig.k
    for ci, c in enumerate(centres):
        sc = scene.at(c)
        f_err, e_err, failures, emitted_types = [], [], 0, set()
        for j in range(frames_per):
            centre, emitted = rig.process(synthetic.frame(sc, seed0 + ci * 1000 + j))
            emitted_types.add(type(emitted).__name__)
            if centre == 0:
                failures += 1
                continue
            f_err.append(centre + k - c)
            e_err.append(emitted + k - c)
        out.append(
            {
                "true_px": c,
                "float_err_px": stats(f_err),
                "emitted_err_px": stats(e_err),
                "failures": failures,
                "emitted_type": sorted(emitted_types),
            }
        )
    return out


def section_subpixel(rig: Rig, scene: synthetic.Scene, frames_per: int) -> dict[str, Any]:
    centres = [960.0 + j / 10.0 for j in range(10)] + [961.0]
    rows = errors_at(rig, scene, centres, frames_per, seed0=10_000)
    return {"smoothing": rig.k, "frames_per_centre": frames_per, "rows": rows}


def section_linearity(rig: Rig, scene: synthetic.Scene, frames_per: int, sigmas: list[float]) -> dict[str, Any]:
    w = scene.width
    centres = [round(0.05 * w + i * (0.90 * w) / 19, 3) for i in range(20)]
    out = {"smoothing": rig.k, "frames_per_centre": frames_per, "by_sigma": []}
    for s in sigmas:
        rows = errors_at(rig, replace(scene, sigma=s), centres, frames_per, seed0=20_000 + int(s))
        means = [r["float_err_px"]["mean"] for r in rows if r["float_err_px"]["n"]]
        out["by_sigma"].append(
            {
                "sigma_px": s,
                "rows": rows,
                "max_abs_float_err_px": float(np.max(np.abs(means))) if means else None,
                "failures": int(sum(r["failures"] for r in rows)),
            }
        )
    return out


def section_repeatability(rig: Rig, scene: synthetic.Scene, n_samples: int) -> dict[str, Any]:
    c = 960.37
    sc = scene.at(c)
    k = rig.k
    tool, floats_path = [], []
    for s in range(n_samples):
        frames = [synthetic.frame(sc, 30_000 + s * SUBSAMPLES + j) for j in range(SUBSAMPLES)]
        sample, floats = rig.take_sample(frames)
        tool.append(sample + k - c)
        floats_path.append(trimmed_mean(floats, OUTLIERS) + k - c)
    ts, fs = stats(tool), stats(floats_path)
    return {
        "true_px": c,
        "smoothing": k,
        "samples": n_samples,
        "subsamples": SUBSAMPLES,
        "outliers_percent": OUTLIERS,
        "tool_err_px": ts,
        "tool_sd_um": ts["sd"] * UM_PER_PX,
        "float_path_err_px": fs,
        "float_path_sd_um": fs["sd"] * UM_PER_PX,
        "note": "tool = the value that SampleWorker gives (after Signal(int)); "
        "float_path = the same outlier removal on the float centres (the #43 fix).",
    }


def measure_mm(rig: Rig, scene: synthetic.Scene, zero_at: float, sample_at: float, k_zero: int, k_sample: int, seed: int) -> dict[str, Any]:
    """Zero, then Take Sample, through SampleWorker and the real Core.received_sample."""
    core = rig.core
    core.samples = []
    rig.sample_worker.OnSampleReady.connect(lambda v: Core.received_sample(core, v))
    try:
        rig.worker.analyser_smoothing = k_zero
        core.setting_zero_sample = True
        rig.take_sample([synthetic.frame(scene.at(zero_at), seed + j) for j in range(SUBSAMPLES)])
        rig.worker.analyser_smoothing = k_sample
        core.setting_zero_sample = False
        rig.take_sample([synthetic.frame(scene.at(sample_at), seed + 100 + j) for j in range(SUBSAMPLES)])
    finally:
        rig.sample_worker.OnSampleReady.disconnect()
        rig.sample_worker.OnSampleReady.connect(rig.samples.append)
    reported = core.samples[-1].y
    true_mm = (sample_at - zero_at) * SENSOR_WIDTH_MM / scene.width
    result = {
        "zero_px": zero_at,
        "sample_px": sample_at,
        "smoothing_at_zero": k_zero,
        "smoothing_at_sample": k_sample,
        "data_width": rig.worker.data_width,
        "reported_um": reported * 1000.0,
        "true_um": true_mm * 1000.0,
        "error_um": (reported - true_mm) * 1000.0,
    }
    if true_mm:
        result["error_percent"] = 100.0 * (reported - true_mm) / true_mm
    core.zero = 0.0
    core.samples = []
    return result


def section_scale(rig: Rig, scene: synthetic.Scene) -> dict[str, Any]:
    rows = []
    for i, k in enumerate((0, 50, 200)):
        rows.append(measure_mm(rig, scene, 700.0, 1200.0, k, k, seed=40_000 + i * 1000))
    moved = measure_mm(rig, scene, 960.0, 960.0, 50, 0, seed=45_000)
    rig.worker.analyser_smoothing = DEFAULT_SMOOTHING
    return {
        "same_smoothing": rows,
        "smoothing_changed_after_zero": moved,
        "note": "The line moves 500 px between Zero and Take Sample. In the last row it does not move, "
        "but smoothing goes from 50 to 0 after Zero.",
    }


def section_memory(rig: Rig, pool: FramePool, seconds: float) -> dict[str, Any]:
    for _ in range(20):
        rig.process(pool.next())
    t0 = time.perf_counter()
    t_next = t0
    times, rss, frames = [], [], 0
    while True:
        t = time.perf_counter()
        if t >= t_next:
            times.append(t - t0)
            rss.append(rss_mb())
            t_next += 1.0
        if t - t0 >= seconds:
            break
        rig.process(pool.next())
        frames += 1
    tt, rr = np.array(times), np.array(rss)
    half = len(tt) // 2
    slope = float(np.polyfit(tt[half:], rr[half:], 1)[0] * 60.0) if len(tt) - half >= 3 else None
    return {
        "seconds": seconds,
        "frames": frames,
        "rss_start_mb": float(rr[0]),
        "rss_end_mb": float(rr[-1]),
        "rss_peak_mb": float(rr.max()),
        "slope_second_half_mb_per_min": slope,
        "trace": [[round(a, 2), round(b, 2)] for a, b in zip(tt, rr)],
    }


def section_graph(cycles: int, per_cycle: int) -> dict[str, Any]:
    """The matplotlib graph: time per update, and memory over clear-and-fill cycles (PR #41)."""
    from src.Widgets import Graph

    samples: list[Sample] = []
    graph = Graph(samples)
    graph.set_units(UNITS)
    graph.set_mode("Raw")
    times: dict[int, list[float]] = {}
    rss_after_cycle = []
    rss0 = rss_mb()
    for _ in range(cycles):
        samples[:] = []
        graph.update_graph()
        for i in range(per_cycle):
            samples.append(Sample(x=len(samples), y=0.01 * np.sin(i / 5.0) + 0.0001 * i))
            samples_recalc(samples)
            t0 = now_ns()
            graph.update_graph()
            times.setdefault(len(samples), []).append((now_ns() - t0) / 1e6)
        rss_after_cycle.append(rss_mb())
    marks = [n for n in (1, 3, 10, 30, 50, 100, 200) if n in times]
    return {
        "cycles": cycles,
        "samples_per_cycle": per_cycle,
        "update_ms_at_n": {str(n): stats(times[n]) for n in marks},
        "rss_before_mb": rss0,
        "rss_after_each_cycle_mb": rss_after_cycle,
    }


def run_synthetic() -> dict[str, Any]:
    q = ARGS.quick
    scene = synthetic.Scene()
    print(f"scene: {scene}", flush=True)

    t = time.perf_counter()
    pool = FramePool(scene, 4 if q else 12)
    print(f"frame pool: {len(pool.frames)} frames in {time.perf_counter() - t:.1f} s", flush=True)
    hashes = {str(i): synthetic.sha256(f) for i, f in enumerate(pool.frames[:4])}

    rig = Rig()
    result: dict[str, Any] = {"scene": scene.to_dict(), "spec_version": synthetic.SPEC_VERSION, "frame_sha256": hashes}

    print("timing ...", flush=True)
    result["timing"] = section_timing(rig, pool, 30 if q else 300)
    print("fit cases ...", flush=True)
    result["fit_cases"] = section_fit_cases(rig, scene, 10 if q else 50)
    print("sub-pixel sweep ...", flush=True)
    result["subpixel"] = section_subpixel(rig, scene, 3 if q else 10)
    print("linearity ...", flush=True)
    result["linearity"] = section_linearity(rig, scene, 2 if q else 5, [20.0, 80.0, 200.0])
    print("repeatability ...", flush=True)
    result["repeatability"] = section_repeatability(rig, scene, 5 if q else 30)
    print("scale ...", flush=True)
    result["scale"] = section_scale(rig, scene)
    mem_s = 5.0 if q else ARGS.memory_seconds
    print(f"memory ({mem_s:.0f} s) ...", flush=True)
    result["memory"] = section_memory(rig, pool, mem_s)
    print("graph ...", flush=True)
    result["graph"] = section_graph(1 if q else 3, 20 if q else 100)
    return result


# --------------------------------------------------------------------------- live


class LiveCounter(QObject):  # type: ignore
    def __init__(self) -> None:
        super().__init__()
        self.t0 = time.perf_counter()
        self.delivered = 0
        self.processed = 0
        self.first_delivered_s: float | None = None
        self.first_processed_s: float | None = None
        self.frame_info: dict[str, Any] | None = None
        self.save_left = 0
        self.saved: list[dict[str, Any]] = []
        self.save_dir: Path | None = None
        self.centres: list[tuple[float, int]] = []  # (seconds since start, centre as the tool emits it)

    @Slot(QVideoFrame)  # type: ignore
    def on_delivered(self, frame: QVideoFrame) -> None:
        self.delivered += 1
        if self.first_delivered_s is None:
            self.first_delivered_s = time.perf_counter() - self.t0
            fmt = frame.surfaceFormat()
            self.frame_info = {
                "pixel_format": str(frame.pixelFormat()),
                "size": [frame.width(), frame.height()],
                "handle_type": str(frame.handleType()),
                "color_space": str(fmt.colorSpace()),
                "color_range": str(fmt.colorRange()),
                "frame_rate": fmt.frameRate(),
            }
        if self.save_left > 0 and self.save_dir is not None:
            self.save_left -= 1
            self.saved.append(save_luma(frame, self.save_dir, len(self.saved)))

    @Slot(int)  # type: ignore
    def on_processed(self, centre: int) -> None:
        self.processed += 1
        t = time.perf_counter() - self.t0
        self.centres.append((t, centre))
        if self.first_processed_s is None:
            self.first_processed_s = t


def save_luma(frame: QVideoFrame, out_dir: Path, i: int) -> dict[str, Any]:
    """Save the luma plane as raw 8-bit rows (`.y8`). Formats with no luma plane go through toImage."""
    name = f"frame_{i:04d}.y8"
    w, h = frame.width(), frame.height()
    fmt = frame.pixelFormat()
    planar = fmt in (
        QVideoFrameFormat.PixelFormat.Format_NV12,
        QVideoFrameFormat.PixelFormat.Format_NV21,
        QVideoFrameFormat.PixelFormat.Format_YUV420P,
        QVideoFrameFormat.PixelFormat.Format_YV12,
    )
    packed = {
        QVideoFrameFormat.PixelFormat.Format_YUYV: 0,
        QVideoFrameFormat.PixelFormat.Format_UYVY: 1,
    }
    source = "luma plane"
    if (planar or fmt in packed) and frame.map(QVideoFrame.MapMode.ReadOnly):
        bpl = frame.bytesPerLine(0)
        raw = np.frombuffer(frame.bits(0), dtype=np.uint8, count=frame.mappedBytes(0)).reshape(-1, bpl)[:h]
        y = raw[:, :w].copy() if planar else raw[:, packed[fmt] : 2 * w : 2].copy()
        del raw
        frame.unmap()
    else:
        img = frame.toImage().convertToFormat(QImage.Format_Grayscale8)
        y = qimage2ndarray.raw_view(img).copy()
        source = "toImage Grayscale8 (no luma plane)"
    (out_dir / name).write_bytes(np.ascontiguousarray(y).tobytes())
    if i == 0:
        QImage(y.data, w, h, w, QImage.Format_Grayscale8).save(str(out_dir / "frame_0000.png"))
    return {"file": name, "width": w, "height": h, "source": source, "sha256": synthetic.sha256(y)}


def run_live() -> dict[str, Any]:
    counter = LiveCounter()
    core = Core()
    core.subsamples = SUBSAMPLES
    core.outliers = OUTLIERS
    core.units = UNITS
    core.sensor_width = SENSOR_WIDTH_MM
    core.frameWorker.analyser_smoothing = DEFAULT_SMOOTHING
    core.frameWorker.analyser_widget_height = ANALYSER_HEIGHT
    core.captureSession.videoSink().videoFrameChanged.connect(counter.on_delivered)
    core.frameWorker.OnCentreChanged.connect(counter.on_processed)

    cameras = core.get_cameras()
    if not cameras:
        raise SystemExit("no camera")
    print(f"cameras: {cameras}; using {ARGS.camera}", flush=True)
    core.set_camera(ARGS.camera)

    result: dict[str, Any] = {"cameras": cameras, "camera_index": ARGS.camera}
    marks: dict[str, Any] = {}

    def start_window() -> None:
        marks["d0"], marks["p0"] = counter.delivered, counter.processed
        marks["t0"] = time.perf_counter()
        marks["cpu0"] = PROC.cpu_times()
        print("measuring ...", flush=True)

    def end_window() -> None:
        dt = time.perf_counter() - marks["t0"]
        cpu1 = PROC.cpu_times()
        cpu_s = (cpu1.user - marks["cpu0"].user) + (cpu1.system - marks["cpu0"].system)
        delivered = counter.delivered - marks["d0"]
        processed = counter.processed - marks["p0"]
        w0 = marks["t0"] - counter.t0
        window = [(round(t - w0, 4), c) for t, c in counter.centres if t >= w0]
        cf = core.camera.cameraFormat()
        result.update(
            {
                "camera_format": {
                    "resolution": [cf.resolution().width(), cf.resolution().height()],
                    "pixel_format": str(cf.pixelFormat()),
                    "min_fps": cf.minFrameRate(),
                    "max_fps": cf.maxFrameRate(),
                },
                "frame_info": counter.frame_info,
                "startup_s": {"first_delivered": counter.first_delivered_s, "first_processed": counter.first_processed_s},
                "window_s": dt,
                "delivered": delivered,
                "processed": processed,
                "delivered_fps": delivered / dt,
                "processed_fps": processed / dt,
                "processed_ratio": processed / delivered if delivered else None,
                "cpu_percent_of_one_core": 100.0 * cpu_s / dt,
                "cpu_count": psutil.cpu_count(),
                "rss_mb": rss_mb(),
                "threads": PROC.num_threads(),
                "centre_px": stats([float(c) for _, c in window]),
                "centres": window,
                "note": "No window and no painting: the GUI tool uses more CPU than this. "
                "centres: (seconds into the window, centre as emitted through Signal(int), profile index).",
            }
        )
        if ARGS.save_frames > 0:
            ARGS.frames_dir.mkdir(parents=True, exist_ok=True)
            counter.save_dir = ARGS.frames_dir
            counter.save_left = ARGS.save_frames
            print(f"saving {ARGS.save_frames} frames to {ARGS.frames_dir} ...", flush=True)
            wait_for_saves()
        else:
            APP.quit()

    def wait_for_saves() -> None:
        if counter.save_left > 0:
            QTimer.singleShot(100, wait_for_saves)
            return
        meta = {"frames": counter.saved, "frame_info": counter.frame_info, "camera_format": result["camera_format"]}
        (ARGS.frames_dir / "frames.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
        result["saved_frames"] = {"dir": str(ARGS.frames_dir), "count": len(counter.saved)}
        APP.quit()

    QTimer.singleShot(int(ARGS.warmup * 1000), start_window)
    QTimer.singleShot(int((ARGS.warmup + ARGS.seconds) * 1000), end_window)
    APP.exec()
    core.camera.stop()
    core.workerThread.quit()
    core.sampleWorkerThread.quit()
    core.workerThread.wait(2000)
    core.sampleWorkerThread.wait(2000)
    return result


# --------------------------------------------------------------------------- main


def environment() -> dict[str, Any]:
    import matplotlib
    import scipy

    return {
        "date": datetime.datetime.now().isoformat(timespec="seconds"),
        "python": platform.python_version(),
        "pyside6": pyside_version,
        "numpy": np.__version__,
        "scipy": scipy.__version__,
        "matplotlib": matplotlib.__version__,
        "os": platform.platform(),
        "cpu": platform.processor(),
        "logical_cpus": psutil.cpu_count(),
        "qt_platform": ARGS.platform,
    }


def main() -> None:
    result: dict[str, Any] = {"harness": "bench/python/run.py", "mode": ARGS.mode, "environment": environment()}
    t0 = time.perf_counter()
    result.update(run_synthetic() if ARGS.mode == "synthetic" else run_live())
    result["run_seconds"] = time.perf_counter() - t0

    out = ARGS.out
    if out is None:
        suffix = "-quick" if getattr(ARGS, "quick", False) else ""
        out = ROOT / "bench" / "results" / f"python-{ARGS.mode}{suffix}-{datetime.date.today().isoformat()}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(f"wrote {out}")
    print(summary(result))


def summary(r: dict[str, Any]) -> str:
    lines = []
    if r["mode"] == "synthetic":
        t = r["timing"]
        lines.append(f"setVideoFrame: mean {t['set_video_frame_ms']['mean']:.2f} ms, p99 {t['set_video_frame_ms']['p99']:.2f} ms, max {t['max_fps']:.0f} fps (k={t['smoothing']})")
        for k, v in t["steps_ms"].items():
            lines.append(f"  {k:<13} {v['mean']:7.3f} ms  p99 {v['p99']:7.3f}")
        lines.append(f"  sum of steps {t['steps_sum_mean_ms']:7.3f} ms")
        for name in ("centre", "edge", "no_line"):
            c = r["fit_cases"][name]
            lines.append(f"fit case {name:<8} line {c['line_px']:7.1f}: fit {c['fit_ms']['mean']:6.2f} ms (p99 {c['fit_ms']['p99']:6.2f}), total {c['total_ms']['mean']:6.2f} ms, found {c['found_px']['mean']:7.1f} px")
        lines.append("sub-pixel (true, float err, emitted err, emitted type):")
        for row in r["subpixel"]["rows"]:
            lines.append(f"  {row['true_px']:8.2f}  {row['float_err_px'].get('mean', float('nan')):+7.3f}  {row['emitted_err_px'].get('mean', float('nan')):+7.3f}  {row['emitted_type']}")
        for s in r["linearity"]["by_sigma"]:
            lines.append(f"linearity sigma {s['sigma_px']:5.0f}: max |float err| {s['max_abs_float_err_px']:.1f} px, failures {s['failures']}")
            lines.append("  " + "  ".join(f"{row['true_px']:.0f}:{row['float_err_px'].get('mean', float('nan')):+.1f}" + (f"F{row['failures']}" if row["failures"] else "") for row in s["rows"]))
        rp = r["repeatability"]
        lines.append(f"repeatability: tool sd {rp['tool_err_px']['sd']:.3f} px ({rp['tool_sd_um']:.2f} um), mean err {rp['tool_err_px']['mean']:+.3f} px; float path sd {rp['float_path_err_px']['sd']:.3f} px, mean err {rp['float_path_err_px']['mean']:+.3f} px")
        for row in r["scale"]["same_smoothing"]:
            lines.append(f"scale k={row['smoothing_at_zero']:3d}: reported {row['reported_um']:8.1f} um, true {row['true_um']:8.1f} um, error {row['error_percent']:+.2f} %")
        m = r["scale"]["smoothing_changed_after_zero"]
        lines.append(f"smoothing 50 -> 0 after Zero, no motion: reported {m['reported_um']:+.1f} um")
        mm = r["memory"]
        lines.append(f"memory: {mm['rss_start_mb']:.0f} -> {mm['rss_end_mb']:.0f} MB over {mm['seconds']:.0f} s ({mm['frames']} frames), slope {mm['slope_second_half_mb_per_min']} MB/min")
        g = r["graph"]
        lines.append("graph update ms at n: " + ", ".join(f"{n}: {v['mean']:.1f}" for n, v in g["update_ms_at_n"].items()))
        lines.append(f"graph rss: before {g['rss_before_mb']:.0f} MB, after cycles {[round(x) for x in g['rss_after_each_cycle_mb']]}")
    else:
        lines.append(json.dumps({k: v for k, v in r.items() if k not in ("environment", "centres")}, indent=2))
    return "\n".join(lines)


if __name__ == "__main__":
    main()
