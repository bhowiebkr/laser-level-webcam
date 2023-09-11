from __future__ import annotations

import sys

import linuxcnc
import qdarktheme
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QMainWindow

s = linuxcnc.stat()
c = linuxcnc.command()


def ready() -> bool:
    s.poll()
    return not s.estop and s.enabled and (s.homed.count(1) == s.joints) and (s.interp_state == linuxcnc.INTERP_IDLE)


# @run_and_wait
def cmd(cmd: str) -> None:
    c.mdi(cmd)
    print(f"Sent: {cmd}")
    c.wait_complete()  # wait until mode switch executed


def run() -> None:
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

        for y in range(y_holes):
            for x in range(x_holes):
                # Move down
                cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")
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

        # Layouts

        # Widgets
        """
        start, pause, stop
        start client, check client, server ip address
        grid length, width, sample density

        """

        # Attach Widgets


def start() -> None:
    app = QApplication(sys.argv)
    qdarktheme.setup_theme(additional_qss="QToolTip {color: black;}")

    window = MainWindow()

    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    start()
