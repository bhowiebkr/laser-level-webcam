import sys

from PyQt5.QtWidgets import (
    QSlider,
    QMainWindow,
    QWidget,
    QHBoxLayout,
    QFormLayout,
    QApplication,
)
from PyQt5.QtCore import Qt

import qdarktheme
from GUI.analyser import Analyser
from GUI.sensor_feed import SensorFeed


# Define the main window
class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()

        self.resize(800, 600)
        self.setWindowTitle("Laser Level Webcam Tool")

        # Set the main window layout
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        self.layout = QHBoxLayout(central_widget)

        # Create the left and right widgets
        self.sensor_feed = SensorFeed(self)
        self.analyser = Analyser(self)

        self.sensor_feed.webcam_thread.setAnalyser(self.analyser)
        self.sensor_feed.webcam_thread.setSensorFeed(self.sensor_feed)

        self.sensor_feed.webcam_thread.image_ready.connect(
            self.sensor_feed.widget.setImage
        )

        self.sensor_feed.webcam_thread.intensity_values_ready.connect(
            self.analyser.widget.setLuminosityScope
        )

        self.sensor_feed.widget.height_changed.connect(
            self.analyser.widget.setFixedHeight
        )

        self.sensor_feed.webcam_thread.start()

        # Add to layouts
        self.layout.addWidget(self.sensor_feed)
        self.layout.addWidget(self.analyser)

    def closeEvent(self, event):
        self.sensor_feed.webcam_thread.stop()
        self.deleteLater()


if __name__ == "__main__":
    app = QApplication([])
    qdarktheme.setup_theme()
    window = MainWindow()
    window.show()
    sys.exit(app.exec_())
