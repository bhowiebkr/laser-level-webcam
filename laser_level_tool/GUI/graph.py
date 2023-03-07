from PySide6.QtWidgets import QSizePolicy, QGroupBox, QComboBox, QVBoxLayout, QTableWidgetItem, QHBoxLayout, QGridLayout, QFormLayout, QSpinBox, QLabel, QPushButton, QTableWidget, QHeaderView, QLineEdit
from PySide6.QtGui import QDoubleValidator
from PySide6.QtCore import QThread, QObject, Signal, Qt

from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
from matplotlib.figure import Figure
import matplotlib.pyplot as plt
import numpy as np
import random

plt.rcParams.update({"lines.color": "white", "patch.edgecolor": "white", "text.color": "black", "axes.facecolor": "white", "axes.edgecolor": "lightgray", "axes.labelcolor": "white", "xtick.color": "white", "ytick.color": "white", "grid.color": "lightgray", "figure.facecolor": "black", "figure.edgecolor": "black", "savefig.facecolor": "black", "savefig.edgecolor": "black"})


class Graph(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

        self.x = None
        self.y = None

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)
        main_layout.setContentsMargins(0, 0, 0, 0)
        plt.style.use("https://github.com/dhaitz/matplotlib-stylesheets/raw/master/pitayasmoothie-dark.mplstyle")

        # Line chart
        fig, self.ax = plt.subplots()
        self.canvas = FigureCanvas(fig)

        self.canvas.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        main_layout.addWidget(self.canvas)

    def set_data(self, data):
        self.y = data
        self.x = np.arange(1, len(data) + 1)

        self.plot_data()

    def plot_data(self):
        # Clear the axis and plot the data
        self.ax.clear()
        self.ax.plot(self.x, self.y, marker="o", markersize=5)

        # Draw the canvas
        self.canvas.draw()
