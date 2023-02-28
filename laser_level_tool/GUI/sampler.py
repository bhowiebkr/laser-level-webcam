from PyQt5.QtWidgets import (
    QSizePolicy,
    QGroupBox,
    QVBoxLayout,
    QHBoxLayout,
    QGridLayout,
    QFormLayout,
    QSpinBox,
    QLabel,
    QPushButton,
    QTableWidget,
    QHeaderView,
    QLineEdit,
)

from PyQt5.QtGui import QIntValidator, QDoubleValidator


from GUI.widgets import ResolutionInputWidget


class Sampler(QGroupBox):
    def __init__(self, parent=None):
        super().__init__(parent)

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

        self.sensor_res = ResolutionInputWidget()

        self.pixel_size = QLineEdit("3")
        self.pixel_size.setValidator(QDoubleValidator())  # Only allow integer values

        zero_btn = QPushButton("Zero")
        take_sample_btn = QPushButton("Take Sample")
        take_sample_btn.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)

        self.sub_sample_label = QLabel("0/0")

        self.sample_table = QTableWidget()
        # Set the table headers
        header_names = ["Sample #", "Measured", "Residual", "Scrape", "Shim"]
        self.sample_table.setColumnCount(len(header_names))
        self.sample_table.setHorizontalHeaderLabels(header_names)
        header = self.sample_table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.Stretch)

        sampling_cmd_layout.addWidget(zero_btn)
        sampling_cmd_layout.addWidget(take_sample_btn)
        sampling_cmd_layout.addWidget(self.sub_sample_label)

        params_left.addRow("Number of Samples", self.num_samples)
        params_left.addRow("Sensor Res (px)", self.sensor_res)
        params_right.addRow("Sample removal (%)", self.sample_filter)
        params_right.addRow("Sensor Width (μm)", self.pixel_size)

        top_layout.addLayout(params_left, 0, 0)
        top_layout.addLayout(params_right, 0, 1)
        top_layout.addLayout(sampling_cmd_layout, 1, 0, 1, 2)

        main_layout.addLayout(top_layout)
        main_layout.addWidget(self.sample_table)

    def set_sensor_res(self, res):
        self.sensor_res._widthInput.setText(str(res[0]))
        self.sensor_res._heightInput.setText(str(res[1]))
