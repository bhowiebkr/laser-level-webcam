from __future__ import annotations

import pytest

from src.Core import Core
from src.Core import samples_recalc
from src.DataClasses import Sample


def samples(heights: list[float]) -> list[Sample]:
    s = [Sample(x=i, y=y) for i, y in enumerate(heights)]
    samples_recalc(s)
    return s


class CoreStub:
    """The part of Core that delete_sample uses."""

    def __init__(self, heights: list[float]) -> None:
        self.samples = samples(heights)


def test_delete_sample_recalculates_the_line() -> None:
    core = CoreStub([0.0, 1.0, 5.0, 3.0, 4.0])

    Core.delete_sample(core, 2)  # type: ignore[arg-type]

    expected = samples([0.0, 1.0, 3.0, 4.0])
    assert [s.x for s in core.samples] == [0, 1, 2, 3]
    for got, want in zip(core.samples, expected):
        assert got.linYError == pytest.approx(want.linYError)
        assert got.shim == pytest.approx(want.shim)
        assert got.scrape == pytest.approx(want.scrape)
