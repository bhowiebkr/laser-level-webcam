import sys

from PySide6.QtWidgets import (
    QSlider,
    QMainWindow,
    QWidget,
    QHBoxLayout,
    QSplitter,
    QApplication,
)
from PySide6.QtCore import Qt

import qdarktheme
from GUI.analyser import Analyser
from GUI.sensor_feed import SensorFeed
from GUI.sampler import Sampler

from utils.misc import get_webcam_max_res


# Define the main window
class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()

        self.max_res = get_webcam_max_res()

        self.resize(800, 600)
        self.setWindowTitle("Laser Level Webcam Tool")

        # Set the main window layout
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        self.layout = QHBoxLayout(central_widget)
        self.layout.setContentsMargins(0, 0, 0, 0)

        splitter = QSplitter()
        left_splitter = QSplitter()
        left_widget = QWidget()
        left_layout = QHBoxLayout()
        left_layout.setContentsMargins(0, 0, 0, 0)

        left_widget.setLayout(left_layout)

        self.sensor_feed = SensorFeed()
        self.analyser = Analyser()
        self.sampler = Sampler()
        self.sampler.set_sensor_res(self.max_res)

        self.sensor_feed.widget.frameWorker.intensityValuesChanged.connect(
            self.analyser.widget.setLuminosityScope
        )

        self.sensor_feed.widget.height_changed.connect(
            self.analyser.widget.setFixedHeight
        )

        # Add to layouts

        left_splitter.addWidget(self.sensor_feed)
        left_splitter.addWidget(self.analyser)

        splitter.addWidget(left_splitter)
        splitter.addWidget(self.sampler)
        self.layout.addWidget(splitter)

    def closeEvent(self, event):
        self.sensor_feed.widget.workerThread.quit()
        self.sensor_feed.widget.workerThread.wait()
        self.deleteLater()


if __name__ == "__main__":
    app = QApplication([])
    qdarktheme.setup_theme()
    window = MainWindow()
    window.show()
    sys.exit(app.exec_())
