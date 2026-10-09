"""The line at the bottom of the terminal, and the place above it where things are shown.

One prompt stays open for as long as somebody is at the terminal. Whatever
is shown is printed above it, and the prompt is drawn again below, with what
the user had typed still in it. ``prompt_toolkit`` does the drawing, and it
alone moves the cursor.

The prompt serves three things at once.

- A line that the user submits is handed to whoever waits for one. What the
  line means is not decided here: a task, something said to a running agent,
  a command.
- The line of a reply that is not complete yet is drawn above the place where
  the user types, and drawn anew as it grows. Only a complete line is printed
  for good, because what was printed cannot be changed.
- A question is answered by one key. While it is asked, what the user had
  typed is put aside and comes back afterwards. Keys cannot be shut out one
  by one, since each has a meaning of its own in the line editor, so the
  line editor is given an empty line to work on instead, and the keys that
  write are swallowed.
"""

import asyncio
from collections.abc import Callable, Sequence

from prompt_toolkit import PromptSession, print_formatted_text
from prompt_toolkit.filters import Condition
from prompt_toolkit.formatted_text import ANSI, StyleAndTextTuples
from prompt_toolkit.key_binding import KeyBindings, KeyPressEvent
from prompt_toolkit.keys import Keys
from prompt_toolkit.utils import get_cwidth
from rich.text import Text

from columbina.apps.cli.terminal import Terminal
from columbina.apps.cli.transcript import on_screen

__all__ = ["Prompt"]

_REDRAW_SECONDS = 0.05
"""The least time between two drawings of the prompt, however fast a reply grows."""

_ESCAPE_SECONDS = 0.1
"""How long Escape waits for a key that would make it the beginning of another key."""

_MARK = "> "
_CUT = "..."
_QUIET = "fg:ansibrightblack"


def _end_of(text: str, cells: int) -> str:
    """Return a text if it fits into so many cells of the screen, and its end if it does not."""
    if get_cwidth(text) <= cells:
        return text
    room = cells - len(_CUT)
    kept = len(text)
    # The text is wider than the room, so this stops before its beginning.
    while get_cwidth(text[kept - 1]) <= room:
        kept -= 1
        room -= get_cwidth(text[kept])
    return _CUT + text[kept:]


class Prompt:
    """The place where the user types, and where everything else is shown.

    Attributes:
        on_cancel: What Escape and Ctrl+C do while something is running.
        status: Says what the line below the prompt holds.
    """

    __slots__ = (
        "_answer",
        "_asking",
        "_busy",
        "_choices",
        "_keys",
        "_leaving",
        "_partial",
        "_session",
        "_terminal",
        "on_cancel",
        "status",
    )

    def __init__(self, terminal: Terminal, *, status: Callable[[], str] | None = None) -> None:
        self._terminal = terminal
        self.status: Callable[[], str] = status or str
        self.on_cancel: Callable[[], None] | None = None
        self._busy = False
        self._partial = Text()
        self._answer: asyncio.Future[str] | None = None
        self._keys = ""
        self._choices = ""
        self._asking = asyncio.Lock()
        self._leaving = False
        self._session: PromptSession[str | None] = PromptSession(
            message=self._message,
            input=terminal.input,
            output=terminal.output,
            key_bindings=self._bindings(),
            bottom_toolbar=lambda: self.status(),
        )
        self._session.app.min_redraw_interval = _REDRAW_SECONDS
        # Escape is also how a terminal begins the keys that are pressed with Alt. A key that
        # follows it at once belongs to it. Escape by itself counts after this long.
        self._session.app.ttimeoutlen = _ESCAPE_SECONDS
        self._session.app.timeoutlen = _ESCAPE_SECONDS
        self._session.default_buffer.on_text_changed += self._stays

    @property
    def busy(self) -> bool:
        """Whether something is running that Escape would stop."""
        return self._busy

    @busy.setter
    def busy(self, running: bool) -> None:
        self._busy = running
        # The line below the prompt says so.
        self._redraw()

    @property
    def typed(self) -> str:
        """What the line holds at this moment."""
        return self._session.default_buffer.text

    @property
    def partial(self) -> Text:
        """The line of a reply that is not complete yet. It is drawn above the typing."""
        return self._partial

    @partial.setter
    def partial(self, text: Text) -> None:
        self._partial = text
        self._redraw()

    async def line(self) -> str | None:
        """Wait for the user to submit a line, and return it as it was typed.

        Returns:
            The line, or ``None`` if the user wants to leave, or if nothing
            more can be typed because the input has come to its end.
        """
        self._leaving = False
        try:
            return await self._session.prompt_async()
        except EOFError:
            return None

    async def ask(self, keys: str, choices: str, about: Sequence[Text] = ()) -> str:
        """Ask for one key, and return the key that was pressed.

        Escape, Ctrl+C and Ctrl+D count as the last of the keys, which is
        where the answer that changes nothing belongs. Whatever else is
        pressed is no answer. Questions are asked one at a time, and the
        words of a question are shown when its turn has come. By then the
        prompt takes keys for answers, so whoever reads the question and
        presses a key answers this question and no other.

        Args:
            keys: The keys that are answers, in lower case.
            choices: What stands in the place of the prompt while the
                question is asked.
            about: The question in words, shown above the prompt.
        """
        async with self._asking:
            buffer = self._session.default_buffer
            aside = buffer.document
            buffer.reset()
            self._keys, self._choices = keys, choices
            self._answer = asyncio.get_running_loop().create_future()
            self.show(about)
            self._redraw()
            try:
                return await self._answer
            finally:
                self._answer = None
                buffer.set_document(aside, bypass_readonly=True)
                self._redraw()

    def show(self, lines: Sequence[Text]) -> None:
        """Print lines above the prompt. They stay as they are from then on."""
        sent = on_screen(lines, colours=self._terminal.colours)
        if sent:
            # While the prompt is open, this prints above it and draws it again below.
            print_formatted_text(ANSI(sent), output=self._terminal.output, end="")

    def _message(self) -> StyleAndTextTuples:
        """Return what stands before the typing: the unfinished line, and the mark below it."""
        parts: StyleAndTextTuples = []
        # A line that was submitted is drawn once more, the way it stays on the screen. The
        # unfinished line is left out then: it grows on in the next prompt and is printed
        # when it is complete, and it would stand on the screen twice.
        if self._partial.plain and not self._session.app.is_done:
            columns = self._terminal.output.get_size().columns
            quiet = _QUIET if self._partial.style and self._terminal.colours else ""
            parts.append((quiet, _end_of(self._partial.plain, columns - 1) + "\n"))
        parts.append(("", f"{self._choices} " if self._answer is not None else _MARK))
        return parts

    def _redraw(self) -> None:
        if self._session.app.is_running:
            self._session.app.invalidate()

    def _stays(self, _: object) -> None:
        # Somebody who types has not made up their mind to leave.
        self._leaving = False

    def _answered(self, key: str) -> None:
        if self._answer is not None and not self._answer.done():
            self._answer.set_result(key)

    def _cancel(self) -> None:
        if self.on_cancel is not None:
            self.on_cancel()

    def _bindings(self) -> KeyBindings:
        bindings = KeyBindings()
        asking = Condition(lambda: self._answer is not None)
        running = Condition(lambda: self.busy and self._answer is None)
        at_rest_and_empty = Condition(lambda: self._answer is None and not self.typed)

        @bindings.add("escape", filter=running)
        def stop(event: KeyPressEvent) -> None:
            self._cancel()

        @bindings.add("escape", filter=asking)
        @bindings.add("c-d", filter=asking)
        def decline(event: KeyPressEvent) -> None:
            self._answered(self._keys[-1])

        @bindings.add(Keys.Any, filter=asking)
        def answer(event: KeyPressEvent) -> None:
            if event.data.lower() in self._keys:
                self._answered(event.data.lower())

        @bindings.add("enter", filter=asking)
        def no_line(event: KeyPressEvent) -> None:
            """Take a line for no answer: the question stays."""

        @bindings.add("c-c")
        def interrupt(event: KeyPressEvent) -> None:
            if self._answer is not None:
                self._answered(self._keys[-1])
            elif self.busy:
                self._cancel()
            elif event.current_buffer.text:
                event.current_buffer.reset()
            elif self._leaving:
                event.app.exit(result=None)
            else:
                self._leaving = True
                self.show([Text("Press Ctrl+C again to leave.", style="dim")])

        @bindings.add("c-d", filter=at_rest_and_empty)
        def leave(event: KeyPressEvent) -> None:
            event.app.exit(result=None)

        return bindings
