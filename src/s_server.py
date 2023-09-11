from __future__ import annotations

import socket
import threading

from PySide6.QtCore import QObject
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QTextEdit
from PySide6.QtWidgets import QVBoxLayout


class MessageServer(QObject):  # type: ignore
    message_received = Signal(str)
    take_sample = Signal()

    def __init__(self, host: str, port: int) -> None:
        super().__init__()
        self.host = host
        self.port = port
        self.socket_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.running = False

    def start_server(self) -> None:
        self.socket_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.socket_server.bind((self.host, self.port))
        self.socket_server.listen(1)
        self.running = True

        while self.running:
            print("running")
            try:
                client_socket, client_address = self.socket_server.accept()
                data = client_socket.recv(1024).decode("utf-8")
                self.message_received.emit(data)

                if data == "TAKE_SAMPLE":
                    self.take_sample.emit()
                client_socket.close()
            except Exception as e:
                print(f"Error: {str(e)}")

    def stop_server(self) -> None:
        self.running = False
        if self.socket_server:
            try:
                self.socket_server.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.socket_server.close()


class SocketWindow(QDialog):  # type: ignore
    def __init__(self, parent: QMainWindow):
        super().__init__(parent)

        self.setWindowTitle("Socket Server GUI")
        self.setGeometry(100, 100, 400, 300)

        # self.setAttribute(Qt.WA_DeleteOnClose)  # This will break everything if we use it. so we don't

        layout = QVBoxLayout()

        self.text_edit = QTextEdit()
        layout.addWidget(self.text_edit)

        self.start_button = QPushButton("Start Server")
        layout.addWidget(self.start_button)
        self.start_button.clicked.connect(self.start_server)

        self.stop_button = QPushButton("Stop Server")
        layout.addWidget(self.stop_button)
        self.stop_button.clicked.connect(self.stop_server)
        self.stop_button.setEnabled(False)

        self.setLayout(layout)

        # Setup but don't start. That way we can link up signals to the rest of the GUI
        self.message_server = MessageServer(socket.gethostname(), 1234)
        self.server_thread = threading.Thread(target=self.message_server.start_server)

    def start_server(self) -> None:
        if not self.server_thread or not self.server_thread.is_alive():
            self.server_thread.start()
            self.start_button.setEnabled(False)
            self.stop_button.setEnabled(True)
            print("server started")

    def stop_server(self) -> None:
        if self.server_thread and self.server_thread.is_alive():
            self.message_server.stop_server()
            self.server_thread.join()
            self.start_button.setEnabled(True)
            self.stop_button.setEnabled(False)
            print("server stopped")

    def update_text_edit(self, message: str) -> None:
        self.text_edit.append(message)

    def closeEvent(self, event: QCloseEvent) -> None:
        self.message_server.stop_server()
        self.start_button.setEnabled(True)
        self.stop_button.setEnabled(False)
        super().closeEvent(event)


"""
from __future__ import annotations

import pickle
import socket

HEADER_SIZE = 10

s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind((socket.gethostname(), 1234))
s.listen(5)

while True:
    client_socket, address = s.accept()
    print(f"Connection from {address} has been established!")

    d = {1: "Hey", 2: "There"}
    msg = pickle.dumps(d)
    msg = bytes(f"{len(msg):<{HEADER_SIZE}}", "utf-8") + msg

    client_socket.send(msg)
"""
