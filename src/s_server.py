from __future__ import annotations

import socket
import threading
import time

from PySide6.QtCore import QObject
from PySide6.QtCore import Signal
from PySide6.QtGui import QCloseEvent
from PySide6.QtWidgets import QDialog
from PySide6.QtWidgets import QMainWindow
from PySide6.QtWidgets import QPushButton
from PySide6.QtWidgets import QTextEdit
from PySide6.QtWidgets import QVBoxLayout

HEADER_LENGTH = 10
IP = "192.168.1.232"

print(f"IP is: {IP}")
PORT = 1234


class MessageServer(QObject):  # type: ignore
    message_received = Signal(str)
    take_sample = Signal()
    zero = Signal()

    def __init__(self, host: str, port: int) -> None:
        super().__init__()

        self.socket_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        # Allow to reconnect:
        self.socket_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.running = False
        self.out_message = ""

    def start_server(self) -> None:
        self.socket_server.bind((IP, PORT))
        self.socket_server.listen()
        self.running = True

        client_socket, client_address = self.socket_server.accept()

        while self.running:
            try:
                print("Start of loop")

                msg = client_socket.recv(1024).decode("utf-8")
                print(f"Got msg: {msg}")
                self.message_received.emit(msg)  # back to the GUI

                if msg == "TAKE_SAMPLE":
                    self.take_sample.emit()
                if msg == "ZERO":
                    self.zero.emit()

                # Waiting for the tool to do something
                while not self.out_message and self.running:
                    time.sleep(0.1)

                client_socket.sendall(self.out_message.encode("utf-8"))
                self.out_message = ""  # reset the message
                print("Message Sent. restarting loop")

            except Exception as e:
                print(f"Error: {str(e)}")

    def stop_server(self) -> None:
        self.running = False
        time.sleep(0.1)
        self.socket_server.shutdown(socket.SHUT_RDWR)
        self.socket_server.close()

    def send_message(self, msg: str) -> None:
        print(f"Sending message: {msg}")
        self.out_message = msg

    def __del__(self) -> None:
        # print("Closing server socket:", self.sock)
        self.socket_server.shutdown(socket.SHUT_RDWR)
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
