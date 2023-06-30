from __future__ import annotations

from src.utils import get_units


def test_get_units() -> None:
    unit_str = get_units(unit="μm", value=0.01)
    assert unit_str == "10.00μm"
