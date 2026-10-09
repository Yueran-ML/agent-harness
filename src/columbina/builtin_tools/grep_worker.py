"""The process that looks for a pattern in the lines of files, for the grep tool.

A pattern that a model writes can make the matcher of Python run for years,
and while it runs it lets no other thread of its process do anything. So the
matching is done here, in a process of its own that can be killed, and the
agent stays awake.

This file is run by its path, with nothing of the package loaded. It needs
only what comes with Python, and it touches no file: what it is to search
arrives on its input.

On its input it reads one line of JSON, the request::

    {"pattern": "...", "ignore_case": false, "max_matches": 250, "max_line": 200}

and then, for every file, one line of JSON followed by the bytes of the file::

    {"id": 7, "size": 1234}

On its output it writes one line of JSON for every line that matches, with
the number of the line, its beginning, and how many characters were cut off::

    {"id": 7, "line": 12, "text": "...", "more": 0}

and at the end one line that says how many files it has searched, and whether
it stopped early because there were more matches than it was to report::

    {"done": true, "files": 31, "cut": false}

A pattern that is no pattern is answered with ``{"error": "..."}`` and the
exit code 2. Input that ends in the middle of a file is answered with the
exit code 1 and no last line. Whoever asked can tell from the last line
whether all it sent was searched.

A line ends at a line feed. A carriage return before the line feed belongs to
the line break. That is how the read tool counts lines too, so a number that
is found here is the number to read from there.
"""

import json
import re
import sys
import warnings
from typing import BinaryIO


def main(source: BinaryIO, sink: BinaryIO) -> int:
    """Answer one request from ``source`` on ``sink``, and return the exit code."""

    def say(message: dict[str, object]) -> None:
        sink.write(json.dumps(message).encode("ascii") + b"\n")

    request = json.loads(source.readline())
    try:
        with warnings.catch_warnings():
            # A warning about the pattern would land among the answers.
            warnings.simplefilter("ignore")
            pattern = re.compile(request["pattern"], re.IGNORECASE if request["ignore_case"] else 0)
    except Exception as error:
        say({"error": str(error) or type(error).__name__})
        return 2
    left: int = request["max_matches"]
    width: int = request["max_line"]
    files = 0
    cut = False
    while not cut:
        announcement = source.readline()
        if not announcement:
            break
        announced = json.loads(announcement)
        content = source.read(announced["size"])
        if len(content) < announced["size"]:
            # Whoever was sending has stopped in the middle of a file.
            return 1
        number = 0
        start = 0
        while start < len(content) and not cut:
            end = content.find(b"\n", start)
            if end < 0:
                end = len(content)
            # No byte of a character that takes several is a line feed, so a line can be
            # cut out before it is decoded.
            line = content[start:end].decode("utf-8", errors="replace").removesuffix("\r")
            start = end + 1
            number += 1
            if pattern.search(line) is None:
                continue
            if left == 0:
                cut = True
            else:
                left -= 1
                say(
                    {
                        "id": announced["id"],
                        "line": number,
                        "text": line[:width],
                        "more": max(0, len(line) - width),
                    }
                )
        if not cut:
            files += 1
    say({"done": True, "files": files, "cut": cut})
    return 0


if __name__ == "__main__":  # pragma: no cover - run in a process of its own
    raise SystemExit(main(sys.stdin.buffer, sys.stdout.buffer))
