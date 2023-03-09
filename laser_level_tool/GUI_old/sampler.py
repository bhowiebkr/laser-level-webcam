from PySide6.QtWidgets import QSizePolicy, QSplitter, QWidget, QRadioButton, QGroupBox, QComboBox, QVBoxLayout, QTableWidgetItem, QHBoxLayout, QGridLayout, QFormLayout, QSpinBox, QLabel, QPushButton, QTableWidget, QHeaderView, QLineEdit
from PySide6.QtGui import QDoubleValidator
from PySide6.QtCore import QThread, QObject, Signal, Qt

import numpy as np
from GUI_old.graph import Graph
from utils.misc import get_units, units_of_measurements, scale_center_point, scale_center_point_no_units
from scipy import stats


class TableUnit(QTableWidgetItem):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.units = None
        self.value = None

    def set_units(self, units):
        self.units = units

    def data(self, role):
        if role == Qt.DisplayRole:
            # return "{:.2f}".format(self.value * units_of_measurements[self.units])
            return get_units(self.units, self.value)
        return super().data(role)


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
        self.header_names = None
        self.sample_data = np.empty(0)
        self.line_data = np.empty(0)

        self.sample_worker = SampleWorker()
        self.sample_worker.OnSampleReady.connect(self.received_sample)
        self.sample_worker.OnSubsampleRecieved.connect(self.subsample_progress_update)
        self.workerThread = QThread()
        self.sample_worker.moveToThread(self.workerThread)
        self.workerThread.start()

        self.setTitle("Sampler")

        # Layouts
        main_layout = QVBoxLayout()
        main_layout.setContentsMargins(0, 0, 0, 0)
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

        self.raw_radio = QRadioButton("Raw")
        self.flat_radio = QRadioButton("Flattened")
        self.raw_radio.setChecked(True)

        for unit in units_of_measurements:
            self.units.addItem(unit)

        self.sensor_width = QLineEdit("5.9")
        self.sensor_width.setValidator(QDoubleValidator())  # Only allow integer values

        self.zero_btn = QPushButton("Zero")
        self.take_sample_btn = QPushButton("Take Sample")
        self.take_sample_btn.setDisabled(True)
        self.take_sample_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)

        self.sample_table = QTableWidget()
        # Set the table headers
        units = self.units.currentText()
        self.header_names = [f"Measured ({units})", f"Residual ({units})", f"Scrape ({units})", f"Shim ({units})"]
        self.sample_table.setColumnCount(len(self.header_names))
        self.sample_table.setHorizontalHeaderLabels(self.header_names)
        header = self.sample_table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.Stretch)
        self.splitter = QSplitter()
        self.splitter.setOrientation(Qt.Orientation.Vertical)

        self.graph = Graph(None, self.units.currentText())
        graph_h_layout = QHBoxLayout()
        graph_h_layout.addStretch()

        graph_h_layout.addWidget(self.raw_radio, alignment=Qt.AlignRight)
        graph_h_layout.addWidget(self.flat_radio)
        graph_h_layout.addStretch()
        graph_v_layout = QVBoxLayout()
        graph_v_layout.setContentsMargins(0, 0, 0, 0)
        graph_v_layout.addSpacing(5)
        graph_v_layout.addLayout(graph_h_layout)
        graph_v_layout.addWidget(self.graph)
        graph_widget = QGroupBox("Plot")
        graph_widget.setLayout(graph_v_layout)

        self.splitter.addWidget(self.sample_table)
        self.splitter.addWidget(graph_widget)

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
        main_layout.addWidget(self.splitter)
        # self.splitter.setSizes([200, 100])

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
        self.header_names = [f"Measured ({units})", f"Residual ({units})", f"Scrape ({units})", f"Shim ({units})"]
        self.sample_table.setHorizontalHeaderLabels(self.header_names)

        for row in range(self.sample_table.rowCount()):
            for col in range(self.sample_table.columnCount()):
                item = self.sample_table.item(row, col)
                if item:
                    item.units = units

        self.OnUnitsChanged.emit(units)

    def zero_btn_cmd(self):
        self.setting_zero = True
        self.sample_data = np.empty(0)
        self.graph.set_data(self.sample_data)

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

            value = scale_center_point_no_units(self.sensor_width.text(), self.sensor_pixel_width, sample, self.zero_point)
            self.sample_data = np.append(self.sample_data, value)

            x = np.arange(1, len(self.sample_data) + 1)

            self.line = np.polyfit(x, self.sample_data, 1)

            self.update_sample_table()

    def update_sample_table(self):
        # Delete the rows
        self.sample_table.setRowCount(0)

        for row, data in enumerate(self.sample_data):
            # Check if there are enough rows in the table widget, and add a new row if necessary
            if row >= self.sample_table.rowCount():
                self.sample_table.insertRow(row)

            cell = TableUnit()
            cell.value = data
            cell.units = self.units.currentText()
            self.sample_table.setItem(row, 0, cell)
        units = self.units.currentText()
        unit_multiplier = units_of_measurements[units]
        self.graph.set_data([self.sample_data * unit_multiplier, self.line * unit_multiplier, units])
