"""What the tool ``computer`` needs of a desktop that no test has: keys, pictures, and which one."""

import struct
import zlib

import pytest

from columbina.computer import desktop, png
from columbina.computer.desktop import KEYS, DesktopError, keys_of
from columbina.computer.macos import CODES
from columbina.computer.x11 import keysym_of

# Keys


@pytest.mark.parametrize(
    ("said", "keys"),
    [
        ("ctrl+c", ("ctrl", "c")),
        ("Control+Shift+T", ("ctrl", "shift", "t")),
        ("cmd+space", ("super", "space")),
        ("Win+d", ("super", "d")),
        ("option+Left", ("alt", "left")),
        ("Return", ("enter",)),
        ("Page_Down", ("pagedown",)),
        ("Prior", ("pageup",)),
        ("Esc", ("escape",)),
        ("F5", ("f5",)),
        ("ctrl+plus", ("ctrl", "+")),
        ("?", ("?",)),
        ("é", ("é",)),
        (" alt + tab ", ("alt", "tab")),
    ],
)
def test_a_chord_is_its_keys_each_by_its_one_name(said: str, keys: tuple[str, ...]) -> None:
    assert keys_of(said) == keys


@pytest.mark.parametrize("said", ["", "ctrl+", "ctrl++", "hyper+a", "f13", "ctrl+ab"])
def test_a_chord_with_something_that_is_no_key_is_refused(said: str) -> None:
    with pytest.raises(DesktopError, match="names no key"):
        keys_of(said)


def test_every_key_that_has_a_name_has_a_place_on_each_system() -> None:
    for key in KEYS:
        assert key in CODES, key
        assert keysym_of(key) > 0xFF or key == "space", key
    # Twelve keys of functions, in their order on X11, and none twice on a Mac.
    assert [keysym_of(f"f{number}") for number in (1, 12)] == [0xFFBE, 0xFFC9]
    assert len(set(CODES.values())) == len(CODES)


def test_a_character_is_its_own_keysym_or_marked_as_unicode() -> None:
    assert keysym_of("a") == 0x61
    assert keysym_of("é") == 0xE9
    assert keysym_of("中") == 0x01004E2D
    assert keysym_of("\x7f") == 0x0100007F


def test_the_letters_of_a_mac_are_where_its_keyboard_has_them() -> None:
    assert (CODES["a"], CODES["c"], CODES["v"], CODES["b"], CODES["q"]) == (0, 8, 9, 11, 12)
    assert (CODES["p"], CODES["enter"], CODES["l"], CODES["."]) == (35, 36, 37, 47)
    assert (CODES["tab"], CODES["space"], CODES["backspace"], CODES["escape"]) == (48, 49, 51, 53)


# Pictures


def chunks(data: bytes) -> list[tuple[bytes, bytes]]:
    """Return the chunks of a PNG, each checked against its sum."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    found: list[tuple[bytes, bytes]] = []
    at = 8
    while at < len(data):
        (length,) = struct.unpack(">I", data[at : at + 4])
        kind, body = data[at + 4 : at + 8], data[at + 8 : at + 8 + length]
        (crc,) = struct.unpack(">I", data[at + 8 + length : at + 12 + length])
        assert crc == zlib.crc32(kind + body)
        found.append((kind, body))
        at += 12 + length
    return found


def test_a_picture_is_written_as_a_png_that_holds_its_pixels() -> None:
    rgb = bytes(range(18))

    written = chunks(png.encode(3, 2, rgb))

    assert [kind for kind, _ in written] == [b"IHDR", b"IDAT", b"IEND"]
    assert written[0][1] == struct.pack(">IIBBBBB", 3, 2, 8, 2, 0, 0, 0)
    assert zlib.decompress(written[1][1]) == b"\x00" + rgb[:9] + b"\x00" + rgb[9:]


@pytest.mark.parametrize(
    ("width", "height", "pixels"), [(0, 2, b""), (3, 2, bytes(17)), (2, 0, b"")]
)
def test_pixels_that_are_no_picture_are_refused(width: int, height: int, pixels: bytes) -> None:
    with pytest.raises(ValueError, match="are not a picture"):
        png.encode(width, height, pixels)
    with pytest.raises(ValueError, match="are not a picture"):
        png.shrunk(width, height, pixels, (1, 1))


def test_a_picture_is_made_smaller_by_its_nearest_pixels() -> None:
    def pixel(row: int, column: int) -> bytes:
        return bytes((row, column, row * 4 + column))

    rgb = b"".join(pixel(row, column) for row in range(4) for column in range(4))

    assert png.shrunk(4, 4, rgb, (4, 4)) is rgb
    assert png.shrunk(4, 4, rgb, (2, 2)) == pixel(0, 0) + pixel(0, 2) + pixel(2, 0) + pixel(2, 2)
    assert png.shrunk(4, 4, rgb, (1, 3)) == pixel(0, 0) + pixel(1, 0) + pixel(2, 0)
    with pytest.raises(ValueError, match="are not a picture"):
        png.shrunk(4, 4, rgb, (0, 2))


# Which desktop


@pytest.mark.parametrize(
    ("env", "why"),
    [
        ({"WAYLAND_DISPLAY": "wayland-0", "DISPLAY": ":0"}, "this is a session of Wayland"),
        ({"XDG_SESSION_TYPE": "wayland"}, "this is a session of Wayland"),
        ({}, "there is no display"),
        ({"DISPLAY": ""}, "there is no display"),
    ],
)
def test_a_session_that_is_not_reached_says_why(env: dict[str, str], why: str) -> None:
    found = desktop._on_linux(env)  # pyright: ignore[reportPrivateUsage]

    assert isinstance(found, str)
    assert found.startswith(why)
