"""Markdown tables that line up when they are read as plain text too."""

MIN_WIDTH = 3
"""The narrowest a column can be: wide enough for the ``---`` of its rule."""

_RULES = {
    None: lambda width: "-" * width,
    "left": lambda width: ":" + "-" * (width - 1),
    "center": lambda width: ":" + "-" * (width - 2) + ":",
    "right": lambda width: "-" * (width - 1) + ":",
}

_PADDING = {
    None: str.ljust,
    "left": str.ljust,
    "center": str.center,
    "right": str.rjust,
}


def cell_text(value):
    """Return a value as a cell shows it: with ``|`` escaped and line breaks as spaces."""
    return str(value).replace("|", "\\|").replace("\r\n", " ").replace("\n", " ")


def render_table(headers, rows, align=None):
    """Return a Markdown table, each of its lines ending with a line break.

    Every column is as wide as the widest text in it, so that the table lines
    up when it is read as plain text.

    Args:
        headers: The titles of the columns.
        rows: The rows, each with one value for each column.
        align: For each column ``"left"``, ``"center"``, ``"right"`` or
            ``None``; or ``None`` to align no column.

    Raises:
        ValueError: A row or ``align`` does not have one entry for each column.
    """
    count = len(headers)
    rows = [list(row) for row in rows]
    align = [None] * count if align is None else list(align)
    if len(align) != count:
        raise ValueError(f"align has {len(align)} entries for {count} columns")
    for row in rows:
        if len(row) != count:
            raise ValueError(f"a row has {len(row)} values for {count} columns")
    widths = [MIN_WIDTH] * count
    for row in rows:
        for index, value in enumerate(row):
            widths[index] = max(widths[index], len(str(value)))
    lines = [_line(headers, widths, align)]
    lines.append("| " + " | ".join(_RULES[a](w) for a, w in zip(align, widths)) + " |")
    lines.extend(_line(row, widths, align) for row in rows)
    return "\n".join(lines) + "\n"


def _line(values, widths, align):
    """Return the line of the table that holds these values."""
    cells = (_PADDING[a](cell_text(v), w) for v, w, a in zip(values, widths, align))
    return "| " + " | ".join(cells) + " |"
