from __future__ import annotations

import sys

import numpy as np
from PySide6.QtCore import QObject
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtNetwork import QHostAddress
from PySide6.QtNetwork import QTcpSocket
from PySide6.QtWidgets import QApplication
from PySide6.QtWidgets import QDoubleSpinBox
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGroupBox

IN_LINUXCNC = False
if sys.platform == "linux":
    print("In LinuxCNC")
    IN_LINUXCNC = True
    import linuxcnc

DEV_MODE = False


class ProbeDriver(QObject):  # type: ignore
    sample_received = Signal(list)
    connection_made = Signal()
    sample_recieved = Signal(list)

    def __init__(self) -> None:
        super().__init__()
        self.socket = QTcpSocket(self)
        self.socket.connected.connect(self.connected)
        self.socket.readyRead.connect(self.receive_message)
        self.message = ""

        if IN_LINUXCNC:
            self.s = linuxcnc.stat()  # type: ignore
            self.c = linuxcnc.command()  # type: ignore

        self.x_holes = 0
        self.y_holes = 0
        self.dist = 0.0
        self.lift = 0.0

    def loop(self) -> None:
        print("Starting LinuxCNC job")
        if IN_LINUXCNC:
            self.c.mode(linuxcnc.MODE_MDI)  # type: ignore
            self.c.wait_complete()  # wait until mode switch executed
        self.cmd("G64")  # Path blending best possible speed

        # Move the W axis back to machine coord zero
        self.cmd("G53 G0 W0Z0")

        # Move to the start position
        self.cmd("G54 G0 X0Y0")

        # Move W to lift height
        self.cmd(f"G0 W{self.lift}")

        # Move down to W zero for setting zero
        self.cmd("G1 F2000 W0")

        # Zero out the webcam sensor
        self.send_message("ZERO")

        # Move W to lift height (Starting position)
        self.cmd(f"G0 W{self.lift}")

        print("About to start loop")

        for y in range(self.y_holes):
            for x in range(self.x_holes):
                # Goto next sample location
                self.cmd(f"G0 X{x*self.dist} Y{y*self.dist}")

                # Move down and take a sample
                self.cmd("G1 F2000 W0")

                # Send the "TAKE_SAMPLE" message and wait for the response
                self.send_message("TAKE_SAMPLE")

                while not self.message:
                    QApplication.processEvents()  # Process events to prevent UI from freezing

                sample = self.message
                self.message = ""
                print(f"I got: {sample}")

                self.sample_recieved.emit([x, y, sample * 1000])  # convert sample mm to um

                # Move up
                self.cmd(f"G0 W{self.lift}")

                print("iteration done")

            print("not ready")

        # Move the W axis back to machine coord zero
        self.cmd("G53 G0 W0Z0")
        print("Finished")

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

    def cmd(self, cmd: str) -> None:
        if IN_LINUXCNC:
            self.c.mdi(cmd)
            print(f"Sent: {cmd}")
            self.c.wait_complete()  # wait until mode switch executed

        else:
            print(f"Sent: {cmd}")
            # time.sleep(0.1)

    def connect_to_host(self, ip: str, port: str) -> None:
        print(f"ip is {ip}")
        print(f"port is {port}")
        print(f"socket is {self.socket}")
        if ip and port:
            self.socket.connectToHost(QHostAddress(ip), int(port))
        else:
            print("must have a valid ip and port")

    def connected(self) -> None:
        self.connection_made.emit()
        # self.text_edit.append("Connected to server.")

    def send_message(self, message: str) -> None:
        print(f"Sent a message to the server: {message}")
        self.socket.write(f"{message}\n".encode())  # Ensure a newline after the message

    def receive_message(self) -> None:
        self.message = self.socket.readLine().data().decode()  # Read line instead of all data
        print(f"recieved messge: {self.message}")


class ProbeJob(QGroupBox):  # type: ignore
    data_changed = Signal(np.ndarray)
    start_job = Signal()

    def __init__(self) -> None:
        QGroupBox.__init__(self)
        self.setTitle("Probe Job")

        self.data = np.zeros((5, 5), dtype=np.float64)

        self.driver = ProbeDriver()

        form = QFormLayout()
        self.setLayout(form)

        self.sample_X_line = QDoubleSpinBox()
        self.sample_Y_line = QDoubleSpinBox()
        self.sample_distance = QDoubleSpinBox()
        self.probe_height = QDoubleSpinBox()

        # Set some values
        self.sample_X_line.setValue(70)
        self.sample_Y_line.setValue(70)
        self.sample_distance.setValue(15)
        self.probe_height.setValue(10)

        form.addRow("Sample X Length", self.sample_X_line)
        form.addRow("Sample Y Length", self.sample_Y_line)
        form.addRow("Sample Distance", self.sample_distance)
        form.addRow("Probe Lift Height", self.probe_height)

        # update the GUI
        self.sample_X_line.valueChanged.connect(self.update_data_shape)
        self.sample_Y_line.valueChanged.connect(self.update_data_shape)
        self.sample_distance.valueChanged.connect(self.update_data_shape)
        self.probe_height.valueChanged.connect(self.update_data_shape)
        self.driver.sample_received.connect(self.sample_in)

        self.start_job.connect(self.driver.loop)

        self.update_data_shape()

    def start_driver(self) -> None:
        self.driver.x_holes = int(self.sample_X_line.value() / self.sample_distance.value())
        self.driver.y_holes = int(self.sample_Y_line.value() / self.sample_distance.value())
        self.driver.dist = self.sample_distance.value()
        self.driver.lift = self.probe_height.value()

        self.data = np.zeros((self.driver.x_holes, self.driver.y_holes), dtype=np.float64)

        self.start_job.emit()

    def sample_in(self, sample: list[int | int | float]) -> None:
        print(f"Sample into the job GUI is: {sample}")
        x = sample[0]
        y = sample[1]
        val = sample[2]

        self.data[y][x] = val
        self.data_changed.emit(self.data)

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
        self.data_changed.emit(self.data)
        print("data emitted")

    def closeEvent(self, event: QCloseEvent) -> QCloseEvent:
        print("inside close event for test job")

        return super().closeEvent(event)
