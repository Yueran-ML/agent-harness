"""The desktop of a session of X11, reached through ``libX11`` and ``libXtst``.

The pointer and the keyboard are moved by the XTest extension, which every
X server of a desktop has: it puts events where those of the devices go.
The picture of the screen is read from the root window, and made smaller
here, since the server only hands pixels over as they are.

A key is pressed by its code, and which code types a character is a matter
of the layout of the keyboard. A character that the layout has no key for
is laid on a code that no key has, pressed there, and taken off again when
the desktop is let go of.

Xlib ends the program on an error of the server, unless it is told to do
something else. So it is told: the error is noted, and said in words where
the call that led to it comes back.
"""

import ctypes
import ctypes.util
import time
from typing import Any

from columbina.computer import png
from columbina.computer.desktop import Button, DesktopError

__all__ = ["X11", "keysym_of"]

_NAMED = {
    "ctrl": 0xFFE3,
    "alt": 0xFFE9,
    "shift": 0xFFE1,
    "super": 0xFFEB,
    "enter": 0xFF0D,
    "tab": 0xFF09,
    "space": 0x0020,
    "backspace": 0xFF08,
    "delete": 0xFFFF,
    "escape": 0xFF1B,
    "insert": 0xFF63,
    "home": 0xFF50,
    "end": 0xFF57,
    "pageup": 0xFF55,
    "pagedown": 0xFF56,
    "up": 0xFF52,
    "down": 0xFF54,
    "left": 0xFF51,
    "right": 0xFF53,
    "capslock": 0xFFE5,
    **{f"f{number}": 0xFFBD + number for number in range(1, 13)},
}
"""The keysym of each key that has a name."""
_BUTTONS: dict[Button, int] = {"left": 1, "middle": 2, "right": 3}
_WHEEL_UP, _WHEEL_DOWN, _WHEEL_LEFT, _WHEEL_RIGHT = 4, 5, 6, 7
_Z_PIXMAP = 2
_ALL_PLANES = ctypes.c_ulong(~0)
_SETTLES = 0.02
"""How long the programs of the desktop are given to hear that a key has another character."""


class _Image(ctypes.Structure):
    """The beginning of an ``XImage``: what is read of it here."""

    _fields_ = (
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("xoffset", ctypes.c_int),
        ("format", ctypes.c_int),
        ("data", ctypes.POINTER(ctypes.c_char)),
        ("byte_order", ctypes.c_int),
        ("bitmap_unit", ctypes.c_int),
        ("bitmap_bit_order", ctypes.c_int),
        ("bitmap_pad", ctypes.c_int),
        ("depth", ctypes.c_int),
        ("bytes_per_line", ctypes.c_int),
        ("bits_per_pixel", ctypes.c_int),
        ("red_mask", ctypes.c_ulong),
        ("green_mask", ctypes.c_ulong),
        ("blue_mask", ctypes.c_ulong),
    )


_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)
_failed: list[bool] = []
"""Whether the server refused something since this was last looked at."""


@_HANDLER
def _noted(display: Any, event: Any) -> int:
    """Note that the server refused something, where Xlib would end the program."""
    _failed.append(True)
    return 0


def keysym_of(key: str) -> int:
    """Return the keysym of a key: that of its name, or that of the one character that it types."""
    named = _NAMED.get(key)
    if named is not None:
        return named
    point = ord(key)
    # The characters of Latin-1 are their own keysyms. The others are marked as Unicode.
    if 0x20 <= point <= 0x7E or 0xA0 <= point <= 0xFF:
        return point
    return 0x01000000 | point


def _library(name: str) -> Any:
    found = ctypes.util.find_library(name)
    if found is None:
        raise DesktopError(
            f"the library lib{name} is not on this machine, without which the screen of X11 "
            "is not reached. It comes with the packages libx11-6 and libxtst6, or those that "
            "the system has for them"
        )
    return ctypes.CDLL(found)


class X11:
    """The desktop of this machine, where a session of X11 runs.

    Args:
        display: The display, as the variable ``DISPLAY`` names it.

    Raises:
        DesktopError: The libraries are not there, the display cannot be
            opened, or its server has no XTest extension.
    """

    __slots__ = ("_display", "_root", "_shifted", "_spare", "_x", "_xtst")

    def __init__(self, display: str) -> None:
        self._x = x = _library("X11")
        self._xtst = xtst = _library("Xtst")
        x.XOpenDisplay.argtypes = (ctypes.c_char_p,)
        x.XOpenDisplay.restype = ctypes.c_void_p
        x.XDefaultRootWindow.argtypes = (ctypes.c_void_p,)
        x.XDefaultRootWindow.restype = ctypes.c_ulong
        x.XDefaultScreen.argtypes = (ctypes.c_void_p,)
        x.XDisplayWidth.argtypes = (ctypes.c_void_p, ctypes.c_int)
        x.XDisplayHeight.argtypes = (ctypes.c_void_p, ctypes.c_int)
        x.XSync.argtypes = (ctypes.c_void_p, ctypes.c_int)
        x.XCloseDisplay.argtypes = (ctypes.c_void_p,)
        x.XQueryPointer.argtypes = (
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_uint),
        )
        x.XGetImage.argtypes = (
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.c_ulong,
            ctypes.c_int,
        )
        x.XGetImage.restype = ctypes.POINTER(_Image)
        x.XDestroyImage.argtypes = (ctypes.POINTER(_Image),)
        x.XKeysymToKeycode.argtypes = (ctypes.c_void_p, ctypes.c_ulong)
        x.XKeysymToKeycode.restype = ctypes.c_ubyte
        x.XkbKeycodeToKeysym.argtypes = (
            ctypes.c_void_p,
            ctypes.c_ubyte,
            ctypes.c_uint,
            ctypes.c_uint,
        )
        x.XkbKeycodeToKeysym.restype = ctypes.c_ulong
        x.XDisplayKeycodes.argtypes = (
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        )
        x.XGetKeyboardMapping.argtypes = (
            ctypes.c_void_p,
            ctypes.c_ubyte,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_int),
        )
        x.XGetKeyboardMapping.restype = ctypes.POINTER(ctypes.c_ulong)
        x.XChangeKeyboardMapping.argtypes = (
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.c_int,
        )
        x.XFree.argtypes = (ctypes.c_void_p,)
        x.XSetErrorHandler.argtypes = (_HANDLER,)
        x.XSetErrorHandler.restype = ctypes.c_void_p
        xtst.XTestQueryExtension.argtypes = (
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int),
        )
        xtst.XTestFakeMotionEvent.argtypes = (
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_ulong,
        )
        xtst.XTestFakeButtonEvent.argtypes = (
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        )
        xtst.XTestFakeKeyEvent.argtypes = (
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        )
        x.XSetErrorHandler(_noted)
        opened = x.XOpenDisplay(display.encode())
        if not opened:
            raise DesktopError(f"the display {display} cannot be opened")
        self._display = ctypes.c_void_p(opened)
        self._root = x.XDefaultRootWindow(self._display)
        self._spare = 0
        self._shifted: set[int] = set()
        numbers = [ctypes.c_int() for _ in range(4)]
        if not xtst.XTestQueryExtension(self._display, *(ctypes.byref(n) for n in numbers)):
            x.XCloseDisplay(self._display)
            raise DesktopError(
                f"the X server of the display {display} has no XTest extension, by which the "
                "pointer and the keyboard are moved"
            )

    def _settled(self, what: str) -> None:
        """Have the server do all that it was sent, and say so if it refused any of it."""
        _failed.clear()
        self._x.XSync(self._display, 0)
        if _failed:
            _failed.clear()
            raise DesktopError(f"The X server refused to {what}.")

    def size(self) -> tuple[int, int]:
        """Return the width and the height of the screen, in its pixels."""
        screen = self._x.XDefaultScreen(self._display)
        return (
            self._x.XDisplayWidth(self._display, screen),
            self._x.XDisplayHeight(self._display, screen),
        )

    def capture(self, region: tuple[int, int, int, int], size: tuple[int, int]) -> bytes:
        """Return a picture of a part of the screen at a size, as a PNG.

        Raises:
            DesktopError: The server did not hand the pixels over, or not in
                a form that is read here.
        """
        left, top, right, bottom = region
        width, height = right - left, bottom - top
        _failed.clear()
        image = self._x.XGetImage(
            self._display, self._root, left, top, width, height, _ALL_PLANES, _Z_PIXMAP
        )
        if not image or _failed:
            _failed.clear()
            raise DesktopError("The X server did not hand over the pixels of the screen.")
        try:
            held = image.contents
            masks = (held.red_mask, held.green_mask, held.blue_mask)
            if held.bits_per_pixel != 32 or masks != (0xFF0000, 0x00FF00, 0x0000FF):
                raise DesktopError(
                    f"The screen has pixels of {held.bits_per_pixel} bits, in an order that is "
                    "not read here: only screens of 24 bits of colour are."
                )
            line = held.bytes_per_line
            # What is read of the image is read before the image is given back.
            lowest_first = held.byte_order == 0
            raw = ctypes.string_at(held.data, line * height)
        finally:
            self._x.XDestroyImage(image)
        if line != width * 4:
            raw = b"".join(raw[at : at + width * 4] for at in range(0, len(raw), line))
        # A pixel is four bytes: blue, green, red and one that is not used, or the reverse of
        # that where the server writes the highest byte first.
        red, green, blue = (2, 1, 0) if lowest_first else (1, 2, 3)
        rgb = bytearray(width * height * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = raw[red::4], raw[green::4], raw[blue::4]
        return png.encode(*size, png.shrunk(width, height, bytes(rgb), size))

    def pointer(self) -> tuple[int, int]:
        """Return where the pointer is."""
        windows = (ctypes.c_ulong(), ctypes.c_ulong())
        places = [ctypes.c_int() for _ in range(4)]
        held = ctypes.c_uint()
        self._x.XQueryPointer(
            self._display,
            self._root,
            *(ctypes.byref(window) for window in windows),
            *(ctypes.byref(place) for place in places),
            ctypes.byref(held),
        )
        return (places[0].value, places[1].value)

    def move(self, x: int, y: int) -> None:
        """Put the pointer somewhere."""
        self._xtst.XTestFakeMotionEvent(self._display, -1, x, y, 0)
        self._settled("move the pointer")

    def button(self, button: Button, down: bool) -> None:
        """Press a button of the pointer, or let it go."""
        self._xtst.XTestFakeButtonEvent(self._display, _BUTTONS[button], int(down), 0)
        self._settled("press a button of the pointer")

    def scroll(self, right: int, down: int) -> None:
        """Turn the wheel by so many notches: X counts a notch as a click of a button."""
        turns = (
            (_WHEEL_DOWN if down > 0 else _WHEEL_UP, abs(down)),
            (_WHEEL_RIGHT if right > 0 else _WHEEL_LEFT, abs(right)),
        )
        for wheel, notches in turns:
            for _ in range(notches):
                self._xtst.XTestFakeButtonEvent(self._display, wheel, 1, 0)
                self._xtst.XTestFakeButtonEvent(self._display, wheel, 0, 0)
        self._settled("turn the wheel")

    def _code(self, keysym: int) -> tuple[int, bool]:
        """Return the code of the key that types a keysym, and whether Shift is held for it."""
        code = self._x.XKeysymToKeycode(self._display, keysym)
        if code:
            if self._x.XkbKeycodeToKeysym(self._display, code, 0, 0) == keysym:
                return code, False
            if self._x.XkbKeycodeToKeysym(self._display, code, 0, 1) == keysym:
                return code, True
        return self._laid_on_a_spare(keysym), False

    def _laid_on_a_spare(self, keysym: int) -> int:
        """Lay a keysym on a code that no key has, and return the code.

        Raises:
            DesktopError: Every code has a key.
        """
        if not self._spare:
            lowest, highest, each = ctypes.c_int(), ctypes.c_int(), ctypes.c_int()
            self._x.XDisplayKeycodes(self._display, ctypes.byref(lowest), ctypes.byref(highest))
            count = highest.value - lowest.value + 1
            mapping = self._x.XGetKeyboardMapping(
                self._display, lowest.value, count, ctypes.byref(each)
            )
            try:
                for code in range(highest.value, lowest.value - 1, -1):
                    row = (code - lowest.value) * each.value
                    if not any(mapping[row + column] for column in range(each.value)):
                        self._spare = code
                        break
            finally:
                self._x.XFree(mapping)
            if not self._spare:
                raise DesktopError(
                    "The keyboard has no key for a character of the text, and no code is free "
                    "to lay it on."
                )
        self._x.XChangeKeyboardMapping(
            self._display, self._spare, 1, ctypes.byref(ctypes.c_ulong(keysym)), 1
        )
        self._settled("lay a character on a key")
        # The programs of the desktop hear of the change a moment after the server made it.
        time.sleep(_SETTLES)
        return self._spare

    def _fake(self, code: int, down: bool) -> None:
        self._xtst.XTestFakeKeyEvent(self._display, code, int(down), 0)

    def key(self, key: str, down: bool) -> None:
        """Press a key, or let it go."""
        code, shifted = self._code(keysym_of(key))
        shift = self._x.XKeysymToKeycode(self._display, _NAMED["shift"])
        if down:
            if shifted:
                self._fake(shift, True)
                self._shifted.add(code)
            self._fake(code, True)
        else:
            self._fake(code, False)
            # The Shift that was pressed for the key goes with it, however the key is found now.
            if code in self._shifted:
                self._shifted.discard(code)
                self._fake(shift, False)
        self._settled("press a key")

    def text(self, text: str) -> None:
        """Type a text as it is, whatever the layout of the keyboard."""
        for character in text.replace("\r\n", "\n").replace("\r", "\n"):
            key = {"\n": "enter", "\t": "tab", " ": "space"}.get(character, character)
            self.key(key, True)
            self.key(key, False)

    def close(self) -> None:
        """Take the last character off the spare key, and close the display."""
        if self._spare:
            self._x.XChangeKeyboardMapping(
                self._display, self._spare, 1, ctypes.byref(ctypes.c_ulong(0)), 1
            )
            self._x.XSync(self._display, 0)
        self._x.XCloseDisplay(self._display)
