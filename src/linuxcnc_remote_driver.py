#!/usr/bin/python           # This is client.py file
from __future__ import annotations

import pickle
import re
import sys
from pathlib import Path
from typing import Any
from typing import Dict

import numpy as np
import plotly
import plotly.graph_objects as go
import plotly.io as io
import qdarktheme
from PySide6.QtCore import QSettings
from PySide6.QtCore import QUrl
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QComboBox
from PySide6.QtWidgets import QFileDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGridLayout
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QLineEdit
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtWidgets import QWidget

from src.CNC_jobs.probe import ProbeJob

# from src.CNC_jobs.probe_and_machine import ProbeAndMachineJob
# from src.CNC_jobs.test_job import TestJob

io.templates.default = "plotly_dark"


DEV_MODE = False  # Use a bunch of dummy things such as fake linuxcnc module
SKIP_CONNECTION = False  # Work without connecting to a socket


def camel_case_split(str: str) -> str:
    return " ".join(re.findall(r"[A-Z](?:[a-z]+|[A-Z]*(?=[A-Z]|$))", str))


# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    OnConnect = Signal(dict)

    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("LinuxCNC Remote Driver")
        self.resize(869, 839)

        self.graph_size = 600

        # Layouts
        central_widget = QWidget(self)
        self.setCentralWidget(central_widget)
        main_layout = QHBoxLayout()
        central_widget.setLayout(main_layout)
        self.left_layout = QVBoxLayout()
        btn_layout = QGridLayout()
        form = QFormLayout()

        # we create an instance of QWebEngineView and set the html code
        self.plot_widget = QWebEngineView()
        self.plot_widget.setMinimumWidth(self.graph_size + 20)
        self.plot_widget.setMinimumHeight(self.graph_size + 20)

        self.connect_btn = QPushButton("Connect")
        self.update_btn = QPushButton("Update")
        self.start_btn = QPushButton("Start")
        self.start_btn.setDisabled(True)
        self.stop_btn = QPushButton("Stop")
        self.stop_btn.setDisabled(True)
        self.ip_line = QLineEdit()
        self.port_line = QLineEdit()

        self.save_btn = QPushButton("Save Graph")
        self.load_btn = QPushButton("Load Graph")
        self.export_btn = QPushButton("Export Graph")

        self.job_type_combo = QComboBox()

        self.jobs_types = {}
        self.job = ProbeJob()

        self.data = np.zeros((5, 5), dtype=np.float64)

        for job in [ProbeJob]:
            job_name = str(job.__name__)
            job_name = camel_case_split(job_name)
            self.jobs_types[job_name] = job
            self.job_type_combo.addItem(job_name)

        # Add layouts
        form.addRow("IP Address", self.ip_line)
        form.addRow("Port", self.port_line)
        form.addRow("Job", self.job_type_combo)

        btn_layout.addWidget(self.connect_btn, 1, 1, 1, 2)
        btn_layout.addWidget(self.start_btn, 2, 1)
        btn_layout.addWidget(self.stop_btn, 2, 2)
        btn_layout.addWidget(self.update_btn, 3, 1, 1, 2)
        btn_layout.addWidget(self.save_btn, 4, 1)
        btn_layout.addWidget(self.load_btn, 4, 2)
        btn_layout.addWidget(self.export_btn, 5, 1, 1, 2)

        self.left_layout.addLayout(form)

        self.left_layout.addWidget(self.job)
        self.left_layout.addStretch()
        self.left_layout.addLayout(btn_layout)

        main_layout.addLayout(self.left_layout)
        main_layout.addWidget(self.plot_widget)

        # Logic
        self.start_btn.clicked.connect(self.start_btn_update_GUI)
        self.job_type_combo.currentIndexChanged.connect(self.job_changed)
        self.update_btn.clicked.connect(self.update_graph)
        self.save_btn.clicked.connect(self.save_np)
        self.load_btn.clicked.connect(self.load_np)
        self.export_btn.clicked.connect(self.export_np)

        # Load GUI saved defaults
        settings = QSettings("linuxcnc_remote_driver", "LinuxCNCRemoteDriver")
        if settings.contains("geometry"):
            self.restoreGeometry(settings.value("geometry"))
        if settings.contains("ip"):
            self.ip_line.setText(settings.value("ip"))
        if settings.contains("port"):
            self.port_line.setText(settings.value("port"))

        # Load the current job GUI
        self.job_changed()
        self.update_graph()

    def save_np(self) -> None:
        options = QFileDialog.Options()
        file_path, _ = QFileDialog.getSaveFileName(
            self, "Save NumPy Array", "", "Pickle Files (*.pkl);;All Files (*)", options=options
        )

        if file_path:
            # Use pickle to save the NumPy array to the selected file
            with open(file_path, "wb") as file:
                pickle.dump(self.data, file)

            print(f"NumPy array saved to {file_path}")

    def load_np(self) -> None:
        options = QFileDialog.Options()
        file_path, _ = QFileDialog.getOpenFileName(
            self, "Load NumPy Array", "", "Pickle Files (*.pkl);;All Files (*)", options=options
        )

        if file_path:
            try:
                # Use pickle to load the NumPy array from the selected file
                with open(file_path, "rb") as file:
                    loaded_array = pickle.load(file)

                # Update the stored data with the loaded array
                self.data = loaded_array

                print(f"NumPy array loaded from {file_path}: {self.data}")
            except Exception as e:
                print(f"Error loading NumPy array: {e}")

        self.update_graph()

    def export_np(self) -> None:
        options = QFileDialog.Options()
        file_path, _ = QFileDialog.getSaveFileName(
            self, "Export NumPy Array to CSV", "", "CSV Files (*.csv);;All Files (*)", options=options
        )

        if file_path:
            try:
                # Save the NumPy array to a CSV file
                np.savetxt(
                    file_path, self.data, delimiter=",", fmt="%.6f"
                )  # fmt = 6 decimal places. Change if you need more

                print(f"NumPy array exported to {file_path} in CSV format.")
            except Exception as e:
                print(f"Error exporting NumPy array to CSV: {e}")

    def job_changed(self) -> None:
        job_name = str(self.job_type_combo.currentText())
        old_widget = self.job
        new_widget = self.jobs_types[job_name]()
        self.left_layout.replaceWidget(old_widget, new_widget)
        self.job = new_widget
        old_widget.deleteLater()

        # Hook up the new connections
        self.connect_btn.clicked.connect(
            lambda ip=self.ip_line.text(), port=self.port_line.text(): self.job.driver.connect_to_host(ip, port)
        )
        self.job.driver.connection_made.connect(self.connect_update_GUI)
        self.stop_btn.clicked.connect(self.job.driver.stop)
        self.job.data_changed.connect(self.update_data)
        self.start_btn.clicked.connect(self.job.start_driver)
        self.job.driver.job_stopped.connect(self.stop_update_GUI)

    def update_data(self, data: Dict[str, Any]) -> None:
        print("updating data:", data)
        self.data = data

    def connect_update_GUI(self) -> None:
        print("updating the GUI that a connection was made")
        self.connect_btn.setDisabled(True)
        self.start_btn.setEnabled(True)

    def start_btn_update_GUI(self) -> None:
        self.start_btn.setDisabled(True)
        self.stop_btn.setEnabled(True)

    def stop_update_GUI(self) -> None:
        self.start_btn.setEnabled(True)
        self.stop_btn.setDisabled(True)
        self.update_graph()

    def closeEvent(self, event: QCloseEvent) -> None:
        print("in close event")
        self.settings = QSettings("linuxcnc_remote_driver", "LinuxCNCRemoteDriver")
        self.settings.setValue("geometry", self.saveGeometry())
        self.settings.setValue("ip", self.ip_line.text())
        self.settings.setValue("port", self.port_line.text())
        self.deleteLater()
        QWidget.closeEvent(self, event)

    def update_graph(self) -> None:
        print("Updating Graph")
        print(self.data)
        graph = go.Surface(z=self.data)
        fig = go.Figure(
            data=[graph],
        )

        fig.update_layout(
            title="Surface Height",
            autosize=False,
            width=self.graph_size,
            height=self.graph_size,
            margin=dict(l=0, r=0, b=0, t=40),
        )

        self.plot = plotly.offline.plot(
            fig,
            output_type="div",
            image_width="100%",
            image_height="100%",
            include_plotlyjs=False,
        )

        # we create html code of the figure
        html = '<html><script src="plotly.js"></script><body style = "background:black">'
        html += self.plot
        html += "</body></html>"
        base = QUrl.fromLocalFile(str(Path(__file__).resolve()))
        self.plot_widget.setHtml(html, baseUrl=base)


def start() -> None:
    app = QApplication(sys.argv)
    qdarktheme.setup_theme(additional_qss="QToolTip {color: black;}")

    window = MainWindow()

    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    start()
