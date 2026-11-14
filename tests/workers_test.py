from __future__ import annotations

import warnings
from typing import Any

import numpy as np
import numpy.typing as npt
from PySide6.QtCore import QSize
from PySide6.QtMultimedia import QVideoFrame
from PySide6.QtMultimedia import QVideoFrameFormat

from src.Workers import FrameWorker


class Parent:
    """The parts of Core that FrameWorker reads."""

    zero = 0.0
    sensor_width = 5.0
    units = "µm"


def beam(centre: float, width: int = 640, height: int = 48) -> npt.NDArray[np.uint8]:
    """A frame with a vertical beam at `centre` pixels."""
    x = np.arange(width)
    row = 16 + 160 * np.exp(-0.5 * ((x - centre) / 40) ** 2)
    return np.tile(np.round(row).astype(np.uint8), (height, 1))


def video_frame(luma: npt.NDArray[np.uint8]) -> QVideoFrame:
    """An NV12 frame with grey colour, as a camera gives it."""
    h, w = luma.shape
    f = QVideoFrame(QVideoFrameFormat(QSize(w, h), QVideoFrameFormat.PixelFormat.Format_NV12))
    assert f.map(QVideoFrame.MapMode.WriteOnly)
    y = np.frombuffer(f.bits(0), dtype=np.uint8, count=f.mappedBytes(0)).reshape(h, f.bytesPerLine(0))
    y[:, :w] = luma
    uv = np.frombuffer(f.bits(1), dtype=np.uint8, count=f.mappedBytes(1))
    uv[:] = 128
    del y, uv
    f.unmap()
    return f


def test_centre_keeps_its_fraction(qapp: Any) -> None:
    worker = FrameWorker(parent_obj=Parent())
    centres: list[float] = []
    worker.OnCentreChanged.connect(centres.append)

    worker.setVideoFrame(video_frame(beam(320.6)))

    assert abs(centres[-1] - 320.6) < 0.2


def test_frame_without_a_beam(qapp: Any) -> None:
    worker = FrameWorker(parent_obj=Parent())

    with warnings.catch_warnings():
        warnings.simplefilter("error")
        worker.setVideoFrame(video_frame(np.full((48, 640), 40, dtype=np.uint8)))

    assert worker.centre == 0
