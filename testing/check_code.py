from __future__ import annotations

import os


def run() -> None:
    os.system("pre-commit run --all-files")


if __name__ == "__main__":
    run()
