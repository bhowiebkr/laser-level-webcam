#!/usr/bin/python           # This is client.py file
from __future__ import annotations

import socket  # Import socket module
import sys

import matplotlib.pyplot as plt
import numpy as np
import qdarktheme
from matplotlib.backends.backend_qt5agg import FigureCanvasQTAgg as FigureCanvas
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtWidgets import QWidget


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


def send_recieve(cmd: str) -> str:
    server.send(cmd.encode("utf-8"))
    recv = server.recv(1024).decode("utf-8")
    return recv


def ready() -> bool:
    s.poll()
    return not s.estop and s.enabled and (s.homed.count(1) == s.joints) and (s.interp_state == linuxcnc.INTERP_IDLE)


def cmd(cmd: str) -> None:
    c.mdi(cmd)
    print(f"Sent: {cmd}")
    c.wait_complete()  # wait until mode switch executed


def main() -> None:
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


# Define the main window
class MainWindow(QMainWindow):  # type: ignore
    def __init__(self) -> None:
        super().__init__()

        self.setWindowTitle("LinuxCNC Remote Driver")
        self.resize(1100, 650)

        central_widget = QWidget(self)
        self.setCentralWidget(central_widget)

        layout = QVBoxLayout(central_widget)

        # Create a Matplotlib Figure and Axes
        self.figure = plt.figure()
        self.canvas = FigureCanvas(self.figure)
        layout.addWidget(self.canvas)

        # Create a 3D subplot
        self.ax = self.figure.add_subplot(111, projection="3d")

        # Generate some sample data
        x = np.linspace(-5, 5, 100)
        y = np.linspace(-5, 5, 100)
        X, Y = np.meshgrid(x, y)
        Z = np.sin(np.sqrt(X**2 / 10 + Y**2))

        # Plot the 3D surface
        self.ax.plot_surface(X, Y, Z, cmap="viridis")
        self.ax.set_xlabel("X Label")
        self.ax.set_ylabel("Y Label")
        self.ax.set_zlabel("Z Label")

        # Adjust the layout
        self.ax.view_init(elev=20, azim=-45)
        self.ax.dist = 10

        self.canvas.draw()


def start() -> None:
    app = QApplication(sys.argv)
    qdarktheme.setup_theme(additional_qss="QToolTip {color: black;}")

    window = MainWindow()

    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    start()
