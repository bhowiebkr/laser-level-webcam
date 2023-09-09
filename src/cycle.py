from __future__ import annotations

from typing import Any

from PySide6.QtCore import QTimer
from PySide6.QtCore import Signal
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QSpinBox


class CyclicMeasurementSetupWindow(QDialog):  # type: ignore
    """
    Represents a non-modal dialog that allows to start/stop cyclic measurements and adjust the interval
    between measurements

    Also handles starting/stopping the timer. Parent class is expected to actually perform the
    measurements when onMeasurementTrigger signal is emitted.
    """

    cycle_time_sb: QSpinBox
    cycle_timer: QTimer
    pb_start: QPushButton
    pb_stop: QPushButton

    onMeasurementTrigger = Signal()

    def __init__(self, parent: Any) -> None:
        super().__init__(parent)
        self.setWindowTitle("Cyclic measurement setup")
        self.setModal(False)
        fl = QFormLayout(self)
        self.cycle_time_sb = QSpinBox(self)
        self.cycle_time_sb.setValue(60)
        self.cycle_time_sb.setMinimum(10)
        self.cycle_time_sb.setMaximum(3600)
        fl.addRow("Cycle time (s)", self.cycle_time_sb)

        self.pb_start = QPushButton(self)
        self.pb_start.setText("Start")
        self.pb_start.released.connect(self.start_cycle)

        self.pb_stop = QPushButton(self)
        self.pb_stop.setText("Stop")
        self.pb_stop.setEnabled(False)
        self.pb_stop.released.connect(self.stop_cycle)
        fl.addRow(self.pb_start, self.pb_stop)
        self.cycle_timer = QTimer(self)
        self.cycle_timer.timeout.connect(self.onMeasurementTrigger)

    def start_cycle(self) -> None:
        self.cycle_timer.setInterval(1000 * self.cycle_time_sb.value())
        self.cycle_timer.start()
        self.pb_start.setEnabled(False)
        self.pb_stop.setEnabled(True)

    def stop_cycle(self) -> None:
        self.cycle_timer.stop()
        self.pb_stop.setEnabled(False)
        self.pb_start.setEnabled(True)
