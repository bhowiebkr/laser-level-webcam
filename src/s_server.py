from __future__ import annotations

import socket

from PySide6.QtCore import Signal
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGroupBox
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QLineEdit
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QTextEdit
from PySide6.QtWidgets import QVBoxLayout
from PySide6.QtNetwork import QTcpServer, QHostAddress


class SocketWindow(QDialog):  # type: ignore
    message_received = Signal(str)
    take_sample = Signal()
    zero = Signal()

    def __init__(self, parent: QMainWindow):
        super().__init__(parent)

        self.setWindowTitle("Socket Server GUI")
        self.setGeometry(100, 100, 400, 300)

        self.server = QTcpServer(self)
        self.client_connection = None

        layout = QVBoxLayout()

        self.history = QTextEdit()

        self.ip_line = QLineEdit()
        self.port_line = QLineEdit()
        form = QFormLayout()
        lookup_ip_btn = QPushButton("lookup")
        lookup_ip_btn.clicked.connect(self.lookup_ip)
        ip_layout = QHBoxLayout()
        ip_layout.addWidget(self.ip_line)
        ip_layout.addWidget(lookup_ip_btn)

        cmd_history_box = QGroupBox("Command History")
        history_layout = QVBoxLayout()
        history_layout.addWidget(self.history)
        history_layout.setContentsMargins(0, 5, 0, 0)
        cmd_history_box.setLayout(history_layout)
        layout.addWidget(cmd_history_box)

        self.start_button = QPushButton("Start Server")
        form.addRow("IP Address", ip_layout)
        form.addRow("Port", self.port_line)

        setup_box = QGroupBox("Setup")
        setup_layout = QVBoxLayout()
        setup_layout.addLayout(form)
        setup_layout.addWidget(self.start_button)
        setup_layout.setContentsMargins(0, 5, 0, 0)
        setup_box.setLayout(setup_layout)
        layout.addWidget(setup_box)

        self.start_button.clicked.connect(self.start_server)

        self.setLayout(layout)

    def lookup_ip(self) -> None:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.connect(("8.8.8.8", 80))
        address = sock.getsockname()[0]
        sock.close()
        self.ip_line.setText(address)

    def newConnection(self):
        self.client_connection = self.server.nextPendingConnection()
        self.client_connection.readyRead.connect(self.receive_message)

    def start_server(self) -> None:
        print("Start server")
        ip = str(self.ip_line.text())
        port = int(self.port_line.text())
        self.server.listen(QHostAddress(ip), port)
        self.server.newConnection.connect(self.newConnection)
        self.update_text_edit(f"Server Started at {ip}:{port}")

    def receive_message(self):
        message = self.client_connection.readAll().data().decode()
        if message == "TAKE_SAMPLE":
            self.take_sample.emit()
        elif message == "ZERO":
            self.zero.emit()
        else:
            self.send_message(f"Invalid command: {message}")

        self.update_text_edit(f"Received: {message}")

    def send_message(self, message):
        self.client_connection.write(f"{message}".encode())

    def update_text_edit(self, message: str) -> None:
        self.history.append(message)
