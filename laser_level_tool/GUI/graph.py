from PySide6.QtWidgets import QSizePolicy, QGroupBox, QComboBox, QVBoxLayout, QTableWidgetItem, QHBoxLayout, QGridLayout, QFormLayout, QSpinBox, QLabel, QPushButton, QTableWidget, QHeaderView, QLineEdit
from PySide6.QtGui import QDoubleValidator
from PySide6.QtCore import QThread, QObject, Signal, Qt

import seaborn as sns
from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
from matplotlib.figure import Figure
import matplotlib.pyplot as plt

plt.rcParams.update({"lines.color": "white", "patch.edgecolor": "white", "text.color": "black", "axes.facecolor": "white", "axes.edgecolor": "lightgray", "axes.labelcolor": "white", "xtick.color": "white", "ytick.color": "white", "grid.color": "lightgray", "figure.facecolor": "black", "figure.edgecolor": "black", "savefig.facecolor": "black", "savefig.edgecolor": "black"})


class Graph(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)
        main_layout.setContentsMargins(0, 0, 0, 0)

        # style must be one of white, dark, whitegrid, darkgrid, ticks
        sns.set_style("darkgrid", {"grid.color": ".5", "grid.linestyle": "--"})
        sns.set_palette(["#599ef7", "#5DADE2"])

        # Create the Seaborn graph using sns.displot()
        tips = sns.load_dataset("tips")
        graph = sns.displot(tips, x="total_bill", kde=True)

        # graph.set_facecolor("red")
        graph.fig.set_facecolor("#202124")

        ax = graph.fig.axes[0]

        ax.set_facecolor("black")  # set the background color
        ax.spines["bottom"].set_color("#656565")  # change the color of the x-axis
        ax.spines["left"].set_color("#656565")  # change the color of the y-axis
        ax.tick_params(axis="both", colors="#999999")  # change the color of the tick marks
        ax.yaxis.label.set_color("#999999")  # change the color of the y-axis label
        ax.set_title("Example Plot", color="#999999")  # change the color of the title

        # Create a FigureCanvas to display the Seaborn graph
        self.canvas = FigureCanvas(graph.fig)

        self.canvas.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)

        main_layout.addWidget(self.canvas)
