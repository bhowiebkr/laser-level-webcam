from PySide6.QtWidgets import QSizePolicy, QGroupBox, QComboBox, QVBoxLayout, QTableWidgetItem, QHBoxLayout, QGridLayout, QFormLayout, QSpinBox, QLabel, QPushButton, QTableWidget, QHeaderView, QLineEdit
from PySide6.QtGui import QDoubleValidator
from PySide6.QtCore import QThread, QObject, Signal, Qt

import numpy as np

from utils.misc import units_of_measurements, scale_center_point


class SampleWorker(QObject):
    OnSampleReady = Signal(float)
    OnSubsampleRecieved = Signal(int)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.ready = True
        self.sample_array = np.empty((0,))
        self.total_samples = 0
        self.running_total = 0
        self.outlier_percent = 0
        self.started = False

    def sample_in(self, sample):
        if not self.started:
            return

        # Append new value to array
        self.sample_array = np.append(self.sample_array, sample)

        # self.acclimated_samples += sample
        self.running_total += 1

        self.OnSubsampleRecieved.emit(self.running_total)

        if self.running_total == self.total_samples:
            # Calculate the number of outliers to remove
            n_outliers = int(len(self.sample_array) * self.outlier_percent / 2.0)

            # Sort and remove the outliers
            self.sample_array = self.sample_array[self.sample_array.argsort()][n_outliers:-n_outliers]

            # Calculate new mean as float
            mean = np.mean(self.sample_array).astype(float)

            self.OnSampleReady.emit(mean)

            # reset
            self.sample_array = np.empty((0,))
            self.running_total = 0
            self.total_samples = 0
            self.started = False

    def start(self, total_samples, outlier_percent):
        self.total_samples = total_samples
        self.outlier_percent = outlier_percent / 100
        self.started = True


class Sampler(QGroupBox):
    OnSetZero = Signal(float)
    OnSensorWidthChange = Signal(float)
    OnUnitsChanged = Signal(str)

    def __init__(self, parent=None):
        super().__init__(parent)

        self.zero_point = None
        self.setting_zero = False
        self.sensor_pixel_width = None

        self.sample_worker = SampleWorker()
        self.sample_worker.OnSampleReady.connect(self.received_sample)
        self.sample_worker.OnSubsampleRecieved.connect(self.subsample_progress_update)
        self.workerThread = QThread()
        self.sample_worker.moveToThread(self.workerThread)
        self.workerThread.start()

        self.setTitle("Sampler")

        # Layouts
        main_layout = QVBoxLayout()
        self.setLayout(main_layout)

        top_layout = QGridLayout()

        params_left = QFormLayout()
        params_right = QFormLayout()
        sampling_cmd_layout = QHBoxLayout()

        # Widgets
        self.num_samples = QSpinBox()
        self.num_samples.setMinimum(0)
        self.num_samples.setMaximum(9999)
        self.num_samples.setValue(10)

        self.sample_filter = QSpinBox()
        self.sample_filter.setMinimum(0)
        self.sample_filter.setMaximum(99)
        self.sample_filter.setValue(30)

        self.units = QComboBox()

        for unit in units_of_measurements.keys():
            self.units.addItem(unit)

        self.sensor_width = QLineEdit("3")
        self.sensor_width.setValidator(QDoubleValidator())  # Only allow integer values

        self.zero_btn = QPushButton("Zero")
        self.take_sample_btn = QPushButton("Take Sample")
        self.take_sample_btn.setDisabled(True)
        self.take_sample_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)

        self.sample_table = QTableWidget()
        # Set the table headers
        units = self.units.currentText()
        header_names = [f"Measured ({units})", f"Residual ({units})", f"Scrape ({units})", f"Shim ({units})"]
        self.sample_table.setColumnCount(len(header_names))
        self.sample_table.setHorizontalHeaderLabels(header_names)
        header = self.sample_table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.Stretch)

        sampling_cmd_layout.addWidget(self.zero_btn)
        sampling_cmd_layout.addWidget(self.take_sample_btn)

        params_left.addRow("Sub Sample #", self.num_samples)
        params_left.addRow("Units", self.units)
        params_right.addRow("Outlier Removal (%)", self.sample_filter)
        params_right.addRow("Sensor Width (mm)", self.sensor_width)

        top_layout.addLayout(params_left, 0, 0)
        top_layout.addLayout(params_right, 0, 1)
        top_layout.addLayout(sampling_cmd_layout, 1, 0, 1, 2)

        main_layout.addLayout(top_layout)
        main_layout.addWidget(self.sample_table)

        # Logic
        self.take_sample_btn.clicked.connect(self.take_sample_btn_cmd)
        self.zero_btn.clicked.connect(self.zero_btn_cmd)
        self.sensor_width.textChanged.connect(self.sensor_width_changed)
        self.sensor_width.textChanged.connect(self.reset_zero_point)
        self.sample_filter.valueChanged.connect(self.reset_zero_point)
        self.num_samples.valueChanged.connect(self.reset_zero_point)
        self.units.currentTextChanged.connect(self.units_changed)

    def units_changed(self):
        units = self.units.currentText()
        self.OnUnitsChanged.emit(units)

    def zero_btn_cmd(self):
        self.setting_zero = True
        self.take_sample_btn_cmd()

    def sensor_width_changed(self):
        self.OnSensorWidthChange.emit(float(self.sensor_width.text()))

    def receive_zero_point(self, zero_point):
        self.zero_point = zero_point
        self.sample_table.setRowCount(0)
        self.take_sample_btn.setEnabled(True)

    def reset_zero_point(self):
        self.zero_point = None
        self.take_sample_btn.setDisabled(True)

    def take_sample_btn_cmd(self):
        subsamples = int(self.num_samples.text())
        outlier_percent = int(self.sample_filter.value())
        self.sample_worker.start(subsamples, outlier_percent)

    def subsample_progress_update(self, num):
        if self.setting_zero == True:
            self.zero_btn.setDisabled(True)
            self.zero_btn.setText(f"{num}/{self.num_samples.text()}")
        else:
            self.take_sample_btn.setDisabled(True)
            self.take_sample_btn.setText(f"{num}/{self.num_samples.text()}")

    def set_sensor_pixel_width(self, sensor_pixel_width):
        self.sensor_pixel_width = sensor_pixel_width

    def received_sample(self, sample):
        if self.setting_zero == True:
            self.zero_btn.setEnabled(True)
            self.zero_btn.setText("Zero")
            self.OnSetZero.emit(sample)
            self.setting_zero = False
        else:
            self.take_sample_btn.setEnabled(True)
            self.take_sample_btn.setText("Take Sample")

            self.sample_table.insertRow(self.sample_table.rowCount())

            row_count = self.sample_table.rowCount()

            # value = float(self.sensor_width.text()) / self.sensor_pixel_width * (sample - self.zero_point) * units
            value = scale_center_point(self.sensor_width.text(), self.sensor_pixel_width, sample, self.zero_point, self.units.currentText())
            new_data = [value, 0, 0, 0]

            for index, data in enumerate(new_data):
                cell = QTableWidgetItem()
                cell.setTextAlignment(Qt.AlignCenter)  # center-align the text
                cell.setData(Qt.DisplayRole, data)
                self.sample_table.setItem(row_count - 1, index, cell)
