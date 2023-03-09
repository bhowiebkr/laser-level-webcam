from PySide6.QtWidgets import QSizePolicy, QTableWidgetItem, QVBoxLayout, QWidget
from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QPainter, QPixmap, QColor
from PySide6.QtGui import QPainter, QPixmap, QPen, QFont

from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
import matplotlib.pyplot as plt
from scipy.interpolate import CubicSpline

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
    def __init__(self, parent=None, units=None):
        super().__init__(parent)

        self.x = np.empty(0)
        self.y = np.empty(0)
        self.units = units

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

        self.plot_data()

    def set_data(self, data_in):
        if not data_in:
            return

        data, self.line, self.units = data_in
        self.y = data
        self.x = np.arange(1, len(data) + 1)

        self.plot_data()

    def plot_data(self):
        # Clear the axis and plot the data
        self.ax.clear()
        if len(self.y) == 0:
            # If the data has size 0, plot an empty plot
            self.ax.plot([], [])
        else:
            self.ax.plot(self.x, self.y, marker="o", markersize=5, label="Samples")

            # Fit a smooth curve to the data points
            if self.x.shape[0] > 3:
                # f = interp1d(self.x, self.y, kind="quadratic")
                f = CubicSpline(self.x, self.y, bc_type="clamped")
                smooth_x = np.linspace(self.x[0], self.x[-1], 500)
                smooth_y = f(smooth_x)

                # Plot the smooth curve
                self.ax.plot(smooth_x, smooth_y, linewidth=2, label="Smooth")

            # Plot line
            line = np.polyval(self.line, self.x)

            self.ax.set_ylabel(self.units)

            self.ax.plot(self.x, line, label="Slope")
            self.ax.legend()
        self.ax.autoscale_view("tight")

        # Draw the canvas
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
            # return "{:.2f}".format(self.value * units_of_measurements[self.units])
            return get_units(self.units, self.value)
        return super().data(role)
