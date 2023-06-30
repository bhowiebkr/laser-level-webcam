from __future__ import annotations

from typing import Any

from src.main import MainWindow


def test_PixmapWidget(qtbot: Any) -> None:
    main = MainWindow()

    qtbot.addWidget(main)
    main.show()

    assert main.isVisible()
