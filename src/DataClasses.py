from dataclasses import dataclass

from PySide6.QtGui import QPixmap


@dataclass
class FrameData:
    def __init__(self, pixmap: QPixmap, sample: int, zero: int, text: str) -> None:
        self.pixmap = pixmap
        self.sample = sample
        self.zero = zero
        self.text = text


@dataclass
class Sample:
    def __init__(self, x: int, y: float) -> None:
        self.x = x
        self.y = y
        self.linYError = 0.0
        self.shim = 0.0
        self.scrape = 0.0
