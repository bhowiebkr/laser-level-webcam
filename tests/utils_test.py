from __future__ import annotations

from src.utils import get_units
from src.utils import scale_sample_real_world


def test_get_units() -> None:
    unit_str = get_units(unit="μm", value=0.01)
    assert unit_str == "10.00μm"


def test_scale_sample_real_world() -> None:
    bla = scale_sample_real_world(sensor_width=100, data_width=0, sample=10, zero=10)
    assert bla == 0
