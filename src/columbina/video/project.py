"""Source identity and atomic writes for the engine's derived analysis files.

The engine owns the project and its undo history. Python never writes project.json.
The bounded source digest uses the engine's identity algorithm so analysis is reused only
for matching media; it is not an independent project revision or history mechanism.
"""

import hashlib
import os
import secrets
from pathlib import Path

from columbina.data import Data

_EDGE = 64 * 1024


class ProjectError(Exception):
    """Something of a project cannot be done. The words say why, for a person."""


class Identity(Data):
    """What tells a source from another file of the same name.

    Attributes:
        size: Its size in bytes.
        modified_ns: When it was changed last.
        digest: SHA-256 of its size, its first and its last 64 KiB.
    """

    size: int
    modified_ns: int
    digest: str


def identity_of(path: Path) -> Identity:
    """Work out the identity of a file. This blocks.

    Raises:
        OSError: It cannot be read.
    """
    with path.open("rb") as file:
        size = os.fstat(file.fileno()).st_size
        stamp = os.fstat(file.fileno()).st_mtime_ns
        digest = hashlib.sha256(str(size).encode())
        digest.update(file.read(_EDGE))
        if size > _EDGE:
            file.seek(max(_EDGE, size - _EDGE))
            digest.update(file.read(_EDGE))
    return Identity(size=size, modified_ns=stamp, digest=digest.hexdigest())


def written(path: Path, text: str) -> None:
    """Write a file beside itself first, and put it in its place. This blocks.

    Raises:
        ProjectError: It could not be written, or put in its place.
    """
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(4)}")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary.write_text(text, encoding="utf-8", newline="\n")
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise ProjectError(f"{path} could not be written: {error}") from None
