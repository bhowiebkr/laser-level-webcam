from __future__ import annotations

import sys
from typing import Any
from typing import Dict

import numpy as np
from PySide6.QtCore import QThread
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGroupBox

from src.client import Client
from src.CNC_jobs.common import LinuxDriver

IN_LINUXCNC = False
if sys.platform == "linux":
    IN_LINUXCNC = True
    import linuxcnc


class ProbeAndMachineDriver(LinuxDriver):
    def init(self, client: Client) -> None:
        super().__init__()

        self.client = client

    def loop(self, params: Dict[str, Any]) -> None:
        x_holes = params["x_holes"]
        y_holes = params["y_holes"]
        dist = params["dist"]

        print("Starting LinuxCNC job")
        self.c.mode(linuxcnc.MODE_MDI)  # type: ignore
        self.c.wait_complete()  # wait until mode switch executed
        self.cmd("G64")  # Path blending best possible speed

        radius = 2  # milling radius
        height = 4  # safe height

        feed = 5000

        print(self.client.send_recieve("ZERO"))

        for y in range(y_holes):
            for x in range(x_holes):
                if QThread.currentThread().isInterruptionRequested():
                    print("Job Stopped")
                    return

                print(f"index x: {x} y: {y}")

                # Move down
                sample = float(self.client.send_recieve("TAKE_SAMPLE").split(" ")[1])
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

            print("not ready")


class ProbeAndMachineJob(QGroupBox):  # type: ignore
    OnDataChanged = Signal(np.ndarray)
    OnStartJob = Signal(dict)

    def __init__(self, client: Client) -> None:
        QGroupBox.__init__(self)
        self.setTitle("Probe And Machine Job")

        self.driver = ProbeAndMachineDriver()
        self.driver_thread = QThread()
        self.driver.moveToThread(self.driver_thread)
        self.driver_thread.start()

        form = QFormLayout()
        self.setLayout(form)

        self.sample_X_line = QDoubleSpinBox()
        self.sample_Y_line = QDoubleSpinBox()
        self.sample_distance = QDoubleSpinBox()

        # Set some values
        self.sample_X_line.setValue(10)
        self.sample_Y_line.setValue(10)
        self.sample_distance.setValue(1)

        form.addRow("Sample X Length", self.sample_X_line)
        form.addRow("Sample Y Length", self.sample_Y_line)
        form.addRow("Sample Distance", self.sample_distance)

        # Update the driver
        self.OnStartJob.connect(self.driver.loop)

        # update the GUI
        self.sample_X_line.valueChanged.connect(self.update_data_shape)
        self.sample_Y_line.valueChanged.connect(self.update_data_shape)
        self.sample_distance.valueChanged.connect(self.update_data_shape)

        self.update_data_shape()

    def start_job(self) -> None:
        sample_X_line = self.sample_X_line.value()
        sample_X_line = self.sample_X_line.value()
        dist = self.sample_distance.value()

        x_holes = int(sample_X_line / dist)
        y_holes = int(sample_X_line / dist)

        self.OnStartJob.emit({"x_holes": x_holes, "y_holes": y_holes, "dist": dist})

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

    def closeEvent(self, event: QCloseEvent) -> QCloseEvent:
        print("inside close event for test job")
        self.driver_thread.requestInterruption()

        return super().closeEvent(event)
