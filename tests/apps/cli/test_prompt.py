"""The line at the bottom of the terminal: what is typed there, and what is shown above it."""

import asyncio
import time

import pytest
from prompt_toolkit.keys import Keys
from rich.text import Text

from columbina.apps.cli import prompt as prompt_module
from columbina.apps.cli.prompt import Prompt
from support.terminal import CTRL_C, CTRL_D, ENTER, ESCAPE, Desk, desk, stands, until

STAND_IN = chr(0xFFFD)
OVERRIDE = chr(0x202E)


async def typed(prompt: Prompt, at: Desk, text: str) -> None:
    """Type something and wait until the line holds it."""
    at.type(text)
    await until(lambda: prompt.typed.endswith(text))


async def offered(at: Desk, mark: str) -> None:
    """Wait until something stands where the prompt is, as the keys of a question do.

    This reads the screen. What is sent to draw a line leaves out the places
    that hold the same as before, so it need not hold the line in one piece.
    """
    await until(lambda: at_the_prompt(at, mark))


def at_the_prompt(at: Desk, mark: str) -> bool:
    """Tell whether something stands where the prompt is at this moment."""
    return any(row.startswith(mark) for row in at.window())


# Lines


async def test_a_line_that_is_submitted_is_handed_over_as_it_was_typed() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        at.enter("  What does notes.txt say? ")

        assert await asyncio.wait_for(prompt.line(), 5) == "  What does notes.txt say? "


async def test_lines_are_handed_over_one_after_the_other() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        at.enter("one")
        at.enter("")
        at.enter("three")

        lines = [await asyncio.wait_for(prompt.line(), 5) for _ in range(3)]

    assert lines == ["one", "", "three"]


async def test_what_is_shown_appears_on_the_screen_and_leaves_the_typing_alone() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "ha")

        prompt.show([Text("The file says 42."), Text("It is short.")])
        await until(lambda: "It is short." in at.shown())
        at.enter("lf")

        assert await asyncio.wait_for(waiting, 5) == "half"
        assert "The file says 42.\nIt is short." in at.shown().replace("\r\n", "\n")


async def test_something_can_be_shown_while_nobody_is_asked_for_a_line() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)

        prompt.show([Text("Before the first line.")])

        assert "Before the first line." in at.shown()


async def test_what_is_shown_cannot_steer_the_terminal() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)

        # Between these two marks a library that reads sequences hands text on as it is.
        prompt.show([Text("a\x01\x1b[2J\x02b\x1b]52;c;aGk=\x07c\x1b[31md")])

        assert at.seen() == (
            f"a{STAND_IN}{STAND_IN}[2J{STAND_IN}b{STAND_IN}]52;c;aGk=c{STAND_IN}[31md\n"
        )
        assert "\x1b" not in at.shown()


async def test_the_unfinished_line_cannot_steer_the_terminal_either() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())

        prompt.partial = Text(f"ab{OVERRIDE}cd\x1b[2Jef")
        await at.draws(f"ab{STAND_IN}cd{STAND_IN}[2Jef\n")
        at.enter()

        await asyncio.wait_for(waiting, 5)
        assert "\x1b" not in at.shown()
        assert OVERRIDE not in at.shown()


async def test_what_is_shown_has_colours_only_on_a_terminal_that_wants_them() -> None:
    red = "\x1b[0;31m"
    with desk() as plain, desk(colours=True) as coloured:
        for at in (plain, coloured):
            Prompt(at.terminal).show([Text("stop", style="red")])

        assert plain.seen() == coloured.seen() == "stop\n"
        assert red in coloured.sent()
        assert red not in plain.sent()


async def test_showing_nothing_shows_nothing() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)

        prompt.show([])

        assert at.shown() == ""


async def test_the_line_of_a_reply_that_is_not_complete_is_shown_with_the_prompt() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "x")

        prompt.partial = Text("The answer is")
        await until(lambda: "The answer is" in at.shown())
        prompt.partial = Text()
        at.enter()

        assert await asyncio.wait_for(waiting, 5) == "x"
        assert prompt.partial.plain == ""


async def test_a_line_that_is_submitted_leaves_no_half_of_a_reply_above_it() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        prompt.partial = Text("The answer is")
        await at.draws("The answer is")
        await typed(prompt, at, "wait")
        await at.draws("wait")
        before = len(at.drawn())

        at.enter()

        assert await asyncio.wait_for(waiting, 5) == "wait"
        # A line that is submitted is drawn once more, the way it stays on the screen. The
        # half of the reply goes on growing in the next prompt and is printed when it is
        # complete, so it must not stay here as well.
        assert at.drawn()[before:] == "> wait\n"
        assert prompt.partial.plain == "The answer is"


async def test_of_a_very_long_unfinished_line_the_end_is_shown() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "x")

        prompt.partial = Text("start " + "word " * 40 + "the end")
        await until(lambda: "the end" in at.shown())
        at.enter()
        await asyncio.wait_for(waiting, 5)

        assert "start word" not in at.shown()


async def test_the_end_of_a_long_line_is_measured_in_the_cells_that_it_takes() -> None:
    moon = chr(0x6708)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "x")

        # The screen of a test is 80 cells wide, and this character takes two of them.
        prompt.partial = Text(moon * 60)
        await until(lambda: moon in at.shown())
        at.enter()
        await asyncio.wait_for(waiting, 5)

        assert "..." + moon * 38 in at.shown()
        assert moon * 39 not in at.shown()


async def test_a_reply_that_grows_fast_is_not_drawn_at_every_step(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # The time between two drawings is made long for this test. The clocks of some systems
    # are coarse, and with a twentieth of a second the count would be blurred.
    between = 0.5
    monkeypatch.setattr(prompt_module, "_REDRAW_SECONDS", between)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "x")

        before, began = at.drawings(), time.monotonic()
        for number in range(10):
            prompt.partial = Text(f"step {number} of the reply")
            await asyncio.sleep(0.04)
        await stands(lambda: at.window()[:1], ["step 9 of the reply"])
        drawn, took = at.drawings() - before, time.monotonic() - began
        at.enter()
        await asyncio.wait_for(waiting, 5)

        # Ten steps, each of which would be drawn by itself, and the last one is shown.
        assert 0 < drawn <= took / between + 2
        assert drawn < 10


# Stopping and leaving


async def test_escape_stops_what_is_running_and_does_nothing_when_nothing_is() -> None:
    stopped: list[str] = []
    with desk() as at:
        prompt = Prompt(at.terminal)
        prompt.on_cancel = lambda: stopped.append("stop")
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "a")

        at.type(ESCAPE)
        # A key that follows Escape at once is taken together with it, as a terminal sends
        # the keys that are pressed with Alt. A person is slower than that.
        await asyncio.sleep(0.4)
        await typed(prompt, at, "b")
        idle = list(stopped)
        prompt.busy = True
        at.type(ESCAPE)
        await until(lambda: stopped == ["stop"])
        at.enter("c")

        assert idle == []
        assert await asyncio.wait_for(waiting, 5) == "abc"


async def test_ctrl_c_stops_what_is_running() -> None:
    stopped: list[str] = []
    with desk() as at:
        prompt = Prompt(at.terminal)
        prompt.on_cancel = lambda: stopped.append("stop")
        prompt.busy = True
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "kept")

        at.type(CTRL_C)
        await until(lambda: stopped == ["stop"])
        at.enter()

        assert await asyncio.wait_for(waiting, 5) == "kept"


async def test_ctrl_c_clears_the_line_and_on_an_empty_line_the_second_one_leaves() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "not this")

        at.type(CTRL_C)
        await until(lambda: prompt.typed == "")
        at.type(CTRL_C)
        await until(lambda: "Press Ctrl+C again to leave." in at.shown())
        at.type(CTRL_C)

        assert await asyncio.wait_for(waiting, 5) is None


async def test_ctrl_c_followed_by_typing_does_not_count_towards_leaving() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())

        at.type(CTRL_C)
        await until(lambda: "Press Ctrl+C again to leave." in at.shown())
        await typed(prompt, at, "x")
        at.type(CTRL_C)
        await until(lambda: prompt.typed == "")
        # The line is empty again, and this is the first of two once more.
        at.type(CTRL_C)
        await until(lambda: at.shown().count("Press Ctrl+C again to leave.") == 2)
        at.enter("stay")

        assert await asyncio.wait_for(waiting, 5) == "stay"


async def test_a_line_that_is_submitted_takes_back_the_wish_to_leave() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        at.type(CTRL_C)
        await until(lambda: "Press Ctrl+C again to leave." in at.shown())

        # An empty line is submitted. Nothing was typed, so nothing but the line says that
        # the user goes on.
        at.enter()
        assert await asyncio.wait_for(waiting, 5) == ""
        waiting = asyncio.ensure_future(prompt.line())
        at.type(CTRL_C)
        await until(lambda: at.shown().count("Press Ctrl+C again to leave.") == 2)
        at.type(CTRL_C)

        assert await asyncio.wait_for(waiting, 5) is None


async def test_ctrl_d_leaves_from_an_empty_line_only() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "ab")

        at.type(CTRL_D)
        at.enter()
        first = await asyncio.wait_for(waiting, 5)
        at.type(CTRL_D)
        second = await asyncio.wait_for(prompt.line(), 5)

        assert first == "ab"
        assert second is None


async def test_escape_does_nothing_when_nobody_said_what_it_stops() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        prompt.busy = True
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "a")

        at.type(ESCAPE)
        await asyncio.sleep(0.4)
        at.enter("b")

        assert await asyncio.wait_for(waiting, 5) == "ab"


async def test_a_terminal_that_goes_away_is_like_leaving() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "half a li")

        at.hang_up()

        assert await asyncio.wait_for(waiting, 5) is None


# Questions


async def test_a_question_is_answered_by_one_key() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yan", "[y] yes  [a] always  [n] no"))
        await offered(at, "[y] yes  [a] always  [n] no")

        at.type("a")

        assert await asyncio.wait_for(asked, 5) == "a"
        at.enter("next")
        assert await asyncio.wait_for(waiting, 5) == "next"


async def test_the_first_answer_counts_and_a_second_key_changes_nothing() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await offered(at, "[y] yes  [n] no")

        at.type("yn")

        assert await asyncio.wait_for(asked, 5) == "y"
        waiting.cancel()


async def test_ctrl_d_answers_a_question_with_its_last_key() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yan", "[y] yes  [a] always  [n] no"))
        await offered(at, "[y] yes  [a] always  [n] no")

        at.type(CTRL_D)

        assert await asyncio.wait_for(asked, 5) == "n"
        assert not waiting.done()
        waiting.cancel()


async def test_escape_answers_a_question_with_its_last_key() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await offered(at, "[y] yes  [n] no")

        at.type(ESCAPE)

        assert await asyncio.wait_for(asked, 5) == "n"
        waiting.cancel()


async def test_ctrl_c_answers_a_question_with_its_last_key_and_stops_nothing() -> None:
    stopped: list[str] = []
    with desk() as at:
        prompt = Prompt(at.terminal)
        prompt.on_cancel = lambda: stopped.append("stop")
        prompt.busy = True
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await offered(at, "[y] yes  [n] no")

        at.type(CTRL_C)

        assert await asyncio.wait_for(asked, 5) == "n"
        assert stopped == []
        waiting.cancel()


async def test_a_key_that_is_no_answer_is_no_answer_and_neither_is_a_line() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await offered(at, "[y] yes  [n] no")

        at.type("aq" + ENTER)
        at.type("Y")

        # The keys before it changed nothing: the answer is the first key that is one.
        assert await asyncio.wait_for(asked, 5) == "y"
        assert prompt.typed == ""
        assert not waiting.done()
        waiting.cancel()


@pytest.mark.parametrize("brought", ["", "ya", "an"])
async def test_a_key_that_does_not_bring_one_letter_is_no_answer(brought: str) -> None:
    # A console hands some keys over without any text, and nothing at all is found in every
    # text. Two letters at once are found in the keys too, and are none of them. The key
    # has to be one that the line editor has no use for, or it would not get this far.
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yan", "[y] yes  [a] always  [n] no"))
        await offered(at, "[y] yes  [a] always  [n] no")

        at.press(Keys.ControlPageUp, brought)
        at.type("n")

        assert await asyncio.wait_for(asked, 5) == "n"
        waiting.cancel()


async def test_what_was_typed_before_a_question_is_there_again_after_it() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "No, the lex")

        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await offered(at, "[y] yes  [n] no")
        at.type("zz")
        at.type("n")
        await asyncio.wait_for(asked, 5)
        at.enter("er.")

        assert await asyncio.wait_for(waiting, 5) == "No, the lexer."


async def test_keys_that_were_under_way_when_a_question_came_do_not_answer_it(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Somebody who is in the middle of a word has not read the question that appears.
    monkeypatch.setattr(prompt_module, "_SETTLE_SECONDS", 0.5)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "Stop that, ")

        asked = asyncio.ensure_future(prompt.ask("yan", "[y/a/n]", [Text("May it be run?")]))
        await until(lambda: prompt.typed == "")
        at.type("any day now")
        await offered(at, "[y/a/n]")
        # The keys are gone: they were no answer, and they are no part of the line either.
        await asyncio.sleep(0.05)
        assert not asked.done()
        at.type("n")

        assert await asyncio.wait_for(asked, 5) == "n"
        at.enter("please.")
        assert await asyncio.wait_for(waiting, 5) == "Stop that, please."


async def test_a_question_takes_an_answer_once_the_keys_have_been_still_for_a_while(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(prompt_module, "_SETTLE_SECONDS", 0.6)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y/n]", [Text("May it be run?")]))
        await at.sees("May it be run?")

        # A key every fifth of a second, for longer than the keys have to be still.
        for _ in range(5):
            at.type("y")
            await asyncio.sleep(0.2)

        assert not at_the_prompt(at, "[y/n]")
        assert not asked.done()
        await offered(at, "[y/n]")
        at.type("y")
        assert await asyncio.wait_for(asked, 5) == "y"
        waiting.cancel()


async def test_the_keys_of_a_question_are_shown_when_they_count(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(prompt_module, "_SETTLE_SECONDS", 30.0)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yn", "[y/n]", [Text("May it be run?")]))
        await at.sees("May it be run?")
        await offered(at, "...")

        at.type("y")
        await asyncio.sleep(0.1)

        # Whoever sees no keys offered has answered nothing by pressing one.
        assert not at_the_prompt(at, "[y/n]")
        assert not asked.done()
        asked.cancel()
        waiting.cancel()


async def test_a_question_can_be_declined_before_its_keys_count(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(prompt_module, "_SETTLE_SECONDS", 30.0)
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        asked = asyncio.ensure_future(prompt.ask("yan", "[y/a/n]", [Text("May it be run?")]))
        await at.sees("May it be run?")
        await offered(at, "...")

        at.type(ESCAPE)

        # The answer that changes nothing is safe to give without having read anything.
        assert await asyncio.wait_for(asked, 5) == "n"
        waiting.cancel()


async def test_a_line_that_was_submitted_does_not_come_back_after_a_question() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        at.enter("do it")
        assert await asyncio.wait_for(prompt.line(), 5) == "do it"

        # The question comes between two lines, as when a call follows a task at once.
        asked = asyncio.ensure_future(prompt.ask("yn", "[y/n]"))
        await asyncio.sleep(0)
        waiting = asyncio.ensure_future(prompt.line())
        await offered(at, "[y/n]")
        at.type("n")
        await asyncio.wait_for(asked, 5)

        assert prompt.typed == ""
        at.enter("next")
        assert await asyncio.wait_for(waiting, 5) == "next"


async def test_a_question_that_is_withdrawn_leaves_the_line_as_it_was() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "half")
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await until(lambda: prompt.typed == "")

        asked.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asked
        at.enter(" done")

        assert await asyncio.wait_for(waiting, 5) == "half done"


async def test_a_question_that_is_withdrawn_at_once_waits_for_nothing_more() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        await typed(prompt, at, "half")
        asked = asyncio.ensure_future(prompt.ask("yn", "[y/n]", [Text("May it be run?")]))
        # The question has put the line aside, and waits for its words to be printed.
        await asyncio.sleep(0)

        asked.cancel()
        with pytest.raises(asyncio.CancelledError):
            await asked
        at.enter(" done")

        assert await asyncio.wait_for(waiting, 5) == "half done"
        assert not at_the_prompt(at, "[y/n]")


async def test_one_question_is_asked_at_a_time() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        waiting = asyncio.ensure_future(prompt.line())
        first = asyncio.ensure_future(prompt.ask("yn", "[1]", [Text("The first question.")]))
        second = asyncio.ensure_future(prompt.ask("yn", "[2]", [Text("The second question.")]))
        await stands(lambda: at.window()[:2], ["The first question.", "[1]"])
        # The second question waits, words and all. Whoever reads a question and presses a
        # key must be answering that question.
        await asyncio.sleep(0.1)
        assert "The second question." not in at.shown()

        at.type("y")
        assert await asyncio.wait_for(first, 5) == "y"
        await stands(
            lambda: at.window()[:3], ["The first question.", "The second question.", "[2]"]
        )
        at.type("n")

        assert await asyncio.wait_for(second, 5) == "n"
        waiting.cancel()


async def test_a_question_can_be_asked_while_nobody_waits_for_a_line() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal)
        asked = asyncio.ensure_future(prompt.ask("yn", "[y] yes  [n] no"))
        await asyncio.sleep(0)

        waiting = asyncio.ensure_future(prompt.line())
        await offered(at, "[y] yes  [n] no")
        at.type("y")

        assert await asyncio.wait_for(asked, 5) == "y"
        waiting.cancel()


# The status line


async def test_the_status_line_is_asked_for_whenever_the_prompt_is_drawn() -> None:
    with desk() as at:
        prompt = Prompt(at.terminal, status=lambda: "model-x | idle")

        assert prompt.status() == "model-x | idle"


async def test_the_status_line_cannot_steer_the_terminal() -> None:
    # The name of a model comes from a file or from what somebody typed.
    with desk() as at:
        prompt = Prompt(at.terminal, status=lambda: f"safe{OVERRIDE}abc | idle")
        waiting = asyncio.ensure_future(prompt.line())

        await until(lambda: any("| idle" in row for row in at.window()))

        assert OVERRIDE not in at.sent()
        assert any(f"safe{STAND_IN}abc | idle" in row for row in at.window())
        waiting.cancel()
