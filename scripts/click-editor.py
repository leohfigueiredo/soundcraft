#!/usr/bin/env python3
"""Click inside a plugin editor window, to test that its GUI actually takes input.

Usage: scripts/click-editor.py X ID [--at X,Y ...] [--drag]

Sends real input events through XTest (the X server's own event injection), so the plugin sees
them exactly like a user's mouse. Each point is relative to the window's top-left corner; without
`--at` it clicks a grid across the window.
"""

import ctypes
import ctypes.util
import sys
import time


class XWindowAttributes(ctypes.Structure):
    _fields_ = [
        ("x", ctypes.c_int),
        ("y", ctypes.c_int),
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("border_width", ctypes.c_int),
        ("depth", ctypes.c_int),
        ("visual", ctypes.c_void_p),
        ("root", ctypes.c_ulong),
        ("class_", ctypes.c_int),
        ("bit_gravity", ctypes.c_int),
        ("win_gravity", ctypes.c_int),
        ("backing_store", ctypes.c_int),
        ("backing_planes", ctypes.c_ulong),
        ("backing_pixel", ctypes.c_ulong),
        ("save_under", ctypes.c_int),
        ("colormap", ctypes.c_ulong),
        ("map_installed", ctypes.c_int),
        ("map_state", ctypes.c_int),
        ("all_event_masks", ctypes.c_long),
        ("your_event_mask", ctypes.c_long),
        ("do_not_propagate_mask", ctypes.c_long),
        ("override_redirect", ctypes.c_int),
        ("screen", ctypes.c_void_p),
    ]


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    display_name, window = sys.argv[1], int(sys.argv[2], 0)
    points = []
    drag = "--drag" in sys.argv
    args = sys.argv[3:]
    for i, arg in enumerate(args):
        if arg == "--at" and i + 1 < len(args):
            x, y = args[i + 1].split(",")
            points.append((int(x), int(y)))

    x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
    xtst = ctypes.CDLL(ctypes.util.find_library("Xtst"))
    x11.XOpenDisplay.restype = ctypes.c_void_p
    display = x11.XOpenDisplay(display_name.encode())
    if not display:
        sys.exit("cannot open the display")

    attrs = XWindowAttributes()
    if not x11.XGetWindowAttributes(ctypes.c_void_p(display), ctypes.c_ulong(window), ctypes.byref(attrs)):
        sys.exit(f"cannot read window {window:#x}")

    # Translate to root coordinates: XTest injects into the root window's coordinate space.
    root_x = ctypes.c_int(0)
    root_y = ctypes.c_int(0)
    child = ctypes.c_ulong(0)
    x11.XTranslateCoordinates(
        ctypes.c_void_p(display),
        ctypes.c_ulong(window),
        ctypes.c_ulong(attrs.root),
        0,
        0,
        ctypes.byref(root_x),
        ctypes.byref(root_y),
        ctypes.byref(child),
    )
    print(f"window {window:#x}: {attrs.width}x{attrs.height} at root {root_x.value},{root_y.value}")
    if not points:
        points = [(attrs.width // 4, attrs.height // 3), (attrs.width // 2, attrs.height // 2), (attrs.width // 3, attrs.height * 2 // 3)]

    def move(x, y):
        xtst.XTestFakeMotionEvent(ctypes.c_void_p(display), -1, root_x.value + x, root_y.value + y, 0)
        x11.XFlush(ctypes.c_void_p(display))
        time.sleep(0.15)

    for (x, y) in points:
        move(x, y)
        xtst.XTestFakeButtonEvent(ctypes.c_void_p(display), 1, True, 0)
        x11.XFlush(ctypes.c_void_p(display))
        time.sleep(0.2)
        if drag:
            for step in range(1, 8):
                move(x, y - step * 3)
        xtst.XTestFakeButtonEvent(ctypes.c_void_p(display), 1, False, 0)
        x11.XFlush(ctypes.c_void_p(display))
        time.sleep(0.3)
        print(f"clicked {x},{y}")
    x11.XCloseDisplay(ctypes.c_void_p(display))


if __name__ == "__main__":
    main()
