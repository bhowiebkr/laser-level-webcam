from __future__ import annotations

import numpy as np

from src.curves import fit_gaussian


def test_fit_gaussian() -> None:
    curve = np.array([1, 2, 3, 2, 1])
    r = round(fit_gaussian(curve=curve), 5)
    assert r == 2
