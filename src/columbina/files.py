"""Files that are written whole or not at all.

A file is written beside its place and then moved there, so that whoever reads it finds the
one before or the one after, and never half of one. ``apps/setup.py``, ``apps/hooks.py`` and
``llm/credentials.py`` write their files this way too, each with words of its own for what
went wrong.
"""

import os
import secrets
from pathlib import Path

__all__ = ["replaced"]


def replaced(path: Path, content: str | bytes, *, private: bool = False) -> None:
    """Put a file of this content in the place of the one that is there, whole or not at all.

    Args:
        path: The file. Where it is a link, the file that it leads to is
            written: put in the place of the link, the new file would leave
            the old one lying where it was.
        content: What the file is to hold: text, which is written as UTF-8
            with line feeds, or bytes.
        private: Whether the file is made for its user alone, with the mode
            600 where files have modes, before anything is written into it.

    Raises:
        OSError: The file cannot be written. The one that was there is as it
            was.
    """
    target = path.resolve()
    beside = target.with_name(f".{target.name}.{secrets.token_hex(8)}.tmp")
    target.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(beside, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600 if private else 0o666)
    try:
        with os.fdopen(descriptor, "wb") as file:
            file.write(content.encode() if isinstance(content, str) else content)
        beside.replace(target)
    finally:
        beside.unlink(missing_ok=True)
