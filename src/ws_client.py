from __future__ import annotations

import asyncio

import websockets


async def message() -> None:
    # Use the local IP of the machine running the laser-webcam tool
    async with websockets.connect("ws://192.168.1.140:1234") as socket:
        msg = input("what do you want to send: ")
        await socket.send(msg)
        print(await socket.recv())


asyncio.get_event_loop().run_until_complete(message())
