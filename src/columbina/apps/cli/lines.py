"""Text that arrives in pieces, taken apart into its complete lines and the one that is not.

A reply comes a few characters at a time. What was printed above the prompt
cannot be changed any more, so only a line that is complete is printed there.
The line that is still being written is shown apart, and shown anew each time
it grows.
"""

__all__ = ["Lines"]


class Lines:
    """The lines of a text that is still arriving.

    Attributes:
        partial: The line that is not complete yet.
    """

    __slots__ = ("partial",)

    def __init__(self) -> None:
        self.partial = ""

    def feed(self, text: str) -> list[str]:
        """Take a piece of the text and return the lines that it completes.

        A line ends with a line feed. A carriage return right before it
        belongs to the line break, also when the two come in different
        pieces.
        """
        *complete, self.partial = (self.partial + text).split("\n")
        return [line.removesuffix("\r") for line in complete]

    def flush(self) -> str | None:
        """Hand over the line that is not complete, if there is one, as the last line."""
        line, self.partial = self.partial.removesuffix("\r"), ""
        return line or None
