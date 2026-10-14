"""Wrapping text to a fixed width, for the notes printed at the foot of a receipt."""


def wrap(text: str, width: int) -> list[str]:
    """Break `text` into lines of at most `width` characters.

    Lines break at spaces, and each line holds as many words as fit on it.
    Any run of whitespace, line breaks included, counts as one space. A word
    longer than `width` is cut into pieces of `width` characters, and the
    last piece may be followed by more words. Text without words gives no
    lines.
    """
    if width < 1:
        raise ValueError("width must be at least 1")
    text = " ".join(text.split())
    lines = []
    while len(text) > width:
        cut = text.rfind(" ", 0, width + 1)
        if cut == -1:
            lines.append(text[:width])
            text = text[width:].lstrip()
        else:
            lines.append(text[:cut])
            text = text[cut + 1 :]
    if text:
        lines.append(text)
    return lines


def fill(text: str, width: int) -> str:
    """Wrap `text` as `wrap` does, and join the lines with newlines."""
    return "\n".join(wrap(text, width))


def wrap_paragraphs(text: str, width: int) -> list[str]:
    """Wrap each paragraph of `text` on its own, with an empty line between paragraphs.

    Paragraphs are separated by lines that are empty or hold only whitespace.
    """
    paragraphs: list[str] = []
    current: list[str] = []
    for line in text.splitlines():
        if line.strip():
            current.append(line)
        elif current:
            paragraphs.append(" ".join(current))
            current = []
    if current:
        paragraphs.append(" ".join(current))
    lines: list[str] = []
    for paragraph in paragraphs:
        if lines:
            lines.append("")
        lines.extend(wrap(paragraph, width))
    return lines
