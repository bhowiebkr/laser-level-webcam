#!/usr/bin/python           # This is client.py file
from __future__ import annotations

import socket  # Import socket module
import sys
from pathlib import Path

import numpy as np
import pandas as pd
import plotly
import plotly.graph_objects as go
import plotly.io as io
import qdarktheme
from PySide6.QtCore import QUrl
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtWidgets import QWidget

io.templates.default = "plotly_dark"
from PySide6.QtWebEngineWidgets import QWebEngineView


IP = "192.168.1.140"  # Get local machine name
PORT = 1234  # Reserve a port for your service.

if sys.platform == "linux":
    import linuxcnc

    try:
        server = socket.socket()  # Create a socket object
        server.connect((IP, PORT))
    except Exception:
        pass

    s = linuxcnc.stat()
    c = linuxcnc.command()


CONTINUE = False


def send_recieve(cmd: str) -> str:
    server.send(cmd.encode("utf-8"))
    recv = server.recv(1024).decode("utf-8")
    return recv


def ready() -> bool:
    s.poll()
    return not s.estop and s.enabled and (s.homed.count(1) == s.joints) and (s.interp_state == linuxcnc.INTERP_IDLE)


def cmd(cmd: str) -> None:
    global CONTINUE
    if CONTINUE:
        c.mdi(cmd)
        print(f"Sent: {cmd}")
        c.wait_complete()  # wait until mode switch executed


def main() -> None:
    if sys.platform != "linux":
        print("This must be run on LinuxCNC")
        return

    global CONTINUE
    if ready():
        c.mode(linuxcnc.MODE_MDI)
        c.wait_complete()  # wait until mode switch executed
        cmd("G64")  # Path blending best possible speed

        radius = 2  # milling radius
        height = 4  # safe height
        dist = 10  # hole distance

        x_holes = 5
        y_holes = 3
        feed = 5000

        print(send_recieve("ZERO"))

        for y in range(y_holes):
            for x in range(x_holes):
                if CONTINUE:
                    # Move down
                    sample = float(send_recieve("TAKE_SAMPLE").split(" ")[1])
                    cmd(f"G0 X{x*dist} Y{y*dist} Z{height + (sample * 100)}")
                    cmd(f"G0 X{x*dist} Y{y*dist} Z0")

                    # Circle
                    cmd(f"G0 X{x*dist -radius} Y{y*dist} Z0")
                    cmd(f"G02 X{x*dist -radius} Y{y*dist} I{radius} J0 F{feed}")
                    cmd(f"G0 X{x*dist } Y{y*dist} Z0")

                    # Move up
                    cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")


def start_btn_cmd() -> None:
    global CONTINUE
    CONTINUE = True
    main()


def stop_btn_cmd() -> None:
    global CONTINUE
    CONTINUE = False


# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("LinuxCNC Remote Driver")
        self.resize(869, 839)

        central_widget = QWidget(self)
        self.setCentralWidget(central_widget)
        main_layout = QVBoxLayout()
        central_widget.setLayout(main_layout)
        btn_layout = QHBoxLayout()

        self.data = pd.read_csv(
            "https://raw.githubusercontent.com/plotly/datasets/master/api_docs/mt_bruno_elevation.csv"
        )

        graph = go.Surface(z=self.data.values)

        fig = go.Figure(
            data=[graph],
        )

        fig.update_layout(
            title="Surface Height",
            autosize=False,
            width=800,
            height=800,
            margin=dict(l=65, r=50, b=65, t=90),
        )

        # we create html code of the figure
        html = '<html><script src="plotly.js"></script><body style = "background:black">'
        html += plotly.offline.plot(
            fig,
            output_type="div",
            image_width="100%",
            image_height="100%",
            include_plotlyjs=False,
        )
        html += "</body></html>"

        # we create an instance of QWebEngineView and set the html code
        plot_widget = QWebEngineView()
        base = QUrl.fromLocalFile(str(Path(__file__).resolve()))
        plot_widget.setHtml(html, baseUrl=base)

        self.start_btn = QPushButton("Start")
        self.stop_btn = QPushButton("Stop")

        btn_layout.addWidget(self.start_btn)
        btn_layout.addWidget(self.stop_btn)

        # set the QWebEngineView instance as main widget
        main_layout.addLayout(btn_layout)
        main_layout.addWidget(plot_widget)

        self.start_btn.clicked.connect(start_btn_cmd)
        self.stop_btn.clicked.connect(stop_btn_cmd)


def start() -> None:
    app = QApplication(sys.argv)
    qdarktheme.setup_theme(additional_qss="QToolTip {color: black;}")

    window = MainWindow()

    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    start()
