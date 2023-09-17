from PySide6.QtCore import Signal
from PySide6.QtCore import QThread
from PySide6.QtCore import QObject
import sys
from src.client_server import ClientServer

IN_LINUXCNC = False
if sys.platform == "linux":
    print('In LinuxCNC')
    IN_LINUXCNC = True
    import linuxcnc

DEV_MODE = False

class LinuxDriver(QObject):  # type: ignore
    OnSampleReceived = Signal(list)

    def __init__(self) -> None:
        super().__init__()
        self.server = ClientServer()

        if IN_LINUXCNC:
            self.s = linuxcnc.stat()  # type: ignore
            self.c = linuxcnc.command()  # type: ignore

    def loop(self) -> None:
        # Override this
        pass
        

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
        if DEV_MODE:
            print(f"Sent: {cmd}")
            time.sleep(0.1)
        else:
            self.c.mdi(cmd)
            print(f"Sent: {cmd}")
            self.c.wait_complete()  # wait until mode switch executed

    def start(self) -> None:
        print("Running")
        self.loop()
        print("Finished")