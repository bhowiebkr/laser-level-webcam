from __future__ import annotations

import asyncio
import socket
from typing import Any

import websockets
from PySide6.QtCore import QThread
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QLineEdit
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QVBoxLayout


async def response(websocket: Any, path: Any) -> None:
    while True:
        try:
            print([websocket])
            message = await websocket.recv()
            print(f"We got the emssage from the client: {message}")
            await websocket.send("I can confirm I got your message!")
        except websockets.exceptions.ConnectionClosed:
            break


class WebSocketThread(QThread):  # type: ignore
    def run(self, local_ip: str, port: str) -> None:
        server = websockets.serve(response, local_ip, port)

        asyncio.get_event_loop().run_until_complete(server)
        asyncio.get_event_loop().run_forever()


class WebsocketWindow(QDialog):  # type: ignore
    """
    Represents a non-modal dialog that allows to start/stop cyclic measurements and adjust the interval
    between measurements

    Also handles starting/stopping the timer. Parent class is expected to actually perform the
    measurements when onMeasurementTrigger signal is emitted.
    """

    def __init__(self, parent: Any) -> None:
        super().__init__(parent)
        self.setWindowTitle("Websocket Server")
        self.resize(500, 300)

        self.server = WebSocketThread()
        self.server.start()

        # Layouts
        main_layout = QVBoxLayout(self)
        form_layout = QFormLayout()
        btn_layout = QHBoxLayout()

        # Widgets
        self.ip_address_line = QLineEdit()
        start_btn = QPushButton("Start")
        test_btn = QPushButton("Test")
        stop_btn = QPushButton("Stop")

        for btn in [start_btn, stop_btn, test_btn]:
            btn.setFixedHeight(30)

        # Add Layouts
        main_layout.addLayout(form_layout)
        main_layout.addStretch()
        main_layout.addLayout(btn_layout)

        btn_layout.addWidget(start_btn)
        btn_layout.addWidget(test_btn)
        btn_layout.addWidget(stop_btn)

        form_layout.addRow("IP Address", self.ip_address_line)

        # Logic
        self.init_GUI()
        start_btn.clicked.connect(self.start_server)

    def init_GUI(self) -> None:
        port = 1234
        local_ip = socket.gethostbyname(socket.gethostname())
        self.ip_address_line.setText(f"{local_ip}:{port}")

    def start_server(self) -> None:
        local_ip = self.ip_address_line.text().split(":")[0]
        port = self.ip_address_line.text().split(":")[1]

        self.server.run(local_ip, port)
