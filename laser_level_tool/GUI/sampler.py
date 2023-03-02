from PySide6.QtWidgets import QSizePolicy, QGroupBox, QVBoxLayout, QHBoxLayout, QGridLayout, QFormLayout, QSpinBox, QLabel, QPushButton, QTableWidget, QHeaderView, QLineEdit
from PySide6.QtGui import QDoubleValidator
from PySide6.QtCore import QThread, QObject, Signal

import numpy as np


class SampleWorker(QObject):
    sample_ready = Signal(float)
    subsample_recieved = Signal(int)

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

        self.subsample_recieved.emit(self.running_total)

        # print(f"Sample {self.running_total}/{self.total_samples}")

        if self.running_total == self.total_samples:
            # Calculate current mean and standard deviation
            mean = np.mean(self.sample_array)
            std_dev = np.std(self.sample_array)

            # Calculate threshold for outlier removal
            threshold = mean + std_dev * self.outlier_percent

            # Remove outliers above threshold
            print(self.sample_array, "\n")
            self.sample_array = self.sample_array[self.sample_array <= threshold]
            print(self.sample_array)

            # Calculate new mean as float
            mean = np.mean(self.sample_array).astype(float)

            # average = self.acclimated_samples / self.total_samples
            self.sample_ready.emit(mean)

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
    def __init__(self, parent=None):
        super().__init__(parent)

        self.zero_point = None

        self.sample_worker = SampleWorker()
        self.sample_worker.sample_ready.connect(self.add_sample)
        self.sample_worker.subsample_recieved.connect(self.subsample_progress_update)
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
        self.num_samples.setValue(60)

        self.sample_filter = QSpinBox()
        self.sample_filter.setMinimum(0)
        self.sample_filter.setMaximum(99)
        self.sample_filter.setValue(20)

        # self.sensor_res = ResolutionInputWidget()

        self.pixel_size = QLineEdit("3")
        self.pixel_size.setValidator(QDoubleValidator())  # Only allow integer values

        self.threshold = QLineEdit("6")
        self.pixel_size.setValidator(QDoubleValidator())  # Only allow integer values

        self.zero_btn = QPushButton("Zero")
        self.take_sample_btn = QPushButton("Take Sample")
        self.take_sample_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)

        # self.sub_sample_label = QLabel("0/0")

        self.sample_table = QTableWidget()
        # Set the table headers
        header_names = ["Sample #", "Measured", "Residual", "Scrape", "Shim"]
        self.sample_table.setColumnCount(len(header_names))
        self.sample_table.setHorizontalHeaderLabels(header_names)
        header = self.sample_table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.Stretch)

        sampling_cmd_layout.addWidget(self.zero_btn)
        sampling_cmd_layout.addWidget(self.take_sample_btn)
        # sampling_cmd_layout.addWidget(self.sub_sample_label)

        params_left.addRow("Sub Sample #", self.num_samples)
        # params_left.addRow("Threshold (μm)", self.threshold)
        params_right.addRow("Outlier Removal (%)", self.sample_filter)
        params_right.addRow("Sensor Width (μm)", self.pixel_size)

        top_layout.addLayout(params_left, 0, 0)
        top_layout.addLayout(params_right, 0, 1)
        top_layout.addLayout(sampling_cmd_layout, 1, 0, 1, 2)

        main_layout.addLayout(top_layout)
        main_layout.addWidget(self.sample_table)

        # Logic
        self.take_sample_btn.clicked.connect(self.take_sample)

    def set_zero_point(self, zero_point):
        self.zero_point = zero_point

    def take_sample(self):
        subsamples = int(self.num_samples.text())
        outlier_percent = int(self.threshold.text())
        self.sample_worker.start(subsamples, outlier_percent)

    def subsample_progress_update(self, num):
        self.take_sample_btn.setDisabled(True)
        self.take_sample_btn.setText(f"{num}/{self.num_samples.text()}")

    def add_sample(self, sample):
        self.take_sample_btn.setEnabled(True)
        self.take_sample_btn.setText("Take Sample")
        print(f"Adding smaple: {sample}")
