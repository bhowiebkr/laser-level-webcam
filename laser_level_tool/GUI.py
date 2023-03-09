import sys

from PySide6.QtWidgets import QMainWindow, QDoubleSpinBox, QRadioButton, QLabel, QGridLayout, QSpinBox, QLineEdit, QFormLayout, QSlider, QVBoxLayout, QTableWidget, QPushButton, QComboBox, QGroupBox, QWidget, QHBoxLayout, QSplitter, QApplication
from PySide6.QtCore import Qt, QThread, Signal, QObject, Slot

import qdarktheme

from Widgets import PixmapWidget, Graph, AnalyserWidget
from utils.misc import units_of_measurements
from Core import Core


# Define the main window
class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()

        self.setWindowTitle("Laser Level Webcam Tool")
        self.resize(1100, 650)

        self.setting_zero = False  # state if the GUI is setting zero

        # Set the main window layout
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        main_layout = QHBoxLayout(central_widget)
        main_layout.setContentsMargins(0, 0, 0, 0)

        # Widgets
        left_splitter = QSplitter()
        middle_splitter = QSplitter()
        right_splitter = QSplitter(Qt.Orientation.Vertical)
        sensor_feed_widget = QGroupBox("Sensor Feed")
        analyser_widget = QGroupBox("Analyser")
        sampler_widget = QGroupBox("Sampler")
        plot_widget = QGroupBox("Plot")

        # -- Sensor Feed --
        self.sensor_feed_widget = PixmapWidget()
        self.camera_combo = QComboBox()
        camera_device_settings_btn = QPushButton("Device Settings")
        sensor_layout = QVBoxLayout()
        sensor_layout.setContentsMargins(1, 6, 1, 1)
        sensor_form = QFormLayout()
        sensor_form.addRow("Camera", self.camera_combo)
        sensor_layout.addWidget(self.sensor_feed_widget)
        sensor_layout.addLayout(sensor_form)
        sensor_layout.addWidget(camera_device_settings_btn)
        sensor_feed_widget.setLayout(sensor_layout)

        # -- Analyser --
        self.analyser_widget = AnalyserWidget()
        self.smoothing = QSlider(Qt.Horizontal)
        self.smoothing.setRange(0, 200)
        self.smoothing.setTickInterval(1)
        analyser_form = QFormLayout()
        analyser_layout = QVBoxLayout()
        analyser_layout.setContentsMargins(1, 6, 1, 1)
        analyser_form.addRow("Smoothing", self.smoothing)
        analyser_layout.addWidget(self.analyser_widget)
        analyser_layout.addLayout(analyser_form)
        analyser_widget.setLayout(analyser_layout)

        # -- Sampler --
        self.subsamples_spin = QSpinBox()
        self.subsamples_spin.setRange(0, 9999)
        self.outlier_spin = QSpinBox()
        self.outlier_spin.setRange(0, 99)
        self.units_combo = QComboBox()
        self.units_combo.addItems(units_of_measurements.keys())
        self.units_combo.setCurrentIndex(1)
        self.sensor_width_spin = QDoubleSpinBox()
        self.zero_btn = QPushButton("Zero")
        self.sample_btn = QPushButton("Take Sample")
        self.sample_table = QTableWidget()
        sample_layout = QGridLayout()
        sample_layout.setContentsMargins(1, 1, 1, 1)
        sample_layout.addWidget(QLabel("Sub Samples #"), 0, 0, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.subsamples_spin, 0, 1, 1, 1)
        sample_layout.addWidget(QLabel("Outlier Removal %"), 0, 2, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.outlier_spin, 0, 3, 1, 1)
        sample_layout.addWidget(QLabel("Units"), 1, 0, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.units_combo, 1, 1, 1, 1)
        sample_layout.addWidget(QLabel("Sensor Width (mm)"), 1, 2, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.sensor_width_spin, 1, 3, 1, 1)
        sample_layout.addWidget(self.zero_btn, 2, 0, 1, 1)
        sample_layout.addWidget(self.sample_btn, 2, 1, 1, 3)
        sample_layout.addWidget(self.sample_table, 3, 0, 1, 4)
        sampler_widget.setLayout(sample_layout)

        # -- Plot --
        self.raw_radio = QRadioButton("Raw")
        self.flat_radio = QRadioButton("Flattened")
        self.graph = Graph()
        plot_layout = QVBoxLayout()
        plot_layout.setContentsMargins(0, 3, 0, 0)
        radio_layout = QHBoxLayout()
        radio_layout.addWidget(self.raw_radio, alignment=Qt.AlignRight)
        radio_layout.addWidget(self.flat_radio)
        plot_layout.addLayout(radio_layout)
        plot_layout.addWidget(self.graph)
        plot_widget.setLayout(plot_layout)

        # Attach Widgets
        left_splitter.addWidget(sensor_feed_widget)
        left_splitter.addWidget(analyser_widget)
        right_splitter.addWidget(sampler_widget)
        right_splitter.addWidget(plot_widget)
        middle_splitter.addWidget(left_splitter)
        middle_splitter.addWidget(right_splitter)
        main_layout.addWidget(middle_splitter)

        # Logic
        middle_splitter.setSizes([300, 100])

        self.core = Core()

        for cam in self.core.get_cameras():
            self.camera_combo.addItem(cam)

        self.core.set_camera(self.camera_combo.currentIndex())

        # Signals
        self.core.OnSensorFeedUpdate.connect(self.sensor_feed_widget.setPixmap)
        self.core.OnAnalyserUpdate.connect(self.analyser_widget.set_data)
        self.sensor_feed_widget.OnHeightChanged.connect(self.analyser_widget.setMaximumHeight)
        self.sensor_feed_widget.OnHeightChanged.connect(self.core.set_analyser_widget_height)
        self.smoothing.valueChanged.connect(self.core.frameWorker.set_smoothness)
        self.subsamples_spin.valueChanged.connect(self.core.set_subsamples)
        self.outlier_spin.valueChanged.connect(self.core.set_outliers)
        self.units_combo.currentTextChanged.connect(self.core.set_units)
        self.sensor_width_spin.valueChanged.connect(self.core.set_sensor_width)
        self.zero_btn.clicked.connect(self.zero_btn_cmd)
        self.sample_btn.clicked.connect(self.sample_btn_cmd)
        self.core.OnSubsampleProgressUpdate.connect(self.subsample_progress_update)
        self.core.OnSampleComplete.connect(self.finished_subsample)
        self.smoothing.setValue(50)
        self.subsamples_spin.setValue(30)
        self.outlier_spin.setValue(30)
        self.units_combo.setCurrentIndex(0)
        self.sensor_width_spin.setValue(5.9)

    def finished_subsample(self):
        """
        Sample complete. Reset the GUI back to the default state
        """
        if self.setting_zero == True:
            self.zero_btn.setEnabled(True)
            self.zero_btn.setText("Zero")
            self.setting_zero = False
        else:
            self.sample_btn.setEnabled(True)
            self.sample_btn.setText("Take Sample")

    def subsample_progress_update(self, sample_total):
        """
        Progress update on either zero or sample button
        """
        sample = sample_total[0]
        total = sample_total[1]

        if self.setting_zero == True:
            self.zero_btn.setDisabled(True)
            self.zero_btn.setText(f"{sample}/{total}")
        else:
            self.sample_btn.setDisabled(True)
            self.sample_btn.setText(f"{sample}/{total}")

    def zero_btn_cmd(self):
        """
        Calls the sample button command but sets a flag so we know the GUI is in a state of setting the zero value
        """
        self.setting_zero = True
        self.sample_btn_cmd()

    def sample_btn_cmd(self):
        """
        Calls on Core to take a sample
        """
        self.core.start_sample(self.setting_zero)

    def closeEvent(self, event):
        self.core.workerThread.quit()
        self.core.workerThread.wait()
        self.core.sampleWorkerThread.quit()
        self.core.sampleWorkerThread.wait()
        self.deleteLater()
        super().closeEvent(event)


if __name__ == "__main__":
    app = QApplication(sys.argv)
    qdarktheme.setup_theme()
    window = MainWindow()
    window.show()
    sys.exit(app.exec())
