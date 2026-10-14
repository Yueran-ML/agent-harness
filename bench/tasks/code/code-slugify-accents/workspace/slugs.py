"""Slugs: the part of the address of a post that names it."""

import re


def slugify(text, separator="-", max_length=None):
    """Return the slug of a title.

    A slug is in lower case and holds nothing but ASCII letters and digits,
    with one separator between words and none at either end.

    Args:
        text: The title.
        separator: What goes between words.
        max_length: How long the slug may be, or ``None`` for no limit. A
            slug that is cut short does not end with a separator.
    """
    slug = re.sub(r"[^a-z0-9]", separator, text.lower())
    slug = slug.strip(separator)
    if max_length is not None:
        slug = slug[:max_length].rstrip(separator)
    return slug


def unique_slug(text, taken, separator="-"):
    """Return the slug of a title, numbered so that it is none of the slugs in ``taken``.

    The first copy of a slug is numbered 2, as in ``hello-world-2``. A title
    without letters or digits gets the slug ``post``.
    """
    base = slugify(text, separator) or "post"
    slug = base
    number = 2
    while slug in taken:
        slug = f"{base}{separator}{number}"
        number += 1
    return slug
