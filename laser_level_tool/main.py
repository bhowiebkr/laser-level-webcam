import sys

from PySide6.QtWidgets import QMainWindow, QWidget, QHBoxLayout, QSplitter, QApplication

import qdarktheme

from GUI.analyser import Analyser
from GUI.sensor_feed import SensorFeed
from GUI.sampler import Sampler

from utils.misc import get_webcam_max_res


# Define the main window
class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()

        self.setWindowTitle("Laser Level Webcam Tool")

        self.max_res = get_webcam_max_res()

        self.resize(1280 + 70, 720 + 39)

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

        # Sensor Feed
        self.sensor_feed.widget.frameWorker.OnIntensityValuesChanged.connect(self.analyser.widget.setLuminosityScope)
        self.sensor_feed.widget.OnHeightChanged.connect(self.analyser.widget.setFixedHeight)
        self.sensor_feed.widget.set_camera(0)
        self.sensor_feed.widget.OnCameraChanged.connect(self.analyser.widget.reset_zero_point)
        self.sensor_feed.widget.OnCameraChanged.connect(self.sampler.reset_zero_point)

        # Analyser
        self.analyser.widget.OnZeroPointChanged.connect(self.sampler.receive_zero_point)
        self.analyser.widget.OnCenterPointChanged.connect(self.sampler.sample_worker.sample_in)
        self.analyser.widget.OnDataWidthChanged.connect(self.sampler.set_sensor_pixel_width)

        # Sampler
        self.sampler.OnSetZero.connect(self.analyser.widget.set_zero)
        self.sampler.OnSensorWidthChange.connect(self.analyser.widget.set_sensor_width)
        self.sampler.sensor_width_changed()
        self.sampler.OnUnitsChanged.connect(self.analyser.widget.set_units)
        self.sampler.units_changed()

        # Add to layouts
        left_splitter.addWidget(self.sensor_feed)
        left_splitter.addWidget(self.analyser)

        splitter.addWidget(left_splitter)
        splitter.addWidget(self.sampler)
        self.layout.addWidget(splitter)

    def closeEvent(self, event):
        self.sensor_feed.widget.workerThread.quit()
        self.sensor_feed.widget.workerThread.wait()

        self.sampler.workerThread.quit()
        self.sampler.workerThread.wait()

        self.deleteLater()


if __name__ == "__main__":
    app = QApplication(sys.argv)
    qdarktheme.setup_theme()
    window = MainWindow()
    window.show()
    sys.exit(app.exec_())
