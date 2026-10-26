"""A page of the web as text that a model reads.

A page is written in HTML for a browser: most of what it holds is markup,
scripts and styles, and a model that is given all of that pays for it and
finds the words in it with trouble. :func:`text_of` keeps what a reader sees,
in the shape of Markdown: headings with their marks, items of a list with a
dash, links with their addresses, code as it stands, and a blank line between
blocks. What a reader does not see is left out: scripts, styles, templates,
pictures drawn in SVG, forms and the head of the page but its title.

This is no browser. A page that writes its words with a script has few words
here, and the model is told how many it got, so it can tell.
"""

import re
from html.parser import HTMLParser
from urllib.parse import urljoin, urlsplit

__all__ = ["Page", "text_of"]

_UNSEEN = frozenset(
    {"script", "style", "noscript", "template", "svg", "head", "iframe", "select", "textarea"}
)
"""Elements whose content no reader sees as text. The title is taken from the head apart."""

_BLOCKS = frozenset(
    {
        "address", "article", "aside", "blockquote", "body", "dd", "details", "div", "dl",
        "dt", "fieldset", "figcaption", "figure", "footer", "form", "header", "hr", "html",
        "li", "main", "nav", "ol", "p", "section", "summary", "table", "tbody", "td", "tfoot",
        "th", "thead", "tr", "ul", "h1", "h2", "h3", "h4", "h5", "h6", "pre",
    }
)  # fmt: skip
"""Elements that stand on lines of their own."""

_VOID = frozenset(
    {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "wbr"}
)
"""Elements that have no end, and so hold nothing."""

_SPACES = re.compile(r"\s+")
_BLANK_LINES = re.compile(r"\n{3,}")
_SPACE_BEFORE_A_BREAK = re.compile(r"[ \t]+\n")

_LINKED = ("http", "https", "mailto")
"""The kinds of address that a link is written with. A script in a link is no address."""


class Page:
    """A page as text.

    Attributes:
        title: The title of the page, or an empty text.
        text: What a reader sees, in the shape of Markdown.
    """

    __slots__ = ("text", "title")

    def __init__(self, title: str, text: str) -> None:
        self.title = title
        self.text = text


class _Reader(HTMLParser):
    """Reads a page and keeps what a reader sees."""

    def __init__(self, address: str) -> None:
        super().__init__(convert_charrefs=True)
        self._address = address
        self._out: list[str] = []
        self._unseen = 0
        self._pre = 0
        self._code = 0
        self._in_title = False
        self._title: list[str] = []
        self._links: list[str | None] = []
        self._link_text: list[list[str]] = []
        self._lists: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "title":
            self._in_title = True
            return
        if tag in _UNSEEN:
            self._unseen += 1
            return
        if self._unseen:
            return
        if tag in _BLOCKS:
            self._break()
        if tag in ("h1", "h2", "h3", "h4", "h5", "h6"):
            self._write("#" * int(tag[1]) + " ")
        elif tag in ("ul", "ol"):
            self._lists.append(tag)
        elif tag == "li":
            depth = max(len(self._lists) - 1, 0)
            self._write("  " * depth + "- ")
        elif tag == "br":
            self._write("\n")
        elif tag == "hr":
            self._write("---")
            self._break()
        elif tag == "pre":
            self._pre += 1
            self._write("```\n")
        elif tag == "code" and not self._pre:
            self._code += 1
            self._write("`")
        elif tag == "a":
            href = dict(attrs).get("href")
            self._links.append(self._resolved(href))
            self._link_text.append([])
        elif tag == "img":
            alt = (dict(attrs).get("alt") or "").strip()
            if alt:
                self._write(f"[{_SPACES.sub(' ', alt)}]")

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.handle_starttag(tag, attrs)
        if tag not in _VOID:
            self.handle_endtag(tag)

    def handle_endtag(self, tag: str) -> None:
        if tag == "title":
            self._in_title = False
            return
        if tag in _UNSEEN:
            self._unseen = max(self._unseen - 1, 0)
            return
        if self._unseen:
            return
        if tag in ("ul", "ol") and self._lists:
            self._lists.pop()
        elif tag == "pre" and self._pre:
            self._pre -= 1
            self._write("\n```")
        elif tag == "code" and self._code and not self._pre:
            self._code -= 1
            self._write("`")
        elif tag == "a" and self._links:
            href = self._links.pop()
            words = _SPACES.sub(" ", "".join(self._link_text.pop())).strip()
            if href is not None and words and words != href:
                self._write(f"[{words}]({href})")
            else:
                self._write(words or (href or ""))
        if tag in _BLOCKS:
            self._break()

    def handle_data(self, data: str) -> None:
        if self._in_title:
            self._title.append(data)
            return
        if self._unseen:
            return
        self._write(data if self._pre else _SPACES.sub(" ", data))

    def _resolved(self, href: str | None) -> str | None:
        if not href:
            return None
        address = urljoin(self._address, href.strip())
        return address if urlsplit(address).scheme in _LINKED else None

    def _write(self, text: str) -> None:
        if self._link_text:
            self._link_text[-1].append(text)
        else:
            self._out.append(text)

    def _break(self) -> None:
        # A link stands on one line, however its words are laid out.
        if self._link_text:
            self._link_text[-1].append(" ")
        else:
            self._out.append("\n\n")

    def page(self) -> Page:
        title = _SPACES.sub(" ", "".join(self._title)).strip()
        lines = "".join(self._out).split("\n")
        text = "\n".join(
            line.strip(" ") if not line.startswith("  ") else line.rstrip() for line in lines
        )
        text = _SPACE_BEFORE_A_BREAK.sub("\n", text)
        text = _BLANK_LINES.sub("\n\n", text).strip()
        return Page(title, text)


def text_of(html: str, address: str) -> Page:
    """Return what a reader sees of a page, and its title.

    Args:
        html: The page.
        address: Where the page is, so that a link that is written from it
            can be written in full.
    """
    reader = _Reader(address)
    reader.feed(html)
    reader.close()
    return reader.page()
