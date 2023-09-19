from src.CNC_jobs.common import LinuxDriver
from PySide6.QtCore import QThread
from PySide6.QtCore import Qt
import numpy as np

from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QGroupBox
from PySide6.QtGui import QCloseEvent
from PySide6.QtCore import Signal

import sys

IN_LINUXCNC = False
if sys.platform == "linux":
    IN_LINUXCNC = True
    import linuxcnc


class ProbeDriver(LinuxDriver):
    OnSampleReceived = Signal(list)

    def init(self, client):
        super().__init__()

        self.client = client

    def loop(self, params) -> None:
        x_holes = params["x_holes"]
        y_holes = params["y_holes"]
        dist = params["dist"]
        lift = params["lift_height"]

        print("Starting LinuxCNC job")
        self.c.mode(linuxcnc.MODE_MDI)  # type: ignore
        self.c.wait_complete()  # wait until mode switch executed
        self.cmd("G64")  # Path blending best possible speed

        radius = 2  # milling radius
        height = 4  # safe height

        feed = 5000
        # Move the W axis back to machine coord zero
        self.cmd("G53 G0 W0Z0")

        # Move to the start position
        self.cmd("G54 G0 X0Y0")

        # Move W to lift height
        self.cmd(f"G0 W{lift}")

        # Move down to W zero for setting zero
        self.cmd(f"G1 F2000 W0")

        # Zero out the webcam sensor
        print(self.client.send_recieve("ZERO"))

        # Move W to lift height (Starting position)
        self.cmd(f"G0 W{lift}")

        for y in range(y_holes):
            for x in range(x_holes):
                # Exit early
                if QThread.currentThread().isInterruptionRequested():
                    self.client.close_socket()
                    print("Job Stopped")
                    return

                # Move down and take a sample
                self.cmd(f"G1 F2000 W0")
                sample = float(self.client.send_recieve("TAKE_SAMPLE").split(" ")[1])
                self.OnSampleReceived.emit(
                    [x, y, sample * 1000]
                )  # convert sample mm to um

                # Move up
                self.cmd(f"G0 W{lift}")

                # Goto next sample location
                self.cmd(f"G0 X{x*dist} Y{y*dist}")

            print("not ready")

        # Move the W axis back to machine coord zero
        self.cmd("G53 G0 W0Z0")
        self.client.close_socket()
        print("Finished")


class ProbeJob(QGroupBox):
    OnDataChanged = Signal(np.ndarray)
    OnStartJob = Signal(dict)

    def __init__(self, client):
        QGroupBox.__init__(self)
        self.setTitle("Probe Job")

        self.data = np.zeros((5, 5), dtype=np.float64)

        self.driver = ProbeDriver(client)
        self.driver_thread = QThread()
        self.driver.moveToThread(self.driver_thread)
        self.driver_thread.start()

        form = QFormLayout()
        self.setLayout(form)

        self.sample_X_line = QDoubleSpinBox()
        self.sample_Y_line = QDoubleSpinBox()
        self.sample_distance = QDoubleSpinBox()
        self.probe_height = QDoubleSpinBox()

        # Set some values
        self.sample_X_line.setValue(70)
        self.sample_Y_line.setValue(70)
        self.sample_distance.setValue(10)
        self.probe_height.setValue(5)

        form.addRow("Sample X Length", self.sample_X_line)
        form.addRow("Sample Y Length", self.sample_Y_line)
        form.addRow("Sample Distance", self.sample_distance)
        form.addRow("Probe Lift Height", self.probe_height)

        # Update the driver
        self.OnStartJob.connect(self.driver.loop)

        # update the GUI
        self.sample_X_line.valueChanged.connect(self.update_data_shape)
        self.sample_Y_line.valueChanged.connect(self.update_data_shape)
        self.sample_distance.valueChanged.connect(self.update_data_shape)
        self.probe_height.valueChanged.connect(self.update_data_shape)
        self.driver.OnSampleReceived.connect(self.sample_in)

        self.update_data_shape()

    def start_job(self):
        sample_X_line = self.sample_X_line.value()
        sample_X_line = self.sample_X_line.value()
        dist = self.sample_distance.value()
        lift_height = self.probe_height.value()

        x_holes = int(sample_X_line / dist)
        y_holes = int(sample_X_line / dist)

        self.data = np.zeros((x_holes, y_holes), dtype=np.float64)

        self.OnStartJob.emit(
            {
                "x_holes": x_holes,
                "y_holes": y_holes,
                "dist": dist,
                "lift_height": lift_height,
            }
        )

    def sample_in(self, sample: list[int | int | float]) -> None:
        print(f"Sample into the job GUI is: {sample}")
        x = sample[0]
        y = sample[1]
        val = sample[2]

        self.data[y][x] = val
        self.OnDataChanged.emit(self.data)

    def update_data_shape(self) -> None:
        x = self.sample_X_line.value()
        y = self.sample_Y_line.value()
        d = self.sample_distance.value()

        if d == 0:
            return

        x_shape = int(x / d)
        y_shape = int(y / d)

        self.data = np.zeros((x_shape, y_shape), dtype=np.float64)
        print("emitting data")
        self.OnDataChanged.emit(self.data)
        print("data emitted")

    def closeEvent(self, event: QCloseEvent) -> None:
        print("inside close event for test job")
        self.driver_thread.requestInterruption()

        return super().closeEvent(event)
