from __future__ import annotations

import socket
import threading
import time

from PySide6.QtCore import QObject
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QFormLayout
from PySide6.QtWidgets import QGroupBox
from PySide6.QtWidgets import QHBoxLayout
from PySide6.QtWidgets import QLineEdit
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QTextEdit
from PySide6.QtWidgets import QVBoxLayout


class MessageServer(QObject):  # type: ignore
    message_received = Signal(str)
    take_sample = Signal()
    zero = Signal()

    def __init__(self, parent: QObject = QObject()) -> None:
        super().__init__(parent)
        self.ip = ""
        self.port = 0

        self.socket_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        # Allow to reconnect:
        self.socket_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.running = False
        self.out_message = ""

    def start_server(self) -> None:
        print("Starting server")
        self.socket_server.bind((self.ip, self.port))
        self.socket_server.listen()
        self.running = True

        client_socket, _ = self.socket_server.accept()

        while self.running:
            try:
                print("Start of loop")

                msg = client_socket.recv(1024).decode("utf-8")
                print(f"Got msg: {msg}")
                self.message_received.emit(msg)  # back to the GUI

                if msg == "TAKE_SAMPLE":
                    self.take_sample.emit()
                elif msg == "ZERO":
                    self.zero.emit()
                else:
                    self.send_message("Invalid command")

                # Waiting for the tool to do something
                while not self.out_message and self.running:
                    time.sleep(0.1)

                client_socket.sendall(self.out_message.encode("utf-8"))
                self.out_message = ""  # reset the message
                print("Message Sent. restarting loop")

            except Exception as e:
                print(f"Error: {str(e)}")
                break

    def stop_server(self) -> None:
        self.running = False
        self.socket_server.close()

    def send_message(self, msg: str) -> None:
        print(f"Sending message: {msg}")
        self.parent().update_text_edit(f"-> {msg}")
        self.out_message = msg


class SocketWindow(QDialog):  # type: ignore
    def __init__(self, parent: QMainWindow):
        super().__init__(parent)

        self.setWindowTitle("Socket Server GUI")
        self.setGeometry(100, 100, 400, 300)

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

        # Setup but don't start. That way we can link up signals to the rest of the GUI
        self.message_server = MessageServer(self)
        self.server_thread = threading.Thread(target=self.message_server.start_server)

    def lookup_ip(self) -> None:
        hostname = socket.gethostname()
        ip_address = socket.gethostbyname(hostname)
        self.ip_line.setText(ip_address)

    def start_server(self) -> None:
        self.message_server.ip = str(self.ip_line.text())
        self.message_server.port = int(self.port_line.text())
        self.server_thread.start()
        print("server started")

    def stop_server(self) -> None:
        self.message_server.stop_server()
        if self.server_thread.is_alive():
            self.server_thread.join()
        print("server stopped")

    def update_text_edit(self, message: str) -> None:
        self.history.append(message)

    def closeEvent(self, event: QCloseEvent) -> None:
        self.stop_server()
        super().closeEvent(event)
