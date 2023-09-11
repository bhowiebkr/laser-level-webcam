#!/usr/bin/python           # This is client.py file
from __future__ import annotations

import socket  # Import socket module

s = socket.socket()  # Create a socket object

host = socket.gethostname()  # Get local machine name
port = 1234  # Reserve a port for your service.

s.connect((host, port))
while True:
    a = input("Enter Command (ZERO, TAKE_SAMPLE):")
    s.send(bytes(a, "ascii"))
    recv = s.recv(1024).decode("utf-8")
    print(recv)

s.close()
