"""API keys: what one looks like.

A key reaches a service as part of a header of the request. What is found
where a key is kept is not always a key as its service issued it: it was
copied by hand, and blanks, a line break or a mark of the text that it was
copied from may have come along. The two questions here are asked of every
key before it is sent.
"""

__all__ = ["sendable", "trimmed"]


def trimmed(key: str) -> str:
    """Return a key without the blanks and line breaks around it.

    They are no part of any key, and they come along easily: a line break at
    the end of a file that holds the key, or a blank at the end of the line
    that sets a variable.
    """
    return key.strip()


def sendable(key: str) -> bool:
    """Whether a key can be sent as it is.

    A key that can is made of the visible characters of ASCII and of nothing
    else. That is all that the header of a request carries, and all that the
    keys of the services are made of. Anything else in a key shows that it
    was not copied whole or not copied alone: a blank or a line break in the
    middle, a quotation mark of a word processor, a mark that cannot be seen.

    Args:
        key: The key, already trimmed.
    """
    return bool(key) and all("!" <= character <= "~" for character in key)
