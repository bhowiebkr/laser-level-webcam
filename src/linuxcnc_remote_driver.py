from __future__ import annotations
import linuxcnc

s = linuxcnc.stat()
c = linuxcnc.command()


def ready() -> None:
    s.poll()
    return (
        not s.estop
        and s.enabled
        and (s.homed.count(1) == s.joints)
        and (s.interp_state == linuxcnc.INTERP_IDLE)
    )


def run_and_wait(func: function) -> function:
    def dec_func(*args, **kwargs):
        func(*args, **kwargs)
        c.wait_complete()  # wait until mode switch executed

    return dec_func


@run_and_wait
def cmd(cmd: str) -> None:
    c.mdi(cmd)
    print(f"Sent: {cmd}")
    run_and_wait(str)


def main() -> None:
    if ready():
        c.mode(linuxcnc.MODE_MDI)
        c.wait_complete()  # wait until mode switch executed
        cmd("G64")  # Path blending best possible speed

        radius = 2  # milling radius
        height = 4  # safe height
        dist = 10  # hole distance

        x_holes = 5
        y_holes = 3
        feed = 5000

        for y in range(y_holes):
            for x in range(x_holes):
                # Move down
                cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")
                cmd(f"G0 X{x*dist} Y{y*dist} Z0")

                # Circle
                cmd(f"G0 X{x*dist -radius} Y{y*dist} Z0")
                cmd(f"G02 X{x*dist -radius} Y{y*dist} I{radius} J0 F{feed}")
                cmd(f"G0 X{x*dist } Y{y*dist} Z0")

                # Move up
                cmd(f"G0 X{x*dist} Y{y*dist} Z{height}")


if __name__ == "__main__":
    main()
