"""The desktop of this machine, as the tool ``computer`` reaches it.

A desktop is what a person sits in front of: a screen, a pointer and a
keyboard. :class:`Desktop` says what the tool asks of one, and each system
answers it in its own way, see :mod:`columbina.computer.windows`,
:mod:`columbina.computer.macos` and :mod:`columbina.computer.x11`.
:func:`find` picks the one for this machine, or says why there is none.

Positions are in the pixels of the screen as the system counts them, from its
upper left corner. Only the main display is reached.

Keys are named in one way for all systems, see :func:`keys_of`: the names
that are common to the keyboards of all three, and any one character.
"""

import os
import sys
from collections.abc import Mapping
from typing import Literal, Protocol

__all__ = [
    "KEYS",
    "MODIFIERS",
    "Button",
    "Desktop",
    "DesktopError",
    "find",
    "keys_of",
]

Button = Literal["left", "right", "middle"]


class DesktopError(Exception):
    """Something could not be done on the desktop. Its text is for the model to read."""


class Desktop(Protocol):
    """A screen, a pointer and a keyboard.

    Every method does its work before it returns, and blocks while it does.
    None of them waits for the desktop to settle: the tool does that.
    """

    def size(self) -> tuple[int, int]:
        """Return the width and the height of the screen, in its pixels."""
        ...

    def capture(self, region: tuple[int, int, int, int], size: tuple[int, int]) -> bytes:
        """Return a picture of a part of the screen, as a PNG.

        Args:
            region: The part: its left, top, right and bottom, in the pixels
                of the screen. The right and the bottom are not in it.
            size: The width and the height that the picture is to have.
        """
        ...

    def pointer(self) -> tuple[int, int]:
        """Return where the pointer is."""
        ...

    def move(self, x: int, y: int) -> None:
        """Put the pointer somewhere."""
        ...

    def button(self, button: Button, down: bool) -> None:
        """Press a button of the pointer, or let it go, where the pointer is."""
        ...

    def scroll(self, right: int, down: int) -> None:
        """Turn the wheel by so many notches, where the pointer is. Left and up are negative."""
        ...

    def key(self, key: str, down: bool) -> None:
        """Press a key, or let it go. The key is one that :func:`keys_of` names."""
        ...

    def text(self, text: str) -> None:
        """Type a text as it is, whatever the layout of the keyboard."""
        ...

    def close(self) -> None:
        """Let go of what was taken from the system."""
        ...


MODIFIERS = ("ctrl", "alt", "shift", "super")
"""The keys that are held while another is pressed. ``super`` is Command, or the key of Windows."""

KEYS = (
    *MODIFIERS,
    "enter",
    "tab",
    "space",
    "backspace",
    "delete",
    "escape",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
    "capslock",
    *(f"f{number}" for number in range(1, 13)),
)
"""The keys that have a name. Any other key is named by the one character that it types."""

_ALSO = {
    "control": "ctrl",
    "option": "alt",
    "cmd": "super",
    "command": "super",
    "win": "super",
    "windows": "super",
    "meta": "super",
    "return": "enter",
    "esc": "escape",
    "del": "delete",
    "ins": "insert",
    "page_up": "pageup",
    "pgup": "pageup",
    "prior": "pageup",
    "page_down": "pagedown",
    "pgdn": "pagedown",
    "next": "pagedown",
    "caps_lock": "capslock",
    "plus": "+",
}
"""Other names that a key goes by, such as those that ``xdotool`` has for it."""


def keys_of(said: str) -> tuple[str, ...]:
    """Return the keys of a chord such as ``ctrl+shift+t``, each by its one name.

    The keys are held in their order and let go in the reverse of it. A name
    is read in any case, and a single character stands for its key as it is.

    Raises:
        DesktopError: A part of the chord names no key, or the chord is empty.
    """
    # The key of the plus sign is written out, since the sign joins the keys.
    parts = [part.strip() for part in said.split("+")]
    keys: list[str] = []
    for part in parts:
        lowered = part.lower()
        key = _ALSO.get(lowered, lowered)
        if key in KEYS or len(key) == 1:
            keys.append(key)
        else:
            known = ", ".join(KEYS)
            raise DesktopError(
                f"{part!r} names no key. A key is one character, or one of: {known}. "
                "Keys that are pressed together are joined by +, as in ctrl+c."
            )
    return tuple(keys)


def find() -> Desktop | str:  # pragma: no cover
    """Return the desktop of this machine, or why it cannot be reached, in words for the model.

    No test calls this: it reaches for the desktop of the machine that it runs on, and the
    tests give the tool a desktop of their own.
    """
    if sys.platform == "win32":  # pragma: windows only
        from columbina.computer import windows

        return windows.Windows()
    if sys.platform == "darwin":  # pragma: posix only
        from columbina.computer import macos

        return macos.MacOS()
    return _on_linux(os.environ)  # pragma: posix only


def _on_linux(env: Mapping[str, str]) -> Desktop | str:  # pragma: posix only
    """Return the X display of this session, or why there is none to reach."""
    if env.get("WAYLAND_DISPLAY") or env.get("XDG_SESSION_TYPE") == "wayland":
        return (
            "this is a session of Wayland, which lets no program see the screen or move the "
            "pointer of another without a portal that is not spoken to yet. A session of X11 "
            "is reached"
        )
    if not env.get("DISPLAY"):
        return "there is no display: the variable DISPLAY names none"
    # From here on a display is opened, which the machines of the tests have not.
    from columbina.computer import x11  # pragma: no cover

    try:  # pragma: no cover
        return x11.X11(env["DISPLAY"])
    except DesktopError as error:  # pragma: no cover
        return str(error)
