from __future__ import annotations

from typing import Any

from src.Widgets import PixmapWidget


def test_PixmapWidget(qtbot: Any) -> None:
    pixmap = PixmapWidget()

    qtbot.addWidget(pixmap)
    pixmap.show()

    assert pixmap.isVisible()
