import PySide6.QtGui
from src.CNC_jobs.common import LinuxDriver
from PySide6.QtCore import QThread
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QGroupBox
from PySide6.QtGui import QCloseEvent


import sys

IN_LINUXCNC = False
if sys.platform == "linux":
    IN_LINUXCNC = True
    import linuxcnc


class ProbeDriver(LinuxDriver):
    def init(self, client):
        super().__init__()

        self.client = client

        self.sample_x_length = 0.0
        self.sample_y_length = 0.0
        self.sample_distance = 0.0

    def loop(self) -> None:
        if self.ready():
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
                    sample = float(
                        self.server.send_recieve("TAKE_SAMPLE").split(" ")[1]
                    )
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

    def set_sample_x_length(self, length: float) -> None:
        self.sample_x_length = length

    def set_sample_y_length(self, length: float) -> None:
        self.sample_y_length = length

    def set_sample_distance(self, distance: float) -> None:
        self.sample_distance = distance


class Probe(QGroupBox):
    def __init__(self, client):
        QGroupBox.__init__(self)
        self.setTitle("Probe Job")

        self.driver = ProbeDriver(client)
        self.driver_thread = QThread()
        self.driver.moveToThread(self.driver_thread)
        self.driver_thread.start()

        form = QFormLayout()
        self.setLayout(form)

        self.sample_X_line = QDoubleSpinBox()
        self.sample_Y_line = QDoubleSpinBox()
        self.sample_distance = QDoubleSpinBox()
        self.probe_height_offset = QDoubleSpinBox()

        form.addRow("Sample X Length", self.sample_X_line)
        form.addRow("Sample Y Length", self.sample_Y_line)
        form.addRow("Sample Distance", self.sample_distance)
        form.addRow("Probe Height", self.probe_height_offset)

        self.sample_X_line.valueChanged.connect(self.driver.set_sample_x_length)
        self.sample_Y_line.valueChanged.connect(self.driver.set_sample_y_length)
        self.sample_distance.valueChanged.connect(self.driver.set_sample_distance)

        # self.driver_thread.error.connect(self.driver_thread.terminate)

    def closeEvent(self, event: QCloseEvent) -> None:
        self.driver_thread.requestInterruption()

        return super().closeEvent(event)
