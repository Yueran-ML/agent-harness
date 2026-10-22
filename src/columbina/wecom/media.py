"""The files that are sent to the bot: downloaded, and decrypted.

WeCom names a file that was sent with a link, which holds for five minutes, and a key that is
the file's own. What lies at the link is the file encrypted with AES-256 in CBC, under that key
as Base64, with the first sixteen bytes of the key as the vector, and filled up to a whole
number of 32 bytes in the way of PKCS #7. The message does not name the file. Where the answer
of the download does, in its ``Content-Disposition``, that is its name.
"""

import base64
import binascii
import re
from dataclasses import dataclass
from http import HTTPStatus
from urllib.parse import unquote

import httpx

from columbina.wecom.app import WecomError, safe_to_fetch

__all__ = ["Fetched", "decrypted", "fetched", "named"]

_BLOCK = 32
"""The size of a block that a file is filled up to before it is encrypted."""

_STARRED = re.compile(r"filename\*\s*=\s*UTF-8''([^;\s]+)", re.IGNORECASE)
_PLAIN = re.compile(r'filename\s*=\s*"?([^";]+)"?', re.IGNORECASE)


@dataclass(frozen=True, slots=True)
class Fetched:
    """What was downloaded: the bytes as they came, and the name of the file where it was given."""

    content: bytes
    name: str | None


def named(disposition: str | None) -> str | None:
    """Return the name of a file that the ``Content-Disposition`` of an answer gives, if any."""
    if not disposition:
        return None
    for pattern in (_STARRED, _PLAIN):
        found = pattern.search(disposition)
        if found is not None:
            name = unquote(found.group(1).strip(), errors="replace")
            if name:
                return name
    return None


async def fetched(http: httpx.AsyncClient, url: str, *, most: int) -> Fetched:
    """Download what lies at a link that WeCom gave, at most ``most`` bytes of it.

    Raises:
        WecomError: The link may not be followed, the download failed, or what lies there is
            larger than ``most``.
    """
    if not safe_to_fetch(url):
        raise WecomError("WeCom named a file at a place that is not reached safely", code="UNSAFE")
    try:
        async with http.stream("GET", url) as response:
            if response.status_code != HTTPStatus.OK:
                raise WecomError(
                    f"the file could not be downloaded: HTTP {response.status_code}",
                    code="DOWNLOAD",
                )
            kept = bytearray()
            async for chunk in response.aiter_bytes():
                kept.extend(chunk)
                if len(kept) > most + _BLOCK:
                    raise WecomError(
                        f"the file is larger than {most} bytes, more than is taken",
                        code="TOO_LARGE",
                    )
            return Fetched(bytes(kept), named(response.headers.get("content-disposition")))
    except httpx.TimeoutException:
        raise WecomError("WeCom did not give the file in time", code="DOWNLOAD") from None
    except httpx.RequestError as error:
        raise WecomError(f"the file could not be downloaded: {error}", code="DOWNLOAD") from None


def decrypted(content: bytes, key: str) -> bytes:
    """Return a file that WeCom encrypted, as it was sent.

    Raises:
        WecomError: The key is no key, or the file was not encrypted with it.
    """
    # The cipher comes with columbina[wecom], which an assistant in WeCom needs.
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

    written = key.strip()
    try:
        raw = base64.b64decode(written + "=" * (-len(written) % 4), validate=True)
    except (binascii.Error, ValueError):
        raw = b""
    if len(raw) != 32:
        raise WecomError("the key of the file is no key of AES-256", code="UNREADABLE")
    if not content or len(content) % 16:
        raise WecomError("the file is not one that was encrypted with its key", code="UNREADABLE")
    opened = Cipher(algorithms.AES(raw), modes.CBC(raw[:16])).decryptor()
    plain = opened.update(content) + opened.finalize()
    filled = plain[-1]
    if (
        not 1 <= filled <= _BLOCK
        or filled > len(plain)
        or plain[-filled:] != bytes([filled]) * filled
    ):
        raise WecomError("the file is not one that was encrypted with its key", code="UNREADABLE")
    return plain[:-filled]
