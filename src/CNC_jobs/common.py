from __future__ import annotations

import sys
import time

from PySide6.QtCore import QObject
from PySide6.QtCore import Signal


IN_LINUXCNC = False
if sys.platform == "linux":
    print("In LinuxCNC")
    IN_LINUXCNC = True
    import linuxcnc

DEV_MODE = False


class LinuxDriver(QObject):  # type: ignore
    OnSampleReceived = Signal(list)

    def __init__(self) -> None:
        super().__init__()

        if IN_LINUXCNC:
            self.s = linuxcnc.stat()  # type: ignore
            self.c = linuxcnc.command()  # type: ignore

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
            time.sleep(0.1)
