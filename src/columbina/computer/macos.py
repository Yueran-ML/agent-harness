"""The desktop of macOS, reached through Core Graphics and two programs that the system brings.

The pointer and the keyboard are moved by events of Core Graphics, which go
where those of the devices go. macOS takes them only from a program that the
person at the machine allowed under Accessibility, and shows the windows of
others only to one that was allowed under Screen Recording. Both are asked
of the program that started this one, a terminal or the application of
Columbina, and are given in the settings of the system. Where one is missing
the tool says which.

The picture of the screen is taken by ``screencapture`` and brought to its
size by ``sips``.

Positions are in the points of the main display, which is what the system
counts in: on a display that draws a point with four pixels they are half as
many as the pixels each way.

A key that has a name, a letter or a digit is pressed by the code that it
has on a keyboard of the layout of the United States, which is what a chord
such as Command and C means on every layout. Any other character is typed
as itself.
"""

import ctypes
import ctypes.util
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from columbina.computer.desktop import MODIFIERS, Button, DesktopError

__all__ = ["CODES", "MacOS"]

CODES = {
    **dict(zip("asdfhgzxcv", range(10), strict=True)),
    "b": 11,
    **dict(zip("qweryt123465=97-80]ou[ip", range(12, 36), strict=True)),
    "enter": 36,
    **dict(zip("lj'k;\\,/nm.", range(37, 48), strict=True)),
    "tab": 48,
    "space": 49,
    "`": 50,
    "backspace": 51,
    "escape": 53,
    "super": 55,
    "shift": 56,
    "capslock": 57,
    "alt": 58,
    "ctrl": 59,
    "f5": 96,
    "f6": 97,
    "f7": 98,
    "f3": 99,
    "f8": 100,
    "f9": 101,
    "f11": 103,
    "f10": 109,
    "f12": 111,
    "insert": 114,
    "home": 115,
    "pageup": 116,
    "delete": 117,
    "f4": 118,
    "end": 119,
    "f2": 120,
    "pagedown": 121,
    "f1": 122,
    "left": 123,
    "right": 124,
    "down": 125,
    "up": 126,
}
"""The code of each key on a keyboard of the layout of the United States."""

_FLAGS = {"shift": 1 << 17, "ctrl": 1 << 18, "alt": 1 << 19, "super": 1 << 20}
"""What an event says of a key that is held while it happens."""
_MOVED = 5
_DOWN: dict[Button, int] = {"left": 1, "right": 3, "middle": 25}
_UP: dict[Button, int] = {"left": 2, "right": 4, "middle": 26}
_DRAGGED: dict[Button, int] = {"left": 6, "right": 7, "middle": 27}
_NUMBER: dict[Button, int] = {"left": 0, "right": 1, "middle": 2}
_CLICK_STATE = 1
"""The field of an event of the mouse that says the how-manieth click of a row it is."""
_AGAIN_WITHIN = 0.5
"""How soon after a click another one counts as the next of a double or a triple click."""
_IN_LINES = 1
_WHERE_DEVICES_SPEAK = 0
_UNITS_AT_ONCE = 20
"""How many units of a text one event of the keyboard carries."""
_ALLOW = (
    "Allow it in System Settings, Privacy & Security, {}, for the program that started "
    "Columbina (the terminal, or the application), and start that program again."
)


class _Point(ctypes.Structure):
    _fields_ = (("x", ctypes.c_double), ("y", ctypes.c_double))


def _ran(*command: str) -> None:
    """Run a program of the system to its end.

    Raises:
        DesktopError: It is not there, or it failed.
    """
    try:
        done = subprocess.run(command, capture_output=True, timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise DesktopError(f"{command[0]} could not be run: {error}") from error
    if done.returncode != 0:
        said = done.stderr.decode(errors="replace").strip()
        raise DesktopError(f"{command[0]} failed: {said or done.returncode}")


class MacOS:
    """The desktop of this machine, where it runs macOS.

    Raises:
        DesktopError: Core Graphics cannot be loaded.
    """

    __slots__ = ("_clicked", "_graphics", "_held", "_pressed")

    def __init__(self) -> None:
        found = ctypes.util.find_library("ApplicationServices")
        if found is None:
            raise DesktopError("Core Graphics is not found on this machine")
        self._graphics = g = ctypes.CDLL(found)
        self._held: set[str] = set()
        self._pressed: Button | None = None
        self._clicked: tuple[Button, tuple[int, int], float, int] | None = None
        event = ctypes.c_void_p
        g.CGMainDisplayID.restype = ctypes.c_uint32
        g.CGDisplayPixelsWide.argtypes = (ctypes.c_uint32,)
        g.CGDisplayPixelsWide.restype = ctypes.c_size_t
        g.CGDisplayPixelsHigh.argtypes = (ctypes.c_uint32,)
        g.CGDisplayPixelsHigh.restype = ctypes.c_size_t
        g.CGEventCreate.argtypes = (ctypes.c_void_p,)
        g.CGEventCreate.restype = event
        g.CGEventGetLocation.argtypes = (event,)
        g.CGEventGetLocation.restype = _Point
        g.CGEventCreateMouseEvent.argtypes = (
            ctypes.c_void_p,
            ctypes.c_uint32,
            _Point,
            ctypes.c_uint32,
        )
        g.CGEventCreateMouseEvent.restype = event
        g.CGEventCreateScrollWheelEvent2.argtypes = (
            ctypes.c_void_p,
            ctypes.c_uint32,
            ctypes.c_uint32,
            ctypes.c_int32,
            ctypes.c_int32,
            ctypes.c_int32,
        )
        g.CGEventCreateScrollWheelEvent2.restype = event
        g.CGEventCreateKeyboardEvent.argtypes = (ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool)
        g.CGEventCreateKeyboardEvent.restype = event
        g.CGEventKeyboardSetUnicodeString.argtypes = (
            event,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_uint16),
        )
        g.CGEventSetFlags.argtypes = (event, ctypes.c_uint64)
        g.CGEventSetIntegerValueField.argtypes = (event, ctypes.c_uint32, ctypes.c_int64)
        g.CGEventPost.argtypes = (ctypes.c_uint32, event)
        g.CFRelease.argtypes = (ctypes.c_void_p,)
        g.AXIsProcessTrusted.restype = ctypes.c_bool
        g.CGPreflightScreenCaptureAccess.restype = ctypes.c_bool

    def _posted(self, event: Any) -> None:
        """Send an event where those of the devices go, with the keys that are held said in it."""
        if not event:
            raise DesktopError("macOS made no event to send.")
        flags = 0
        for key in self._held:
            flags |= _FLAGS[key]
        self._graphics.CGEventSetFlags(event, flags)
        self._graphics.CGEventPost(_WHERE_DEVICES_SPEAK, event)
        self._graphics.CFRelease(event)

    def _allowed_to_act(self) -> None:
        if not self._graphics.AXIsProcessTrusted():
            raise DesktopError(
                "macOS does not let this program use the pointer or the keyboard. "
                + _ALLOW.format("Accessibility")
            )

    def size(self) -> tuple[int, int]:
        """Return the width and the height of the main display, in its points."""
        display = self._graphics.CGMainDisplayID()
        return (
            self._graphics.CGDisplayPixelsWide(display),
            self._graphics.CGDisplayPixelsHigh(display),
        )

    def capture(self, region: tuple[int, int, int, int], size: tuple[int, int]) -> bytes:
        """Return a picture of a part of the screen at a size, as a PNG.

        Raises:
            DesktopError: macOS does not let the screen be seen, or one of
                the two programs failed.
        """
        if not self._graphics.CGPreflightScreenCaptureAccess():
            raise DesktopError(
                "macOS does not let this program see the screen. "
                + _ALLOW.format("Screen & System Audio Recording")
            )
        left, top, right, bottom = region
        with tempfile.TemporaryDirectory(prefix="columbina-screen-") as kept:
            picture = Path(kept) / "screen.png"
            part = f"{left},{top},{right - left},{bottom - top}"
            _ran("/usr/sbin/screencapture", "-x", "-t", "png", "-R", part, str(picture))
            if not picture.exists():
                raise DesktopError("screencapture took no picture of the screen.")
            # It takes the height first, and brings the picture to exactly this size.
            _ran("/usr/bin/sips", "-z", str(size[1]), str(size[0]), str(picture))
            return picture.read_bytes()

    def pointer(self) -> tuple[int, int]:
        """Return where the pointer is."""
        event = self._graphics.CGEventCreate(None)
        if not event:
            raise DesktopError("macOS does not say where the pointer is.")
        at = self._graphics.CGEventGetLocation(event)
        self._graphics.CFRelease(event)
        return (int(at.x), int(at.y))

    def _mouse(self, kind: int, at: tuple[int, int], button: Button, clicks: int = 0) -> None:
        event = self._graphics.CGEventCreateMouseEvent(
            None, kind, _Point(float(at[0]), float(at[1])), _NUMBER[button]
        )
        if event and clicks:
            self._graphics.CGEventSetIntegerValueField(event, _CLICK_STATE, clicks)
        self._posted(event)

    def move(self, x: int, y: int) -> None:
        """Put the pointer somewhere. With a button held, what is under it is dragged."""
        self._allowed_to_act()
        held = self._pressed
        self._mouse(_MOVED if held is None else _DRAGGED[held], (x, y), held or "left")

    def button(self, button: Button, down: bool) -> None:
        """Press a button of the pointer, or let it go, where the pointer is."""
        self._allowed_to_act()
        at = self.pointer()
        if not down:
            self._pressed = None
            clicks = self._clicked[3] if self._clicked else 1
            self._mouse(_UP[button], at, button, clicks)
            return
        # A program takes two clicks for a double click only where the second one says that
        # it is the second: of the same button, at the same place, soon after the first.
        now, clicks = time.monotonic(), 1
        last = self._clicked
        if last is not None and last[:2] == (button, at) and now - last[2] <= _AGAIN_WITHIN:
            clicks = last[3] + 1
        self._clicked = (button, at, now, clicks)
        self._pressed = button
        self._mouse(_DOWN[button], at, button, clicks)

    def scroll(self, right: int, down: int) -> None:
        """Turn the wheel by so many lines. The system counts up and left as positive."""
        self._allowed_to_act()
        self._posted(
            self._graphics.CGEventCreateScrollWheelEvent2(None, _IN_LINES, 2, -down, -right, 0)
        )

    def key(self, key: str, down: bool) -> None:
        """Press a key, or let it go."""
        self._allowed_to_act()
        if key in MODIFIERS:
            # An event says which of these are held, its own among them.
            (self._held.add if down else self._held.discard)(key)
        code = CODES.get(key)
        if code is None:
            self._typed(key, down)
            return
        self._posted(self._graphics.CGEventCreateKeyboardEvent(None, code, down))

    def _typed(self, text: str, down: bool) -> None:
        """Send an event of the keyboard that carries characters as they are."""
        encoded = text.encode("utf-16-le")
        units = (ctypes.c_uint16 * (len(encoded) // 2)).from_buffer_copy(encoded)
        event = self._graphics.CGEventCreateKeyboardEvent(None, 0, down)
        if event:
            self._graphics.CGEventKeyboardSetUnicodeString(event, len(units), units)
        self._posted(event)

    def text(self, text: str) -> None:
        """Type a text as it is, whatever the layout of the keyboard."""
        self._allowed_to_act()
        named = {"\n": "enter", "\t": "tab"}
        pieces = [""]
        for character in text.replace("\r\n", "\n").replace("\r", "\n"):
            if character in named:
                pieces += [character, ""]
            elif len(pieces[-1].encode("utf-16-le")) // 2 + 2 > _UNITS_AT_ONCE:
                # A character beyond the first plane is two units, which are not parted.
                pieces.append(character)
            else:
                pieces[-1] += character
        for piece in pieces:
            if piece in named:
                self.key(named[piece], True)
                self.key(named[piece], False)
            elif piece:
                self._typed(piece, True)
                self._typed(piece, False)

    def close(self) -> None:
        """Let go of what was taken from the system, which is nothing that stays."""
