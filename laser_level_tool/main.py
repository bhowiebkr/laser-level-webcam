import threading
import sys
import imageio
import numpy as np
from PyQt5 import QtWidgets, QtGui
from PyQt5.QtWidgets import QSlider, QSizePolicy
from PyQt5.QtCore import Qt
from PyQt5.QtGui import QPen
from GUI.palette import palette

from GUI.parameters import ParametersWidget
from GUI.analyser import Analyser
from GUI.sensor_feed import SensorFeed, WebcamThread


# Define the main window
class MainWindow(QtWidgets.QMainWindow):
    def __init__(self):
        super().__init__()

        self.resize(800, 600)
        self.setWindowTitle("Laser Level Webcam Tool")

        # Set the main window layout
        central_widget = QtWidgets.QWidget()
        self.setCentralWidget(central_widget)
        self.layout = QtWidgets.QHBoxLayout(central_widget)

        # Layouts
        buttonLayout = QtWidgets.QFormLayout()

        # Create the left and right widgets
        self.sensor_feed = SensorFeed(self)
        self.analyser = Analyser(self)

        # Widgets
        self.smoothingSlider = QSlider(Qt.Horizontal, self)
        self.smoothingSlider.setMinimum(0)
        self.smoothingSlider.setMaximum(100)
        self.smoothingSlider.setValue(0)
        self.smoothingSlider.setTickInterval(1)

        # Add to layouts
        self.layout.addWidget(self.sensor_feed)
        self.layout.addWidget(self.analyser)

        # Start the webcam thread
        self.webcam_thread = WebcamThread(self)
        self.webcam_thread.start()

        self.layout.addLayout(buttonLayout)
        buttonLayout.addRow("Smoothing", self.smoothingSlider)

    def closeEvent(self, event):
        self.webcam_thread.stop()
        self.deleteLater()


if __name__ == "__main__":
    app = QtWidgets.QApplication([])
    app.setPalette(palette)
    window = MainWindow()
    window.show()
    sys.exit(app.exec_())
