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
from PySide6.QtCore import QObject
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


io.templates.default = "plotly_dark"


IN_LINUXCNC = False
if sys.platform == "linux":
    IN_LINUXCNC = True
    import linuxcnc


DEV_MODE = True  # Use a bunch of dummy things such as fake linuxcnc module
SKIP_CONNECTION = False  # Work without connecting to a socket


class Server(QObject):  # type: ignore
    server = socket.socket()  # Create a socket object
    IP = "192.168.1.140"  # Get local machine name
    PORT = 1234  # Reserve a port for your service.

    def __init__(self) -> None:
        super().__init__()

    def connect_socket(self) -> bool:
        print("Connecting")
        connected = False
        num_fails = 0
        while not connected:
            if num_fails >= 3:
                break
            try:
                self.server.connect((self.IP, self.PORT))
                connected = True
                return True
            except Exception as e:
                print(
                    f"Failed to connect with the following: {e}. Using IP {self.IP}, Port: \
                        {self.PORT}, Try: {num_fails+1}/3"
                )  # print why and try again
                num_fails += 1
                continue
        print("...Stopped")
        return False

    def send_recieve(self, cmd: str) -> str:
        if SKIP_CONNECTION:
            recv = f"Fake_Data: {random.uniform(-1.0, 1.0)}"
        else:
            self.server.send(cmd.encode("utf-8"))
            recv = self.server.recv(1024).decode("utf-8")
        return recv


class LinuxDriver(QObject):  # type: ignore
    OnSampleReceived = Signal(list)

    def __init__(self) -> None:
        super().__init__()
        self.server = Server()

        self.sample_x_length = 0.0
        self.sample_y_length = 0.0
        self.sample_distance = 0.0

        if IN_LINUXCNC:
            self.s = linuxcnc.stat()  # type: ignore
            self.c = linuxcnc.command()  # type: ignore

    def loop(self) -> None:
        if self.ready():
            if not DEV_MODE:
                self.c.mode(linuxcnc.MODE_MDI)  # type: ignore
                self.c.wait_complete()  # wait until mode switch executed
            self.cmd("G64")  # Path blending best possible speed

            radius = 2  # milling radius
            height = 4  # safe height
            dist = self.sample_distance

            x_holes = int(self.sample_x_length / self.sample_distance)
            y_holes = int(self.sample_y_length / self.sample_distance)
            feed = 5000

            print(self.server.send_recieve("ZERO"))

            for y in range(y_holes):
                for x in range(x_holes):
                    if QThread.currentThread().isInterruptionRequested():
                        print("Job Stopped")
                        return

                    print(f"index x: {x} y: {y}")

                    # Move down
                    sample = float(self.server.send_recieve("TAKE_SAMPLE").split(" ")[1])
                    self.OnSampleReceived.emit([x, y, sample])
                    self.cmd(f"G0 X{x*dist} Y{y*dist} Z{height + (sample * 100)}")
                    self.cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")
                    self.cmd(f"G0 X{x*dist} Y{y*dist} Z0")

                    # Circle
                    self.cmd(f"G0 X{x*dist -radius} Y{y*dist} Z0")
                    self.cmd(f"G02 X{x*dist -radius} Y{y*dist} I{radius} J0 F{feed}")
                    self.cmd(f"G0 X{x*dist } Y{y*dist} Z0")

                    # Move up
                    self.cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")

    def ready(self) -> bool:
        if DEV_MODE:
            return True
        self.s.poll()
        return (
            not self.s.estop
            and self.s.enabled
            and (self.s.homed.count(1) == self.s.joints)
            and (self.s.interp_state == linuxcnc.INTERP_IDLE)  # type: ignore
        )

    def set_sample_x_length(self, length: float) -> None:
        self.sample_x_length = length

    def set_sample_y_length(self, length: float) -> None:
        self.sample_y_length = length

    def set_sample_distance(self, distance: float) -> None:
        self.sample_distance = distance

    def cmd(self, cmd: str) -> None:
        if DEV_MODE:
            print(f"Sent: {cmd}")
            time.sleep(0.1)
        else:
            self.c.mdi(cmd)
            print(f"Sent: {cmd}")
            self.c.wait_complete()  # wait until mode switch executed

    def start(self) -> None:
        print("Running")
        if DEV_MODE:
            self.loop()
            return

        if not self.is_LinuxCNC():
            return
        self.loop()
        print("Finished")


# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("LinuxCNC Remote Driver")
        self.resize(869, 839)

        self.graph_size = 800

        # Components
        self.lcnc_driver = LinuxDriver()
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
