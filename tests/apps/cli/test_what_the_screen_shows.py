"""What stands on the screen while somebody is at the prompt.

The other tests of the prompt look at the text that was printed or drawn. These
look at the screen itself, the way a person does: which line stands where, what
is left when something was written over, and what stays when a line was
submitted. The screen is that of a terminal that is emulated.
"""

import asyncio

from rich.text import Text

from columbina.apps.cli.prompt import Prompt
from support.terminal import CTRL_D, Desk, desk, stands, until

STAND_IN = chr(0xFFFD)
WIDE = chr(0x4E2D) + chr(0x6587)


def status() -> str:
    return "the status"


async def typed(prompt: Prompt, at: Desk, text: str) -> None:
    """Type something and wait until the line holds it."""
    at.type(text)
    await until(lambda: prompt.typed.endswith(text))


# The prompt and the line below it


async def test_the_prompt_stands_in_the_first_free_row_and_the_status_in_the_last() -> None:
    with desk(rows=6, columns=40) as at:
        prompt = Prompt(at.terminal, status=status)
        waiting = asyncio.ensure_future(prompt.line())

        await typed(prompt, at, "abc")

        await stands(at.window, ["> abc", "", "", "", "", "the status"])
        assert at.cursor() == (0, 5)
        at.enter()
        assert await asyncio.wait_for(waiting, 5) == "abc"
        # The line that was submitted stays. The line below the prompt does not.
        assert at.screen() == ["> abc"]
        assert at.cursor() == (1, 0)


async def test_what_is_printed_goes_above_the_prompt_and_the_typing_stays() -> None:
    with desk(rows=6, columns=40) as at:
        prompt = Prompt(at.terminal, status=status)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "ab")

        prompt.show([Text("one"), Text("two")])
        await stands(at.window, ["one", "two", "> ab", "", "", "the status"])
        prompt.show([Text("three")])
        await stands(at.window, ["one", "two", "three", "> ab", "", "the status"])

        assert at.cursor() == (3, 4)
        at.enter("c")
        assert await asyncio.wait_for(waiting, 5) == "abc"
        assert at.screen() == ["one", "two", "three", "> abc"]


async def test_when_the_window_is_full_the_oldest_lines_scroll_away() -> None:
    with desk(rows=5, columns=40) as at:
        prompt = Prompt(at.terminal, status=status)
        waiting = asyncio.ensure_future(prompt.line())

        prompt.show([Text(f"line {number}") for number in range(1, 9)])

        await stands(at.window, ["line 6", "line 7", "line 8", ">", "the status"])
        assert at.screen()[:8] == [f"line {number}" for number in range(1, 9)]
        at.type(CTRL_D)
        assert await asyncio.wait_for(waiting, 5) is None


async def test_a_line_that_is_longer_than_the_window_is_broken_by_the_terminal() -> None:
    piece = "abcdefghij"
    with desk(rows=8, columns=20) as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())

        prompt.show([Text(piece * 5), Text("next")])

        await stands(lambda: at.window()[:5], [piece * 2, piece * 2, piece, "next", ">"])
        at.type(CTRL_D)
        await asyncio.wait_for(waiting, 5)


# The line of a reply that is not complete


async def test_the_unfinished_line_stands_above_the_typing_and_grows_in_its_place() -> None:
    with desk(rows=6, columns=40) as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "ab")

        prompt.partial = Text("The answer")
        await stands(lambda: at.window()[:3], ["The answer", "> ab", ""])
        prompt.partial = Text("The answer is 42")
        await stands(lambda: at.window()[:3], ["The answer is 42", "> ab", ""])
        on_the_typing = at.cursor()
        # The line is complete now. It is printed, and the place above the typing is free.
        prompt.partial = Text()
        prompt.show([Text("The answer is 42.")])
        await stands(lambda: at.window()[:3], ["The answer is 42.", "> ab", ""])

        assert on_the_typing == (1, 4)
        assert at.cursor() == (1, 4)
        at.enter()
        await asyncio.wait_for(waiting, 5)
        assert at.screen() == ["The answer is 42.", "> ab"]


async def test_a_line_that_is_submitted_stays_without_the_half_line_above_it() -> None:
    with desk(rows=6, columns=40) as at:
        prompt = Prompt(at.terminal, status=status)
        waiting = asyncio.ensure_future(prompt.line())
        prompt.partial = Text("The answer is")
        await typed(prompt, at, "wait")
        await stands(lambda: at.window()[:2], ["The answer is", "> wait"])

        at.enter()

        assert await asyncio.wait_for(waiting, 5) == "wait"
        assert at.screen() == ["> wait"]
        # The next prompt has the half line again, below the line that was submitted.
        waiting = asyncio.ensure_future(prompt.line())
        await stands(at.window, ["> wait", "The answer is", ">", "", "", "the status"])
        prompt.partial = Text()
        prompt.show([Text("The answer is 42.")])
        await stands(lambda: at.window()[:3], ["> wait", "The answer is 42.", ">"])
        at.type(CTRL_D)
        await asyncio.wait_for(waiting, 5)
        assert at.screen() == ["> wait", "The answer is 42.", ">"]


async def test_a_long_unfinished_line_takes_one_row_and_shows_its_end() -> None:
    growing = "start " + "word " * 10 + "the end"
    with desk(rows=6, columns=20) as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())

        prompt.partial = Text(growing)

        # One cell stays free at the end of the row, and three points stand for what is not shown.
        await stands(lambda: at.window()[:2], ["..." + growing[-16:], ">"])
        at.type(CTRL_D)
        await asyncio.wait_for(waiting, 5)


# A question


async def test_a_question_takes_the_place_of_the_mark_and_gives_the_typing_back() -> None:
    with desk(rows=8, columns=60) as at:
        prompt = Prompt(at.terminal, status=status)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "half a thought")

        asking = asyncio.ensure_future(
            prompt.ask("yan", "[y/a/n]", [Text("May I?"), Text("  y  yes")])
        )
        await stands(lambda: at.window()[:4], ["May I?", "  y  yes", "[y/a/n]", ""])
        asked_at = at.cursor()
        at.type("x")
        at.type("y")

        assert await asyncio.wait_for(asking, 5) == "y"
        await stands(lambda: at.window()[:4], ["May I?", "  y  yes", "> half a thought", ""])
        assert asked_at == (2, 8)
        assert at.window()[-1] == "the status"
        at.enter()
        assert await asyncio.wait_for(waiting, 5) == "half a thought"


# Text


async def test_wide_characters_take_two_cells_and_the_cursor_knows_it() -> None:
    with desk(rows=6, columns=40) as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())

        prompt.show([Text(WIDE * 2)])
        await typed(prompt, at, f"{WIDE}a")

        await stands(lambda: at.window()[:2], [WIDE * 2, f"> {WIDE}a"])
        assert at.cursor() == (1, 7)
        at.enter()
        assert await asyncio.wait_for(waiting, 5) == f"{WIDE}a"


async def test_text_that_would_steer_a_terminal_leaves_the_screen_as_it_is() -> None:
    clear, up_and_erase = "\x1b[2J", "\x1b[1A\x1b[2K"
    with desk(rows=8, columns=60) as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        prompt.show([Text("python check.py asks for a yes"), Text("(not allowed)")])

        # The first sequence stands between the marks that make a library hand it on as it is.
        prompt.show([Text(f"three\x01{clear}\x02{up_and_erase}(allowed)")])
        prompt.partial = Text(f"four{clear}")

        s = STAND_IN
        await stands(
            lambda: at.window()[:5],
            [
                "python check.py asks for a yes",
                "(not allowed)",
                f"three{s}{s}[2J{s}{s}[1A{s}[2K(allowed)",
                f"four{s}[2J",
                ">",
            ],
        )
        at.type(CTRL_D)
        await asyncio.wait_for(waiting, 5)


async def test_the_line_below_the_prompt_is_drawn_anew_when_something_begins_to_run() -> None:
    said = ["at rest"]
    with desk(rows=4, columns=40) as at:
        prompt = Prompt(at.terminal, status=lambda: said[0])
        waiting = asyncio.ensure_future(prompt.line())
        await stands(lambda: at.window()[-1:], ["at rest"])

        # Nothing is typed and nothing is printed. That something runs is reason enough.
        said[0] = "running | esc stops"
        prompt.busy = True

        await stands(lambda: at.window()[-1:], ["running | esc stops"])
        at.type(CTRL_D)
        await asyncio.wait_for(waiting, 5)
