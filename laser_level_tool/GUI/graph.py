from PySide6.QtWidgets import QSizePolicy, QGroupBox, QVBoxLayout
from PySide6.QtCore import Qt

from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
from matplotlib.figure import Figure
import matplotlib.pyplot as plt
import numpy as np
from scipy.interpolate import interp1d, CubicSpline

plt.style.use("https://github.com/dhaitz/matplotlib-stylesheets/raw/master/pitayasmoothie-dark.mplstyle")


class Graph(QGroupBox):
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
            self.ax.plot(self.x, self.y, marker="o", markersize=5)

            # Fit a smooth curve to the data points
            if self.x.shape[0] > 3:
                # f = interp1d(self.x, self.y, kind="quadratic")
                f = CubicSpline(self.x, self.y, bc_type="clamped")
                smooth_x = np.linspace(self.x[0], self.x[-1], 500)
                smooth_y = f(smooth_x)

                # Plot the smooth curve
                self.ax.plot(smooth_x, smooth_y, linewidth=2)

            # Plot line
            line = np.polyval(self.line, self.x)

            self.ax.set_ylabel(self.units)

            self.ax.plot(self.x, line, label="Line")

        # Draw the canvas
        self.canvas.draw()
