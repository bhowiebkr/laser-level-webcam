from __future__ import annotations

import asyncio
import socket
from typing import Any

import websockets


async def response(websocket: Any, path: Any) -> None:
    print([websocket])
    message = await websocket.recv()
    print(f"We got the emssage from the client: {message}")
    await websocket.send("I can confirm I got your message!")


# Get the local IP address so we can map to it
port = 1234
local_ip = socket.gethostbyname(socket.gethostname())
print(f"Server IP address: {local_ip}:{port}")
# Start the websocket server
start_server = websockets.serve(response, local_ip, port)
asyncio.get_event_loop().run_until_complete(start_server)
asyncio.get_event_loop().run_forever()
