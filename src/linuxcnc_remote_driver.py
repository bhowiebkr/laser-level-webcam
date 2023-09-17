#!/usr/bin/python           # This is client.py file
from __future__ import annotations

import random
import socket  # Import socket module
import sys
import time
from pathlib import Path

import numpy as np
import plotly
import plotly.graph_objects as go
import plotly.io as io
import qdarktheme
from PySide6.QtCore import QSettings
from PySide6.QtCore import QThread
from PySide6.QtCore import QUrl
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWebEngineWidgets import QWebEngineView
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGridLayout
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QLineEdit
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtWidgets import QWidget

from src.CNC_jobs.test_job import TestJob


io.templates.default = "plotly_dark"





DEV_MODE = False  # Use a bunch of dummy things such as fake linuxcnc module
SKIP_CONNECTION = False  # Work without connecting to a socket






# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    OnIPChanged = Signal(str)
    OnPortChanged = Signal(int)
    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("LinuxCNC Remote Driver")
        self.resize(869, 839)

        self.graph_size = 800

        # Components
        self.lcnc_driver = TestJob()
        self.lcnc_driver_thread = QThread()
        self.lcnc_driver.moveToThread(self.lcnc_driver_thread)
        self.lcnc_driver_thread.start()

        # Layouts
        central_widget = QWidget(self)
        self.setCentralWidget(central_widget)
        main_layout = QHBoxLayout()
        central_widget.setLayout(main_layout)
        left_layout = QVBoxLayout()
        btn_layout = QGridLayout()
        form = QFormLayout()

        # Widgets

        self.data = np.empty((2, 2))

        # we create an instance of QWebEngineView and set the html code
        self.plot_widget = QWebEngineView()
        self.plot_widget.setMinimumWidth(self.graph_size + 20)
        self.plot_widget.setMinimumHeight(self.graph_size + 20)

        self.connect_btn = QPushButton("Connect")
        self.disconnect_btn = QPushButton("Disconnect")
        self.disconnect_btn.setDisabled(True)
        self.update_btn = QPushButton("Update")
        self.start_btn = QPushButton("Start")
        self.start_btn.setDisabled(True)
        self.stop_btn = QPushButton("Stop")
        self.stop_btn.setDisabled(True)
        self.ip_line = QLineEdit()
        self.port_line = QLineEdit()
        self.sample_X_line = QDoubleSpinBox()
        self.sample_Y_line = QDoubleSpinBox()
        self.sample_distance = QDoubleSpinBox()

        # Add layouts
        form.addRow("IP Address", self.ip_line)
        form.addRow("Port", self.port_line)

        form.addRow("Sample X Length", self.sample_X_line)
        form.addRow("Sample Y Length", self.sample_Y_line)
        form.addRow("Sample Distance", self.sample_distance)

        btn_layout.addWidget(self.connect_btn, 1, 1)
        btn_layout.addWidget(self.disconnect_btn, 1, 2)
        btn_layout.addWidget(self.start_btn, 2, 1)
        btn_layout.addWidget(self.stop_btn, 2, 2)
        btn_layout.addWidget(self.update_btn, 3, 1)

        left_layout.addLayout(form)
        left_layout.addStretch()
        left_layout.addLayout(btn_layout)

        main_layout.addLayout(left_layout)
        main_layout.addWidget(self.plot_widget)

        # Logic
        self.connect_btn.clicked.connect(self.connect_update_GUI)
        self.connect_btn.clicked.connect(self.lcnc_driver.server.connect_socket)
        self.disconnect_btn.clicked.connect(self.lcnc_driver.server.disconnect)
        self.start_btn.clicked.connect(self.start_btn_update_GUI)
        self.start_btn.clicked.connect(self.lcnc_driver.start)
        self.stop_btn.clicked.connect(self.stop)
        self.update_btn.clicked.connect(self.update_data)
        self.lcnc_driver.OnSampleReceived.connect(self.sample_in)
        self.sample_X_line.valueChanged.connect(self.lcnc_driver.set_sample_x_length)
        self.sample_Y_line.valueChanged.connect(self.lcnc_driver.set_sample_y_length)
        self.sample_distance.valueChanged.connect(self.lcnc_driver.set_sample_distance)

        self.sample_X_line.valueChanged.connect(self.update_data_shape)
        self.sample_Y_line.valueChanged.connect(self.update_data_shape)
        self.sample_distance.valueChanged.connect(self.update_data_shape)
        self.start_btn.clicked.connect(self.update_data_shape)  # resets the shape too
        self.ip_line.textChanged.connect(self.IP_changed)
        self.port_line.textChanged.connect(self.port_changed)

        self.OnIPChanged.connect(self.lcnc_driver.server.set_IP)
        self.OnPortChanged.connect(self.lcnc_driver.server.set_port)


        # Load GUI saved defaults
        settings = QSettings("linuxcnc_remote_driver", "LinuxCNCRemoteDriver")
        if settings.contains("geometry"):
            self.restoreGeometry(settings.value("geometry"))
        if settings.contains("ip"):
            self.ip_line.setText(settings.value("ip"))
        if settings.contains("port"):
            self.port_line.setText(settings.value("port"))
        if settings.contains("sample_x_length"):
            self.sample_X_line.setValue(float(settings.value("sample_x_length")))
        if settings.contains("sample_y_length"):
            self.sample_Y_line.setValue(float(settings.value("sample_y_length")))
        if settings.contains("sample_distance"):
            self.sample_distance.setValue(float(settings.value("sample_distance")))

        #self.OnIPChanged.emit()
        #self.OnPortChanged.emit()

    def IP_changed(self):
        ip = self.ip_line.text()
        self.OnIPChanged.emit(str(ip))

    def port_changed(self):
        port = self.port_line.text()
        self.OnPortChanged.emit(int(port))

    def sample_in(self, sample: list[int | int | float]) -> None:
        print(f"Sample into the GUI is: {sample}")
        x = sample[0]
        y = sample[1]
        val = sample[2]

        self.data[x][y] = val

    def connect_update_GUI(self) -> None:
        self.connect_btn.setDisabled(True)
        self.start_btn.setEnabled(True)
        self.disconnect_btn.setEnabled(True)

    def start_btn_update_GUI(self) -> None:
        self.start_btn.setDisabled(True)
        self.stop_btn.setEnabled(True)

    def stop(self) -> None:
        self.lcnc_driver_thread.requestInterruption()
        self.stop_btn.setDisabled(True)
        self.start_btn.setEnabled(True)

    def closeEvent(self, event: QCloseEvent) -> None:
        self.settings = QSettings("linuxcnc_remote_driver", "LinuxCNCRemoteDriver")
        self.settings.setValue("geometry", self.saveGeometry())
        self.settings.setValue("ip", self.ip_line.text())
        self.settings.setValue("port", self.port_line.text())
        self.settings.setValue("sample_x_length", self.sample_X_line.value())
        self.settings.setValue("sample_y_length", self.sample_Y_line.value())
        self.settings.setValue("sample_distance", self.sample_distance.value())
        QWidget.closeEvent(self, event)

    def update_data_shape(self) -> None:
        x = self.sample_X_line.value()
        y = self.sample_Y_line.value()
        d = self.sample_distance.value()

        if d == 0:
            return

        x_shape = int(x / d)
        y_shape = int(y / d)

        self.data = np.zeros((x_shape, y_shape), dtype=np.float64)

        print(f"data shape: {self.data.shape} type: {self.data.dtype}")

        self.update_data()

    def update_data(self) -> None:
        graph = go.Surface(z=self.data)
        fig = go.Figure(
            data=[graph],
        )

        fig.update_layout(
            title="Surface Height",
            autosize=False,
            width=self.graph_size,
            height=self.graph_size,
            margin=dict(l=65, r=50, b=65, t=90),
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
