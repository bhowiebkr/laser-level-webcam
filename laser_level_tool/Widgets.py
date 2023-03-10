from PySide6.QtWidgets import QSizePolicy, QTableWidgetItem, QVBoxLayout, QWidget
from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QPainter, QPixmap, QColor
from PySide6.QtGui import QPainter, QPixmap, QPen, QFont

from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
import matplotlib.pyplot as plt
from scipy.interpolate import CubicSpline
from utils import units_of_measurements

import numpy as np

from utils import get_units

style = {
    "axes.grid": "True",
    "axes.edgecolor": "white",
    "axes.linewidth": "0",
    "xtick.major.size": "0",
    "ytick.major.size": "0",
    "xtick.minor.size": "0",
    "ytick.minor.size": "0",
    "text.color": "0.9",
    "axes.labelcolor": "0.9",
    "xtick.color": "0.9",
    "ytick.color": "0.9",
    "grid.color": "2A3459",
    "font.sans-serif": "Overpass, Helvetica, Helvetica Neue, Arial, Liberation Sans, DejaVu Sans, Bitstream Vera Sans, sans-serif",
    "figure.facecolor": "202124",
    "axes.facecolor": "101012",
    "savefig.facecolor": "212946",
    "image.cmap": "RdPu",
}
plt.style.use(style)


class Graph(QWidget):
    def __init__(self, samples):
        super().__init__()

        self.samples = samples
        self.units = None
        self.mode = None

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)
        main_layout.setContentsMargins(0, 0, 0, 0)

        # Line chart
        fig, self.ax = plt.subplots()
        self.canvas = FigureCanvas(fig)

        self.ax.set_ylabel(self.units)
        self.ax.autoscale_view("tight")

        self.canvas.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        main_layout.addWidget(self.canvas)

    def set_units(self, units):
        self.units = units
        self.update(self.set_units)

    def set_mode(self, mode):
        self.mode = mode
        self.update(self.set_mode)

    def update(self, sender):
        # Clear the axis and plot the data
        self.ax.clear()

        if self.units == None or self.mode == None or len(self.samples) == 0:
            self.canvas.draw()
            return

        unit_multiplier = units_of_measurements[self.units]

        x = np.arange(1, len(self.samples) + 1)
        y = []
        if self.mode == "Raw":
            # Raw points
            for s in self.samples:
                y.append(s.y * unit_multiplier)
            self.ax.plot(x, y, marker="o", markersize=5, label="Samples")

            # Fit a smooth curve to the data points
            if len(x) > 2:
                f = CubicSpline(x, y, bc_type="clamped")
                smooth_x = np.linspace(x[0], x[-1], 500)
                smooth_y = f(smooth_x)
                self.ax.plot(smooth_x, smooth_y, linewidth=2, label="Smooth")

            # Plot line
            line = np.polyfit(x, y, 1)
            line = np.polyval(line, x)
            self.ax.set_ylabel(self.units)
            self.ax.plot(x, line, label="Slope")
        else:
            # Raw points
            for s in self.samples:
                y.append(s.linYError * unit_multiplier)
            self.ax.plot(x, y, marker="o", markersize=5, label="Samples")

            # Fit a smooth curve to the data points
            if len(x) > 2:
                f = CubicSpline(x, y, bc_type="clamped")
                smooth_x = np.linspace(x[0], x[-1], 500)
                smooth_y = f(smooth_x)
                self.ax.plot(smooth_x, smooth_y, linewidth=2, label="Smooth")

            # Plot line

            zeros = np.zeros(len(self.samples))
            self.ax.plot(x, zeros, label="Slope")

        self.ax.legend()
        self.canvas.draw()


class PixmapWidget(QWidget):
    OnHeightChanged = Signal(int)

    def __init__(self, parent=None):
        super().__init__(parent)

        self.pixmap = QPixmap(100, 100)
        self.pixmap.fill(QColor(0, 0, 0))

        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
        super().paintEvent(event)

        if not self.pixmap:
            return

        painter = QPainter(self)
        painter.drawPixmap(self.rect(), self.pixmap)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        new_height = event.size().height()
        self.OnHeightChanged.emit(new_height)

    def setPixmap(self, pixmap):
        self.pixmap = pixmap
        self.update()


class AnalyserWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.pixmap = QPixmap(100, 100)
        self.pixmap.fill(QColor(0, 0, 0))
        self.sample = 0  # location of the sample in pixel space on the widget
        self.zero = None  # location of zero. if not set, it's None
        self.text = None  # Text to display if zero is set. shows the distance from zero

        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

    def paintEvent(self, event):
        super().paintEvent(event)
        painter = QPainter(self)

        # Pixmap
        painter.drawPixmap(self.rect(), self.pixmap)

        # Sample
        if self.sample:
            pen = QPen(Qt.green, 0, Qt.SolidLine)
            painter.setPen(pen)
            painter.drawLine(0, self.sample, self.width(), self.sample)

        # zero
        if self.zero:
            painter.setPen(Qt.red)
            painter.drawLine(0, self.zero, self.width(), self.zero)

        if self.text:
            painter.setFont(QFont("Arial", 12))
            painter.setPen(Qt.green)
            textWidth = painter.fontMetrics().horizontalAdvance(self.text)
            textHeight = painter.fontMetrics().height()
            x = (self.width() - textWidth) / 2
            y = self.sample - (textHeight / 2)
            painter.setPen(Qt.green)
            painter.drawText(int(x), int(y), self.text)

    def set_data(self, data):
        self.pixmap, self.sample, self.zero, self.text = data
        self.update()


class TableUnit(QTableWidgetItem):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.units = None
        self.value = None

    def set_units(self, units):
        self.units = units

    def data(self, role):
        if role == Qt.DisplayRole:
            return get_units(self.units, self.value)
        return super().data(role)
