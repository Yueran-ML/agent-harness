"""The desktop of Windows, reached through what the system brings: ``user32`` and ``gdi32``.

The pointer and the keyboard are moved by ``SendInput``, which puts events
where those of the devices go. A text is typed as the characters that it has,
whatever the layout of the keyboard. The system lets a program put nothing
into the window of one that runs with higher rights, and does not say so:
what is seen is that it took none of the events.

The picture of the screen is copied from the screen by GDI, which also
brings it to the size that is asked for.

Windows tells a program that has not said otherwise a screen of fewer pixels
than there are, and scales what the program does. The thread that speaks to
the desktop says that it knows each display as it is, so that a pixel here
is a pixel of the screen.
"""

import ctypes
import sys
import threading
from ctypes import wintypes

from columbina.computer import png
from columbina.computer.desktop import Button, DesktopError

# Nothing below is there on another system, and nothing else loads this module there. What the
# module gives is named after this line, so that a checker of types that reads the module as
# another system would, and so reads nothing below, does not look for it.
assert sys.platform == "win32"

__all__ = ["Windows"]

_user32 = ctypes.WinDLL("user32", use_last_error=True)
_gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)

_ULONG_PTR = ctypes.c_size_t


class _MouseInput(ctypes.Structure):
    _fields_ = (
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", _ULONG_PTR),
    )


class _KeyboardInput(ctypes.Structure):
    _fields_ = (
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", _ULONG_PTR),
    )


class _HardwareInput(ctypes.Structure):
    _fields_ = (
        ("uMsg", wintypes.DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    )


class _Either(ctypes.Union):
    _fields_ = (("mi", _MouseInput), ("ki", _KeyboardInput), ("hi", _HardwareInput))


class _Input(ctypes.Structure):
    _anonymous_ = ("either",)
    _fields_ = (("type", wintypes.DWORD), ("either", _Either))


class _BitmapHeader(ctypes.Structure):
    _fields_ = (
        ("biSize", wintypes.DWORD),
        ("biWidth", wintypes.LONG),
        ("biHeight", wintypes.LONG),
        ("biPlanes", wintypes.WORD),
        ("biBitCount", wintypes.WORD),
        ("biCompression", wintypes.DWORD),
        ("biSizeImage", wintypes.DWORD),
        ("biXPelsPerMeter", wintypes.LONG),
        ("biYPelsPerMeter", wintypes.LONG),
        ("biClrUsed", wintypes.DWORD),
        ("biClrImportant", wintypes.DWORD),
    )


_user32.GetDC.argtypes = (wintypes.HWND,)
_user32.GetDC.restype = wintypes.HDC
_user32.ReleaseDC.argtypes = (wintypes.HWND, wintypes.HDC)
_user32.GetSystemMetrics.argtypes = (ctypes.c_int,)
_user32.GetCursorPos.argtypes = (ctypes.POINTER(wintypes.POINT),)
_user32.SetCursorPos.argtypes = (ctypes.c_int, ctypes.c_int)
_user32.SendInput.argtypes = (wintypes.UINT, ctypes.POINTER(_Input), ctypes.c_int)
_user32.SendInput.restype = wintypes.UINT
_user32.VkKeyScanW.argtypes = (wintypes.WCHAR,)
_user32.VkKeyScanW.restype = ctypes.c_short
_user32.MapVirtualKeyW.argtypes = (wintypes.UINT, wintypes.UINT)
_user32.MapVirtualKeyW.restype = wintypes.UINT
_gdi32.CreateCompatibleDC.argtypes = (wintypes.HDC,)
_gdi32.CreateCompatibleDC.restype = wintypes.HDC
_gdi32.CreateCompatibleBitmap.argtypes = (wintypes.HDC, ctypes.c_int, ctypes.c_int)
_gdi32.CreateCompatibleBitmap.restype = wintypes.HBITMAP
_gdi32.SelectObject.argtypes = (wintypes.HDC, wintypes.HGDIOBJ)
_gdi32.SelectObject.restype = wintypes.HGDIOBJ
_gdi32.SetStretchBltMode.argtypes = (wintypes.HDC, ctypes.c_int)
_gdi32.SetBrushOrgEx.argtypes = (wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.LPVOID)
_gdi32.StretchBlt.argtypes = (
    wintypes.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.HDC,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_int,
    wintypes.DWORD,
)
_gdi32.GetDIBits.argtypes = (
    wintypes.HDC,
    wintypes.HBITMAP,
    wintypes.UINT,
    wintypes.UINT,
    wintypes.LPVOID,
    wintypes.LPVOID,
    wintypes.UINT,
)
_gdi32.DeleteObject.argtypes = (wintypes.HGDIOBJ,)
_gdi32.DeleteDC.argtypes = (wintypes.HDC,)

_MOUSE, _KEYBOARD = 0, 1
_MOVE, _ABSOLUTE = 0x0001, 0x8000
_WHEEL, _HWHEEL = 0x0800, 0x1000
_NOTCH = 120
_BUTTONS: dict[Button, tuple[int, int]] = {
    "left": (0x0002, 0x0004),
    "right": (0x0008, 0x0010),
    "middle": (0x0020, 0x0040),
}
"""What ``SendInput`` calls it when a button goes down, and when it comes up."""
_EXTENDED, _UP, _UNICODE = 0x0001, 0x0002, 0x0004
_SHIFT = 0x10
_VIRTUAL = {
    "ctrl": 0x11,
    "alt": 0x12,
    "shift": _SHIFT,
    "super": 0x5B,
    "enter": 0x0D,
    "tab": 0x09,
    "space": 0x20,
    "backspace": 0x08,
    "delete": 0x2E,
    "escape": 0x1B,
    "insert": 0x2D,
    "home": 0x24,
    "end": 0x23,
    "pageup": 0x21,
    "pagedown": 0x22,
    "up": 0x26,
    "down": 0x28,
    "left": 0x25,
    "right": 0x27,
    "capslock": 0x14,
    **{f"f{number}": 0x6F + number for number in range(1, 13)},
}
"""The virtual key of each key that has a name."""
_ON_THE_EXTENDED_PAD = frozenset(
    {
        "super",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "up",
        "down",
        "left",
        "right",
    }
)
"""The keys that a keyboard has twice, of which the one beside the letters is meant."""
_SRCCOPY, _CAPTUREBLT, _HALFTONE = 0x00CC0020, 0x40000000, 4
_EACH_DISPLAY_AS_IT_IS = ctypes.c_void_p(-4)
"""What a thread tells Windows so that it is told the pixels of each display as they are."""


def _mouse(flags: int, data: int = 0, x: int = 0, y: int = 0) -> _Input:
    event = _Input(type=_MOUSE)
    event.mi = _MouseInput(dx=x, dy=y, mouseData=data & 0xFFFFFFFF, dwFlags=flags)
    return event


def _keyboard(virtual: int, scan: int, flags: int) -> _Input:
    event = _Input(type=_KEYBOARD)
    event.ki = _KeyboardInput(wVk=virtual, wScan=scan, dwFlags=flags)
    return event


def _sent(*events: _Input) -> None:
    """Put events where those of the devices go.

    Raises:
        DesktopError: The system took none of them.
    """
    batch = (_Input * len(events))(*events)
    if _user32.SendInput(len(events), batch, ctypes.sizeof(_Input)) != len(events):
        raise DesktopError(
            "Windows took none of the input. It lets a program put nothing into the window of "
            "one that runs with higher rights, such as one that was started as administrator, "
            "and takes nothing while the screen is locked."
        )


class Windows:
    """The desktop of this machine, where it runs Windows."""

    __slots__ = ("_aware",)

    def __init__(self) -> None:
        self._aware = threading.local()

    def _as_it_is(self) -> None:
        """Have this thread told the pixels of each display as they are, once."""
        if getattr(self._aware, "said", False):
            return
        self._aware.said = True
        say = getattr(_user32, "SetThreadDpiAwarenessContext", None)
        # A Windows from before 2016 has no such call, and tells every thread the same.
        if say is not None:
            say.argtypes = (ctypes.c_void_p,)
            say.restype = ctypes.c_void_p
            say(_EACH_DISPLAY_AS_IT_IS)

    def size(self) -> tuple[int, int]:
        """Return the width and the height of the main display, in its pixels."""
        self._as_it_is()
        return (_user32.GetSystemMetrics(0), _user32.GetSystemMetrics(1))

    def capture(self, region: tuple[int, int, int, int], size: tuple[int, int]) -> bytes:
        """Return a picture of a part of the screen at a size, as a PNG.

        Raises:
            DesktopError: The screen could not be copied.
        """
        self._as_it_is()
        left, top, right, bottom = region
        width, height = size
        screen = _user32.GetDC(None)
        memory = _gdi32.CreateCompatibleDC(screen)
        bitmap = _gdi32.CreateCompatibleBitmap(screen, width, height)
        try:
            if not (screen and memory and bitmap):
                raise DesktopError("Windows gave nothing to copy the screen into.")
            before = _gdi32.SelectObject(memory, bitmap)
            _gdi32.SetStretchBltMode(memory, _HALFTONE)
            _gdi32.SetBrushOrgEx(memory, 0, 0, None)
            copied = _gdi32.StretchBlt(
                memory,
                0,
                0,
                width,
                height,
                screen,
                left,
                top,
                right - left,
                bottom - top,
                _SRCCOPY | _CAPTUREBLT,
            )
            # The rows come from the top where the height is written as a negative one.
            header = _BitmapHeader(
                biSize=ctypes.sizeof(_BitmapHeader),
                biWidth=width,
                biHeight=-height,
                biPlanes=1,
                biBitCount=32,
            )
            pixels = ctypes.create_string_buffer(width * height * 4)
            rows = _gdi32.GetDIBits(memory, bitmap, 0, height, pixels, ctypes.byref(header), 0)
            _gdi32.SelectObject(memory, before)
            if not copied or rows != height:
                raise DesktopError(
                    "Windows did not let the screen be copied. It lets nobody while the screen "
                    "is locked, or while it asks for the consent of an administrator."
                )
        finally:
            if bitmap:
                _gdi32.DeleteObject(bitmap)
            if memory:
                _gdi32.DeleteDC(memory)
            if screen:
                _user32.ReleaseDC(None, screen)
        # GDI writes a pixel as its blue, green and red, and a byte that is not used.
        raw = pixels.raw
        rgb = bytearray(width * height * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = raw[2::4], raw[1::4], raw[0::4]
        return png.encode(width, height, bytes(rgb))

    def pointer(self) -> tuple[int, int]:
        """Return where the pointer is."""
        self._as_it_is()
        point = wintypes.POINT()
        if not _user32.GetCursorPos(ctypes.byref(point)):
            raise DesktopError(
                "Windows does not say where the pointer is: the screen may be locked."
            )
        return (point.x, point.y)

    def move(self, x: int, y: int) -> None:
        """Put the pointer somewhere on the main display."""
        width, height = self.size()
        # An event of the mouse counts a display as 65536 steps each way. The windows under
        # the pointer hear of it as of a mouse that moved; then the pointer is set to the
        # pixel that was meant, which the steps may miss by one.
        _sent(
            _mouse(
                _MOVE | _ABSOLUTE,
                x=(x * 65536 + width // 2) // width,
                y=(y * 65536 + height // 2) // height,
            )
        )
        _user32.SetCursorPos(x, y)

    def button(self, button: Button, down: bool) -> None:
        """Press a button of the pointer, or let it go."""
        self._as_it_is()
        _sent(_mouse(_BUTTONS[button][0 if down else 1]))

    def scroll(self, right: int, down: int) -> None:
        """Turn the wheel by so many notches."""
        self._as_it_is()
        events: list[_Input] = []
        if down:
            # The wheel counts away from the person as up, and a notch as 120.
            events.append(_mouse(_WHEEL, data=-down * _NOTCH))
        if right:
            events.append(_mouse(_HWHEEL, data=right * _NOTCH))
        if events:
            _sent(*events)

    def key(self, key: str, down: bool) -> None:
        """Press a key, or let it go."""
        self._as_it_is()
        up = 0 if down else _UP
        virtual = _VIRTUAL.get(key)
        if virtual is not None:
            extended = _EXTENDED if key in _ON_THE_EXTENDED_PAD else 0
            _sent(_keyboard(virtual, _user32.MapVirtualKeyW(virtual, 0), extended | up))
            return
        found = _user32.VkKeyScanW(key)
        if found == -1:
            # The layout has no key for the character, so the character itself is sent.
            _sent(*(_keyboard(0, unit, _UNICODE | up) for unit in _units(key)))
            return
        virtual, shifted = found & 0xFF, bool(found >> 8 & 1)
        one = _keyboard(virtual, _user32.MapVirtualKeyW(virtual, 0), up)
        shift = _keyboard(_SHIFT, _user32.MapVirtualKeyW(_SHIFT, 0), up)
        if not shifted:
            _sent(one)
        elif down:
            _sent(shift, one)
        else:
            _sent(one, shift)

    def text(self, text: str) -> None:
        """Type a text as it is, whatever the layout of the keyboard."""
        self._as_it_is()
        events: list[_Input] = []
        for character in text.replace("\r\n", "\n").replace("\r", "\n"):
            named = {"\n": "enter", "\t": "tab"}.get(character)
            if named is not None:
                # A line break that is sent as a character breaks no line in most windows.
                virtual = _VIRTUAL[named]
                scan = _user32.MapVirtualKeyW(virtual, 0)
                events += (_keyboard(virtual, scan, 0), _keyboard(virtual, scan, _UP))
                continue
            for unit in _units(character):
                events += (_keyboard(0, unit, _UNICODE), _keyboard(0, unit, _UNICODE | _UP))
        if events:
            _sent(*events)

    def close(self) -> None:
        """Let go of what was taken from the system, which is nothing that stays."""


def _units(character: str) -> list[int]:
    """Return a character as Windows counts it: in one unit of sixteen bits, or in two."""
    encoded = character.encode("utf-16-le")
    return [int.from_bytes(encoded[at : at + 2], "little") for at in range(0, len(encoded), 2)]
