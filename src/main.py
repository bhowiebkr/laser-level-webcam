from __future__ import annotations

import csv
import shutil
import subprocess
import sys

import qdarktheme
from PySide6.QtCore import Qt
from PySide6.QtCore import QUrl
from PySide6.QtGui import QAction
from PySide6.QtGui import QCloseEvent
from PySide6.QtGui import QDesktopServices
from PySide6.QtWidgets import QAbstractItemView
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QButtonGroup
from PySide6.QtWidgets import QComboBox
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QFileDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGridLayout
from PySide6.QtWidgets import QGroupBox
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QHeaderView
from PySide6.QtWidgets import QLabel
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QMenu
from PySide6.QtWidgets import QMessageBox
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QRadioButton
from PySide6.QtWidgets import QSlider
from PySide6.QtWidgets import QSpinBox
from PySide6.QtWidgets import QSplitter
from PySide6.QtWidgets import QTableWidget
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtWidgets import QWidget

from src.Core import Core
from src.cycle import CyclicMeasurementSetupWindow
from src.s_server import SocketWindow
from src.tooltips import tooltips as tt
from src.utils import units_of_measurements
from src.Widgets import AnalyserWidget
from src.Widgets import Graph
from src.Widgets import PixmapWidget
from src.Widgets import TableUnit


# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    cycle_dialog: CyclicMeasurementSetupWindow

    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("Laser Level Webcam Tool")
        self.resize(1100, 650)

        # create a "File" menu and add an "Export CSV" action to it
        file_menu = QMenu("File", self)
        self.menuBar().addMenu(file_menu)
        export_action = QAction("Export CSV", self)
        export_action.triggered.connect(self.export_csv)
        file_menu.addAction(export_action)

        # create "File->Cyclic measurement" action
        cycle_action = QAction("Cyclic measurement", self)
        cycle_action.triggered.connect(self.cycle_measurement_action)
        self.cycle_dialog = CyclicMeasurementSetupWindow(self)
        self.cycle_dialog.onMeasurementTrigger.connect(self.on_cyclic_measurement)
        file_menu.addAction(cycle_action)

        # websocket server action
        websocket_action = QAction("Socket Server", self)
        websocket_action.triggered.connect(self.socket_server_action)
        self.socket_dialog = SocketWindow(self)
        file_menu.addAction(websocket_action)

        # create a QAction for the "Exit" option
        exit_action = QAction("Exit", self)
        exit_action.setShortcut("Ctrl+Q")
        exit_action.triggered.connect(self.close)
        file_menu.addAction(exit_action)

        # create a QAction for the "Source Code" option
        source_action = QAction("Source Code", self)
        source_action.triggered.connect(self.openSourceCode)
        help_menu = self.menuBar().addMenu("Help")
        help_menu.addAction(source_action)

        # Create status bar
        self.status_bar = self.statusBar()

        self.setting_zero = False  # state if the GUI is setting zero
        self.replace_sample = False  # state if we are replcing a sample
        self.table_selected_index = 0  # we keep track of the index so we can reselect it

        self.core = Core()  # where all the magic happens

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
        self.sensor_feed_widget.setToolTip(tt["feed"])
        self.camera_combo = QComboBox()
        self.camera_combo.setToolTip(tt["cameras"])
        camera_device_settings_btn = QPushButton("Device Settings")
        camera_device_settings_btn.setToolTip(tt["cam_device"])
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
        self.analyser_widget.setToolTip(tt["analyser"])
        self.smoothing = QSlider(Qt.Horizontal)
        self.smoothing.setToolTip(tt["smoothing"])
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
        self.subsamples_spin.setToolTip(tt["subsamples"])
        self.subsamples_spin.setRange(1, 9999)
        self.outlier_spin = QSpinBox()
        self.outlier_spin.setToolTip(tt["outliers"])
        self.outlier_spin.setRange(0, 99)
        self.units_combo = QComboBox()
        self.units_combo.setToolTip(tt["units"])
        self.units_combo.addItems(list(units_of_measurements.keys()))
        self.units_combo.setCurrentIndex(1)
        self.sensor_width_spin = QDoubleSpinBox()
        self.sensor_width_spin.setToolTip(tt["sensor_width"])
        self.zero_btn = QPushButton("Zero")
        self.zero_btn.setToolTip(tt["zero_btn"])
        self.sample_btn = QPushButton("Take Sample")
        self.sample_btn.setToolTip(tt["samples"])
        self.replace_btn = QPushButton("Replace")
        self.replace_btn.setToolTip(tt["replace"])
        self.delete_btn = QPushButton("Delete")
        self.delete_btn.setDisabled(True)
        self.sample_btn.setDisabled(True)
        self.replace_btn.setDisabled(True)
        self.sample_table = QTableWidget()
        self.sample_table.setToolTip(tt["table"])
        self.sample_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.sample_table.setSelectionMode(QAbstractItemView.SingleSelection)  # limit selection to a single row
        sample_layout = QGridLayout()
        sample_layout.setContentsMargins(1, 1, 1, 1)
        sample_layout.addWidget(QLabel("Sub Samples #"), 0, 0, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.subsamples_spin, 0, 1, 1, 1)
        sample_layout.addWidget(QLabel("Outlier Removal %"), 0, 2, 1, 2, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.outlier_spin, 0, 4, 1, 1)
        sample_layout.addWidget(QLabel("Units"), 1, 0, 1, 1, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.units_combo, 1, 1, 1, 1)
        sample_layout.addWidget(QLabel("Sensor Width (mm)"), 1, 2, 1, 2, alignment=Qt.AlignRight)
        sample_layout.addWidget(self.sensor_width_spin, 1, 4, 1, 1)
        sample_layout.addWidget(self.zero_btn, 2, 0, 1, 1)
        sample_layout.addWidget(self.sample_btn, 2, 1, 1, 2)
        sample_layout.addWidget(self.replace_btn, 2, 3, 1, 1)
        sample_layout.addWidget(self.delete_btn, 2, 4, 1, 1)
        sample_layout.addWidget(self.sample_table, 3, 0, 1, 5)
        sampler_widget.setLayout(sample_layout)

        # -- Plot --
        self.graph_mode_group = QButtonGroup()

        self.raw_radio = QRadioButton("Raw")
        self.raw_radio.setToolTip(tt["raw"])
        self.graph_mode_group.addButton(self.raw_radio)
        self.flat_radio = QRadioButton("Flattened")
        self.flat_radio.setToolTip(tt["flat"])
        self.graph_mode_group.addButton(self.flat_radio)
        self.graph = Graph(self.core.samples)
        self.graph.setToolTip(tt["plot"])
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

        self.graph.samples = self.core.samples

        for cam in self.core.get_cameras():
            self.camera_combo.addItem(cam)

        self.core.set_camera(self.camera_combo.currentIndex())

        # Signals
        # self.core.OnSensorFeedUpdate.connect(self.sensor_feed_widget.setPixmap)
        self.core.frameWorker.OnAnalyserUpdate.connect(self.analyser_widget.set_data)
        self.sensor_feed_widget.OnHeightChanged.connect(self.analyser_widget.setMaximumHeight)
        self.sensor_feed_widget.OnHeightChanged.connect(
            lambda value: setattr(self.core.frameWorker, "analyser_widget_height", value)
        )
        self.smoothing.valueChanged.connect(lambda value: setattr(self.core.frameWorker, "analyser_smoothing", value))
        self.smoothing.valueChanged.connect(self.smoothing_value)
        self.subsamples_spin.valueChanged.connect(lambda value: setattr(self.core, "subsamples", value))
        self.outlier_spin.valueChanged.connect(lambda value: setattr(self.core, "outliers", value))
        self.units_combo.currentTextChanged.connect(self.core.set_units)
        self.sensor_width_spin.valueChanged.connect(lambda value: setattr(self.core, "sensor_width", value))
        self.zero_btn.clicked.connect(self.zero_btn_cmd)
        self.sample_btn.clicked.connect(self.sample_btn_cmd)
        self.replace_btn.clicked.connect(self.replace_btn_cmd)
        self.delete_btn.clicked.connect(self.delete_btn_cmd)
        self.core.OnSubsampleProgressUpdate.connect(self.subsample_progress_update)
        self.core.OnSampleComplete.connect(self.finished_subsample)
        self.core.OnSampleComplete.connect(self.update_table)
        self.core.OnUnitsChanged.connect(self.update_table)
        self.core.OnUnitsChanged.connect(self.graph.set_units)
        camera_device_settings_btn.clicked.connect(self.extra_controls)
        self.camera_combo.currentIndexChanged.connect(self.core.set_camera)
        self.graph_mode_group.buttonClicked.connect(self.update_graph_mode)
        self.sample_table.itemSelectionChanged.connect(self.hightlight_sample)

        # New
        self.core.frameWorker.OnPixmapChanged.connect(self.sensor_feed_widget.setPixmap)
        self.core.frameWorker.OnCentreChanged.connect(self.core.sample_worker.sample_in)

        # Trigger the state of things
        self.smoothing.setValue(50)
        self.subsamples_spin.setValue(10)
        self.outlier_spin.setValue(30)
        self.units_combo.setCurrentIndex(0)
        self.sensor_width_spin.setValue(5.9)
        self.raw_radio.setChecked(True)
        self.update_graph_mode()  # have to trigger it manually the first time

        self.status_bar.showMessage("Loading first camera", 1000)  # 3 seconds

    def smoothing_value(self, val: float) -> None:
        self.status_bar.showMessage(f"Smoothing: {val}", 1000)  # 3 seconds

    def openSourceCode(self) -> None:
        url = "https://github.com/bhowiebkr/laser-level-webcam"
        QDesktopServices.openUrl(QUrl(url))

    def export_csv(self) -> None:
        # get the file path from the user using a QFileDialog
        file_path, _ = QFileDialog.getSaveFileName(self, "Export CSV", "", "CSV Files (*.csv)")
        if not file_path:
            return

        # open the file and write the data from the QTableWidget to it as CSV
        with open(file_path, "w", newline="") as csv_file:
            writer = csv.writer(csv_file)
            for row in range(self.sample_table.rowCount()):
                row_data = []
                for column in range(self.sample_table.columnCount()):
                    item = self.sample_table.item(row, column)
                    if item is not None:
                        row_data.append(item.text().replace("\u03bc", "u"))
                    else:
                        row_data.append("")
                writer.writerow(row_data)

    def socket_server_action(self) -> None:
        """Show the dialog for the websocket server"""
        self.socket_dialog.message_server.message_received.connect(self.socket_dialog.update_text_edit)
        self.socket_dialog.message_server.take_sample.connect(self.sample_btn_cmd)
        self.socket_dialog.message_server.zero.connect(self.zero_btn_cmd)
        self.core.OnSampleComplete.connect(self.socket_server_sample_complete)

        self.socket_dialog.show()

    def socket_server_sample_complete(self) -> None:
        # Zeroing out samples
        if self.core.setting_zero_sample:
            self.socket_dialog.message_server.send_message("ZERO_COMPLETE")
        # Sample finished
        else:
            sample_val = self.core.samples[-1].y
            self.socket_dialog.message_server.send_message(f"SAMPLE {sample_val}")

    def cycle_measurement_action(self) -> None:
        """Displays the cyclic measurement dialog"""
        self.cycle_dialog.show()

    def on_cyclic_measurement(self) -> None:
        """Executed on each cyclic measurement- acquires a sample (if zeroed), zeroes measurements otherwise."""
        if self.sample_btn.isEnabled():
            self.sample_btn.click()
        else:
            self.zero_btn.click()

    def hightlight_sample(self) -> None:
        index = self.sample_table.currentRow()
        self.graph.set_selected_index(index)

    def extra_controls(self) -> None:
        # If the command exists
        if shutil.which("ffmpeg"):
            cmd = f'ffmpeg -f dshow -show_video_device_dialog true -i video="{self.camera_combo.currentText()}"'
            subprocess.Popen(cmd, shell=True)
        else:
            print("missing!")
            msg = QMessageBox()
            msg.setWindowTitle("Missing FFMPEG")
            msg_str = "FFMPEG is not installed or is not found in the Windows path. "
            msg_str += "Please download, install, and add it to your Windows path"
            msg.setText(msg_str)
            msg.exec_()

    def update_graph_mode(self) -> None:
        checked_button = self.graph_mode_group.checkedButton()
        self.graph.set_mode(checked_button.text())

    def update_table(self) -> None:
        units = self.core.units
        header_names = [
            f"Measured ({units})",
            f"Flattened ({units})",
            f"Below Max ({units})",
            f"Above Min ({units})",
        ]
        self.sample_table.setColumnCount(len(header_names))
        self.sample_table.setHorizontalHeaderLabels(header_names)
        header = self.sample_table.horizontalHeader()
        header.setSectionResizeMode(QHeaderView.Stretch)

        # Delete the rows
        self.sample_table.setRowCount(0)

        for sample in self.core.samples:
            # Check if there are enough rows in the table widget, and add a new row if necessary
            if sample.x >= self.sample_table.rowCount():
                self.sample_table.insertRow(sample.x)

            for col, val in enumerate([sample.y, sample.linYError, sample.shim, sample.scrape]):
                # measured value
                cell = TableUnit()
                cell.value = val
                cell.units = self.core.units
                self.sample_table.setItem(sample.x, col, cell)

        # if there are rows and nothing is selected: select an index
        if self.sample_table.rowCount() and not self.sample_table.selectedIndexes():
            self.sample_table.selectRow(0)

        self.sample_table.selectRow(self.table_selected_index)
        self.graph.update_graph()

    def finished_subsample(self) -> None:
        """
        Sample complete. Reset the GUI back to the default state
        """
        self.zero_btn.setEnabled(True)
        self.sample_btn.setEnabled(True)
        self.replace_btn.setEnabled(True)
        self.delete_btn.setEnabled(True)

        if self.setting_zero is True:
            self.zero_btn.setText("Zero")
            self.setting_zero = False
        else:
            if self.replace_sample:
                self.replace_btn.setText("Replace Sample")
                self.replace_sample = False
            else:
                self.sample_btn.setText("Take Sample")

    def subsample_progress_update(self, sample_total: list[int]) -> None:
        """
        Progress update on either zero or sample button
        """

        sample = sample_total[0]
        total = sample_total[1]

        if self.setting_zero is True:
            self.zero_btn.setText(f"{sample}/{total}")
        else:
            if self.replace_sample:
                self.replace_btn.setText(f"{sample}/{total}")
            else:
                self.sample_btn.setText(f"{sample}/{total}")

    def zero_btn_cmd(self) -> None:
        """
        Calls the sample button command but sets a flag so we know the GUI is in a state of setting the zero value
        """
        self.table_selected_index = 0

        self.setting_zero = True
        self.replace_sample = False
        self.zero_btn.setDisabled(True)
        self.sample_btn.setDisabled(True)
        self.replace_btn.setDisabled(True)

        self.core.samples[:] = []  # clear list in-place without changing it's reference
        self.graph.update_graph()
        self.core.start_sample(self.setting_zero, replacing_sample=False, replacing_sample_index=0)

    def sample_btn_cmd(self) -> None:
        """
        Calls on Core to take a sample
        """
        self.table_selected_index = self.sample_table.currentRow()

        self.zero_btn.setDisabled(True)
        self.sample_btn.setDisabled(True)
        self.replace_btn.setDisabled(True)
        self.core.start_sample(self.setting_zero, replacing_sample=False, replacing_sample_index=0)

    def replace_btn_cmd(self) -> None:
        """
        Call for when we are replacing a sample
        """
        self.table_selected_index = self.sample_table.currentRow()

        self.zero_btn.setDisabled(True)
        self.sample_btn.setDisabled(True)
        self.replace_btn.setDisabled(True)
        self.replace_sample = True
        index = self.sample_table.currentRow()
        self.core.start_sample(self.setting_zero, replacing_sample=True, replacing_sample_index=index)

    def delete_btn_cmd(self) -> None:
        self.table_selected_index = self.sample_table.currentRow()

        self.core.delete_sample(self.table_selected_index)

        self.table_selected_index -= 1
        self.update_table()

    def closeEvent(self, event: QCloseEvent) -> None:
        self.socket_dialog.closeEvent(event)  # Make sure the socket server is close to not hang the system
        self.core.workerThread.quit()
        self.core.workerThread.wait()
        self.core.sampleWorkerThread.quit()
        self.core.sampleWorkerThread.wait()
        self.deleteLater()
        super().closeEvent(event)


def start() -> None:
    app = QApplication(sys.argv)
    qdarktheme.setup_theme(additional_qss="QToolTip {color: black;}")

    window = MainWindow()

    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    start()
