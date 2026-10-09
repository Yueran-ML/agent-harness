"""A result that is too large: the model gets its two ends, and the whole of it is saved."""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

import columbina.fs
import columbina.session
import columbina.spill.local
import columbina.spill.policy
import columbina.tools
from columbina.kernel import Context, Next
from columbina.llm import ToolCallBlock
from columbina.session import SESSIONS, Session
from columbina.spill import Saved
from columbina.spill.policy import Config, two_ends
from columbina.token_meter import estimate
from columbina.tools import (
    POST_EXECUTE,
    TOOLS,
    Accept,
    Executed,
    Tool,
    ToolOutcome,
    ToolRun,
    Verdict,
)

LINE = "The quick brown fox jumps over the lazy dog.\n"


class Nothing(BaseModel):
    pass


@dataclass
class Bench:
    ctx: Context
    session: Session
    work: Path
    root: Path

    def offer(self, name: str, value: object, **more: Any) -> None:
        async def give(args: Nothing, run: ToolRun) -> object:
            return value

        self.ctx.use(TOOLS).add(
            self.ctx,
            Tool(name=name, description="Gives.", parameters=Nothing, execute=give, **more),
        )

    async def call(self, name: str) -> ToolOutcome:
        block = ToolCallBlock(call_id="c-1", name=name, arguments="{}")
        return await self.ctx.use(TOOLS).run(
            self.ctx, block, ToolRun("c-1", self.work, self.session)
        )

    def saved(self) -> list[Path]:
        return sorted(path for path in self.root.rglob("*") if path.is_file())


async def bench(ctx: Context, tmp_path: Path, *, store: bool = True, **config: object) -> Bench:
    work = tmp_path / "work"
    work.mkdir()
    root = tmp_path / "home" / "spill"
    await ctx.plugin(columbina.session)
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    if store:
        await ctx.plugin(columbina.spill.local, {"root": root})
    await ctx.plugin(columbina.spill.policy, config)
    session = ctx.use(SESSIONS).create(ctx, work, session_id="s-1")
    return Bench(ctx, session, work, root)


def test_a_result_may_have_twelve_and_a_half_thousand_tokens() -> None:
    assert Config().max_tokens == 12_500


async def test_a_result_that_is_small_enough_goes_through_as_it_is(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=400)
    text = LINE * 20
    assert estimate(text) <= 400
    at.offer("say", text)

    outcome = await at.call("say")

    assert outcome.text == text
    assert at.saved() == []


async def test_of_a_result_that_is_too_large_the_model_gets_both_ends_and_the_place_of_the_rest(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=400)
    lines = [f"line {number}: {LINE}" for number in range(400)]
    text = "".join(lines)
    at.offer("say", text)

    outcome = await at.call("say")

    (file,) = at.saved()
    assert file.read_text(encoding="utf-8") == text
    assert file.name.endswith("-say-result.txt")
    shown = outcome.text
    assert outcome.error is None
    assert estimate(shown) <= 400
    assert shown.startswith(lines[0] + lines[1])
    assert lines[-1] in shown
    head, gap, rest = shown.partition("\n[... ")
    assert gap
    left_out, _, rest = rest.partition(" characters are left out here ...]\n")
    tail, _, footer = rest.rpartition("\n(")
    # Each end stops where a line ends, and the line break belongs to it.
    assert int(left_out) == len(text) - len(head + "\n") - len(tail + "\n")
    assert text.startswith(head + "\n")
    assert text.endswith(tail + "\n")
    # Both ends are whole lines.
    assert tail.startswith("line ")
    assert footer == (
        f"The whole result has {len(text.encode())} bytes and is in {file}. Read a part of it "
        "with the read tool, by offset and limit, or search it with grep.)"
    )


async def test_the_two_ends_get_the_same_room(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path, max_tokens=1000)
    text = LINE * 2000
    at.offer("say", text)

    shown = (await at.call("say")).text

    head, _, rest = shown.partition("\n[... ")
    tail = rest.partition(" ...]\n")[2].rpartition("\n(")[0]
    assert abs(len(head) - len(tail)) <= len(LINE)
    assert estimate(shown) > 900


async def test_a_text_without_line_breaks_is_cut_where_the_room_ends(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    text = "ab " * 5000
    at.offer("say", text)

    shown = (await at.call("say")).text

    assert estimate(shown) <= 300
    assert estimate(shown) > 270
    assert shown.startswith("ab ab ab ")


def test_the_two_ends_of_a_text_never_hold_a_part_of_it_twice() -> None:
    # With more room than the text needs, the beginning is all of it, and nothing is left
    # that the end could be taken from.
    shown = two_ends("one\ntwo\n", 1000, "(the last line)")

    assert shown == "one\ntwo\n[... 0 characters are left out here ...]\n(the last line)"


async def test_a_result_of_exactly_the_limit_goes_through(ctx: Context, tmp_path: Path) -> None:
    text = LINE * 30
    at = await bench(ctx, tmp_path, max_tokens=estimate(text))
    at.offer("say", text)

    assert (await at.call("say")).text == text
    assert at.saved() == []


async def test_a_result_of_one_token_more_than_the_limit_is_cut(
    ctx: Context, tmp_path: Path
) -> None:
    text = LINE * 30
    at = await bench(ctx, tmp_path, max_tokens=estimate(text) - 1)
    at.offer("say", text)

    assert (await at.call("say")).text != text
    assert len(at.saved()) == 1


async def test_an_end_stops_at_a_line_only_where_that_costs_it_little(
    ctx: Context, tmp_path: Path
) -> None:
    # A line break near the cut is where a reader wants the cut. One that lies far from it
    # would take most of what there was room for.
    at = await bench(ctx, tmp_path, max_tokens=300)
    text = "first\n" + "ab " * 5000 + "\nlast"
    at.offer("say", text)

    shown = (await at.call("say")).text

    assert shown.startswith("first\nab ab ")
    assert "ab \nlast\n(The whole result has " in shown
    assert estimate(shown) > 270


async def test_a_tool_that_hands_out_its_result_in_pages_is_left_alone(
    ctx: Context, tmp_path: Path
) -> None:
    # It bounds what it returns by itself, and says how to get the next page.
    at = await bench(ctx, tmp_path, max_tokens=100)
    text = LINE * 500
    at.offer("pages", text, paged=True)

    outcome = await at.call("pages")

    assert outcome.text == text
    assert at.saved() == []


async def test_the_read_tool_hands_out_its_result_in_pages(ctx: Context, tmp_path: Path) -> None:
    from columbina.builtin_tools import read

    await bench(ctx, tmp_path)
    await ctx.plugin(read)

    assert ctx.use(TOOLS).registry["read"].paged
    assert not Tool(name="say", description="Says.", parameters=Nothing, execute=None).paged  # pyright: ignore[reportArgumentType]


@dataclass(frozen=True)
class Ran:
    """What a tool returns that has saved the whole of its output itself."""

    text: str
    complete: Saved | None

    def __str__(self) -> str:
        return self.text


async def test_a_result_whose_whole_was_saved_by_its_tool_is_not_saved_again(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    kept = Saved("X:/spill/abcd-output.txt", 4_000_000, "Read it in parts.")
    at.offer("run", Ran(LINE * 500, kept))

    shown = (await at.call("run")).text

    assert at.saved() == []
    assert estimate(shown) <= 300
    assert shown.endswith(
        "\n(The whole result has 4000000 bytes and is in X:/spill/abcd-output.txt. "
        "Read it in parts.)"
    )
    assert " characters are left out here ...]\n" in shown


async def test_a_small_result_with_a_hole_in_it_still_says_where_the_whole_is(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    kept = Saved("X:/spill/abcd-output.txt", 4_000_000, "Read it in parts.")
    at.offer("run", Ran("the beginning\n[many bytes are left out here]\nthe end", kept))

    shown = (await at.call("run")).text

    assert shown == (
        "the beginning\n[many bytes are left out here]\nthe end\n"
        "(The whole result has 4000000 bytes and is in X:/spill/abcd-output.txt. "
        "Read it in parts.)"
    )


async def test_a_tool_that_saved_nothing_is_treated_like_any_other(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    at.offer("run", Ran("all of it", None))

    assert (await at.call("run")).text == "all of it"


async def test_without_a_place_to_save_it_a_large_result_is_cut_all_the_same(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, store=False, max_tokens=300)
    at.offer("say", LINE * 500)

    shown = (await at.call("say")).text

    assert estimate(shown) <= 300
    assert shown.endswith(
        "\n(The whole result was not kept: this program has no place to save it.)"
    )


async def test_a_result_that_cannot_be_saved_is_cut_all_the_same_and_says_so(
    ctx: Context, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    # Where the directory of what is saved would be, there is a file.
    at.root.parent.mkdir(parents=True)
    at.root.write_text("in the way", encoding="utf-8")
    at.offer("say", LINE * 500)

    shown = (await at.call("say")).text

    assert estimate(shown) <= 300
    assert "\n(The whole result could not be saved: " in shown
    assert shown.endswith(")")
    assert "could not be saved" in caplog.text


async def test_what_another_check_made_of_a_result_is_what_is_measured(
    ctx: Context, tmp_path: Path
) -> None:
    async def longer(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return Accept(text=LINE * 500)

    # The other check is there first, and the limit still comes after it.
    ctx.on(POST_EXECUTE, longer)
    at = await bench(ctx, tmp_path, max_tokens=300)
    at.offer("say", "short")

    shown = (await at.call("say")).text

    assert estimate(shown) <= 300
    (file,) = at.saved()
    assert file.read_text(encoding="utf-8") == LINE * 500


async def test_a_result_that_is_withheld_stays_withheld(ctx: Context, tmp_path: Path) -> None:
    from columbina.tools import Block

    at = await bench(ctx, tmp_path, max_tokens=300)

    async def withheld(executed: Executed, proceed: Next[Executed, Verdict]) -> Verdict:
        return Block(reason="it holds a secret")

    ctx.on(POST_EXECUTE, withheld)
    at.offer("say", LINE * 500)

    outcome = await at.call("say")

    assert outcome.error is not None
    assert outcome.error.code == "RESULT_BLOCKED"
    assert at.saved() == []


async def test_a_result_of_json_on_one_line_can_be_read_where_it_was_saved(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, max_tokens=300)
    text = json.dumps({"rows": [{"id": number, "name": "herring"} for number in range(2000)]})
    at.offer("say", text)

    shown = (await at.call("say")).text

    (file,) = at.saved()
    kept = file.read_text(encoding="utf-8")
    assert kept.replace("\n", "") == text
    assert max(len(line) for line in kept.split("\n")) == 2000
    assert shown.endswith("Lines of more than 2000 characters are broken there.)")
