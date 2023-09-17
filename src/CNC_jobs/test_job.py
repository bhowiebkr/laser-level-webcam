from src.CNC_jobs.common import LinuxDriver
from PySide6.QtCore import QThread
import sys

IN_LINUXCNC = False
if sys.platform == "linux":
    IN_LINUXCNC = True
    import linuxcnc


class TestJob(LinuxDriver):
    def init(self):
        super(TestJob).__init__()

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

    def set_sample_x_length(self, length: float) -> None:
        self.sample_x_length = length

    def set_sample_y_length(self, length: float) -> None:
        self.sample_y_length = length

    def set_sample_distance(self, distance: float) -> None:
        self.sample_distance = distance