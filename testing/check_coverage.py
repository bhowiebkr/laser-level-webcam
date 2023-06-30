from __future__ import annotations

import os


def run() -> None:
    os.chdir("../")

    os.system("coverage run -m pytest tests")

    if not os.path.exists("coverage"):
        os.makedirs("coverage")

    os.system("coverage-lcov --output_file_path coverage/lcov.info")


if __name__ == "__main__":
    run()
