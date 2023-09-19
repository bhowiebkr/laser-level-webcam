from __future__ import annotations

import random
import socket
from typing import Any
from typing import Dict

from PySide6.QtCore import QObject

SKIP_CONNECTION = False


class Client(QObject):  # type: ignore
    server = socket.socket()  # Create a socket object

    def __init__(self) -> None:
        super().__init__()

        self.port = 0
        self.ip = ""

    def connect_socket(self, params: Dict[str, Any]) -> bool:
        self.port = params["port"]
        self.ip = params["ip"]
        print(f"Connecting.. IP: {self.ip} Port: {self.port}")
        connected = False
        num_fails = 0
        while not connected:
            if num_fails >= 3:
                break
            try:
                self.server.connect((self.ip, self.port))
                connected = True
                print("Connected.")
                return True
            except Exception as e:
                print(
                    f"Failed to connect with the following: {e}. Using IP {self.ip}, Port: \
{self.port}, Try: {num_fails+1}/3"
                )  # print why and try again
                num_fails += 1
                continue
        print("...Stopped")
        return False

    def send_recieve(self, cmd: str) -> str:
        if SKIP_CONNECTION:
            recv = f"Fake_Data: {random.uniform(-1.0, 1.0)}"
        else:
            self.server.send(cmd.encode("utf-8"))
            recv = self.server.recv(1024).decode("utf-8")
        return recv

    def set_IP(self, ip: str) -> None:
        self.ip = ip
        print("Set IP to:", self.ip)

    def set_port(self, port: int) -> None:
        self.port = port
        print("Set port to:", self.port)

    def close_socket(self) -> None:
        self.server.close()
