"""The stroke, the shadow, the background and the runs of a title, as the agent gives them."""

from collections.abc import AsyncIterator, Mapping
from pathlib import Path
from typing import Any

import pytest
from pydantic import ValidationError

from columbina.kernel import Context
from columbina.tools import ToolOutcome
from columbina.video import Config, Videos, clip_keys, edits, titles, tools_of
from columbina.video.edits import (
    Background,
    Key,
    Keys,
    Shadow,
    Span,
    Stroke,
    TitleSource,
    TitleStyle,
)
from columbina.video.titles import TitleError
from support.desk import Desk
from support.fake_engine import Fake, served
from support.studio import clip_of, timeline_of, track_of
from support.studio_tools import desk_of, result

pytestmark = pytest.mark.usefixtures("studio_here")

TEXT = "Hello brave world"
"""The text of the title of the tests: "brave" is its characters 6 to 10, "world" 12 to 16."""

STYLE: dict[str, Any] = {
    "font": "", "size": 40, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center",
}  # fmt: skip
"""A style without a stroke, a shadow, a background or spans, as the engine writes it."""


def title_of(id: str, start: int, length: int, style: dict[str, Any]) -> dict[str, Any]:
    """A clip of a title that says :data:`TEXT`."""
    clip = clip_of(id, start, length)
    clip["source"] = {"kind": "title", "text": TEXT, "style": style}
    return clip


def film(fake: Fake) -> None:
    """Timeline t1: c2, a clip of media on frames 0 to 39 of V1, and c1, a title on frames 40
    to 59 after it, with "brave" bold."""
    bold: dict[str, Any] = {**STYLE, "spans": [{"from": 6, "to": 11, "bold": True}]}
    pictures = [clip_of("c2", 0, 40), title_of("c1", 40, 20, bold)]
    timeline = timeline_of(track_of("V1", pictures, kind="video"))
    fake.project.update(schema=5, next_id=20, media=[], timelines=[timeline])


@pytest.fixture
async def studio(ctx: Context, tmp_path: Path) -> AsyncIterator[tuple[Desk, Fake]]:
    desk = await desk_of(ctx, tmp_path)
    async with served(desk.work / ".columbina" / "studio") as fake:
        film(fake)
        yield desk, fake


def clip(fake: Fake, id: str) -> dict[str, Any]:
    clips = fake.project["timelines"][0]["tracks"][0]["clips"]
    return next(one for one in clips if one["id"] == id)


def style_of(fake: Fake, id: str = "c1") -> dict[str, Any]:
    """The style of a title of V1, as the fake engine keeps it."""
    return clip(fake, id)["source"]["style"]


async def edited(desk: Desk, items: list[dict[str, Any]]) -> dict[str, Any]:
    return result(await desk.call("video_edit", edits=items))


def refused(outcome: ToolOutcome, code: str = "VIDEO_TITLE") -> str:
    """What a call that was refused said; it is to be refused as ``code``."""
    assert outcome.error is not None, outcome.text
    assert outcome.error.code == code, outcome.text
    return outcome.text


def own(span: Mapping[str, Any]) -> dict[str, Any]:
    """What a span has besides the characters that it is over."""
    return {key: value for key, value in span.items() if key not in ("from", "to")}


# ---------------------------------------------------------------------------------------------
# The builders
# ---------------------------------------------------------------------------------------------


def test_the_builders_write_the_parts_of_a_style_and_leave_out_those_it_has_not() -> None:
    # A title without them is written as it was before them, to the letter.
    plain = '{"font":"","size":72,"color":"#ffffffff","x":0.5,"y":0.5,"align":"center"}'
    assert TitleStyle().model_dump_json() == plain
    assert TitleStyle(spans=[]).model_dump_json() == plain
    assert TitleStyle.model_validate_json(plain).model_dump_json() == plain
    # A part given in part has the rest of a new one.
    style = TitleStyle(stroke=Stroke(width=8), shadow=Shadow(x=-2), background=Background(radius=0))
    written = style.model_dump(mode="json")
    assert written["stroke"] == {"color": "#000000ff", "width": 8}
    assert written["shadow"] == {"color": "#00000080", "x": -2, "y": 4, "blur": 4}
    assert written["background"] == {"color": "#00000099", "padding": 16, "radius": 0}
    assert TitleStyle.model_validate(written) == style
    # A span is written with what it has of its own, by "from", as the engine reads it.
    bold = Span.model_validate({"from": 6, "to": 11, "bold": True})
    red = Span.model_validate({"from": 12, "to": 17, "color": "#ff0000ff", "scale": 1.5})
    source = TitleSource(text=TEXT, length=50, style=TitleStyle(spans=[bold, red]))
    assert edits.set_clip("t1", "c1", source=source)["source"]["style"]["spans"] == [
        {"from": 6, "to": 11, "bold": True},
        {"from": 12, "to": 17, "color": "#ff0000ff", "scale": 1.5},
    ]
    # What the engine would refuse is refused here.
    grows = Keys(keys=[Key(frame=0, value=2), Key(frame=9, value=300)])
    for wrong in (
        lambda: Stroke(width=257),
        lambda: Stroke(width=-1),
        lambda: Stroke(color="#000000"),
        lambda: Shadow(x=1025),
        lambda: Shadow(blur=grows),
        lambda: Background(padding=1025),
        lambda: Background(radius=513),
        lambda: Span.model_validate({"from": 3, "to": 3}),
        lambda: Span.model_validate({"from": 0, "to": 2, "scale": 11}),
        lambda: TitleStyle(spans=[red, bold]),
        lambda: TitleSource(text="Hello", length=5, style=TitleStyle(spans=[bold])),
    ):
        with pytest.raises(ValidationError):
            wrong()


# ---------------------------------------------------------------------------------------------
# The parts and the runs, worked out
# ---------------------------------------------------------------------------------------------


def test_a_part_given_in_part_is_filled_with_what_the_title_or_a_new_one_has() -> None:
    assert titles.part_of("stroke", {}) == {"color": "#000000ff", "width": 4}
    assert titles.part_of("stroke", {"width": 8}) == {"color": "#000000ff", "width": 8}
    assert titles.part_of("shadow", {"x": -2, "color": "#FF000080"}) == {
        "color": "#ff000080",
        "x": -2,
        "y": 4,
        "blur": 4,
    }
    # A colour of six digits is opaque.
    assert titles.part_of("background", {"color": "#336699"}) == {
        "color": "#336699ff",
        "padding": 16,
        "radius": 8,
    }
    # Over the part that a title has, what is left out is as the title has it, keys and all.
    keys = {"keys": [{"frame": 0, "value": 2, "interp": "linear"}]}
    had: dict[str, Any] = {"color": "#ff0000ff", "width": keys}
    assert titles.part_of("stroke", {"color": "#00ff00ff"}, had) == {**had, "color": "#00ff00ff"}
    # The numbers of a new part are those of the engine's catalog, each within its range.
    for name, new in titles.PARTS.items():
        for setting, value in new.items():
            if setting == "color":
                continue
            spec = clip_keys.SETTINGS[f"title.{name}.{setting}"]
            assert spec.default == value
            assert spec.low <= value <= spec.high


def test_what_the_engine_would_refuse_of_a_part_is_said_in_words() -> None:
    for name, given, why in [
        (
            "stroke",
            {"width": 257},
            "title.stroke.width is 0 to 256 px, and 257 is not (the engine refuses it as "
            "out_of_range stroke)",
        ),
        ("stroke", {"width": -1}, "title.stroke.width is 0 to 256 px, and -1 is not"),
        (
            "shadow",
            {"x": 1025},
            "title.shadow.x is -1024 to 1024 px, and 1025 is not (the engine refuses it as "
            "out_of_range shadow)",
        ),
        (
            "shadow",
            {"blur": {"keys": [{"frame": 0, "value": 300}]}},
            "a key of title.shadow.blur is 300, out of 0 to 256 px (the engine refuses it as "
            "out_of_range shadow)",
        ),
        ("background", {"padding": 1025}, "(the engine refuses it as out_of_range background)"),
        ("background", {"radius": 513}, "title.background.radius is 0 to 512 px"),
        ("background", {"radius": "8"}, "title.background.radius is a number, 0 to 512 px"),
        ("stroke", {"color": "black"}, "stroke.color is a colour #rrggbbaa, alpha last"),
        ("stroke", {"size": 3}, "a stroke has no size; it has color, width"),
        ("shadow", True, "shadow is an object {color, x, y, blur}"),
    ]:
        with pytest.raises(TitleError) as raised:
            titles.part_of(name, given)
        assert why in str(raised.value), (given, str(raised.value))


def test_a_run_is_found_by_what_it_says_or_by_its_characters() -> None:
    text = "the cat and the hat"
    assert titles.run_of(text, {"text": "the"}) == (0, 3)
    assert titles.run_of(text, {"text": "the", "occurrence": 2}) == (12, 15)
    assert titles.run_of(text, {"from": 4, "to": 7}) == (4, 7)
    # Characters are Unicode scalars, as the engine counts them: one picture is one.
    assert titles.run_of("\U0001f3ac Take two", {"text": "two"}) == (7, 10)
    for asked, why in [
        ({"text": "dog"}, "'dog' is found nowhere in the text 'the cat and the hat'"),
        (
            {"text": "cat", "occurrence": 2},
            "'cat' is found once in the text 'the cat and the hat', and occurrence is 2",
        ),
        ({"text": "the", "occurrence": 3}, "'the' is found 2 times"),
        ({"text": "the", "occurrence": 0}, "occurrence is which time the text is found"),
        ({"text": "the", "from": 0, "to": 3}, "is named by text or by from and to, not by both"),
        ({"text": ""}, "text is some of the title's text"),
        ({"from": 4}, 'or by the indices of its characters, {"from": 0, "to": 4}'),
        (
            {"from": 4, "to": 20},
            "a span is from 4 to 20: a span is within the text, from 0 to its 19 characters",
        ),
        (
            {"from": 5, "to": 5},
            "ends after it begins (the engine refuses it as out_of_range spans)",
        ),
    ]:
        with pytest.raises(TitleError) as raised:
            titles.run_of(text, asked)
        assert why in str(raised.value), (asked, str(raised.value))


def test_runs_are_laid_over_the_spans_that_a_title_has() -> None:
    bold = [{"from": 6, "to": 11, "bold": True}]
    # What a run is given goes over its part of the text, and the rest of a span is kept.
    red = titles.runs_of(TEXT, [{"text": "o b", "color": "#FF0000"}])
    assert titles.layered(bold, red) == [
        {"from": 4, "to": 6, "color": "#ff0000ff"},
        {"from": 6, "to": 7, "color": "#ff0000ff", "bold": True},
        {"from": 7, "to": 11, "bold": True},
    ]
    # A run takes a setting away, or all that the text has there; what has nothing is no span.
    plain = titles.runs_of(TEXT, [{"text": "brave", "bold": False}])
    assert titles.layered(bold, plain) == []
    cleared = titles.runs_of(TEXT, [{"from": 8, "to": 17, "clear": True, "italic": True}])
    assert titles.layered(bold, cleared) == [
        {"from": 6, "to": 8, "bold": True},
        {"from": 8, "to": 17, "italic": True},
    ]
    # Two that meet with the same style are one, and a later run goes over an earlier one.
    runs = titles.runs_of(
        TEXT,
        [
            {"text": "Hello", "bold": True},
            {"from": 5, "to": 6, "bold": True},
            {"text": "world", "scale": 2},
            {"text": "world", "scale": None},
        ],
    )
    assert titles.layered(bold, runs) == [{"from": 0, "to": 11, "bold": True}]
    # A title has 256 spans at the most.
    many = [{"from": at, "to": at + 1, "bold": True} for at in range(0, 2 * 257, 2)]
    with pytest.raises(TitleError, match="256 spans at the most, and this would have 257"):
        titles.layered([], titles.runs_of("x" * 600, many))
    for asked, why in [
        ({"text": "brave"}, "spans is a list of runs"),
        ([{"text": "brave", "underline": True}], "spans[0] has no underline"),
        ([{"text": "brave", "scale": 20}], "spans[0].scale is 0.1 to 10"),
        ([{"text": "brave", "bold": 1}], "spans[0].bold is true or false"),
        ([{"text": "brave", "clear": "yes"}], "spans[0].clear is true or false"),
        ([{"text": "brave"}, "world"], 'spans[1] is an object, as {"text": "word"'),
    ]:
        with pytest.raises(TitleError) as raised:
            titles.runs_of(TEXT, asked)
        assert why in str(raised.value), (asked, str(raised.value))


def test_the_spans_of_a_title_move_with_its_text() -> None:
    spans = [{"from": 6, "to": 11, "bold": True}, {"from": 12, "to": 17, "italic": True}]
    brave, world = {"bold": True}, {"italic": True}

    def moved(new: str) -> list[tuple[str, dict[str, Any]]]:
        """Each span that is left, by the text that it is over, with its style."""
        kept = titles.moved(TEXT, new, spans)
        return [(new[one["from"] : one["to"]], own(one)) for one in kept]

    assert titles.moved(TEXT, TEXT, spans) == spans
    # Text written before a span or at either end of it moves it, and is not in it.
    assert moved("Oh, Hello brave world") == [("brave", brave), ("world", world)]
    assert moved("Hello brave new world!") == [("brave", brave), ("world", world)]
    assert titles.moved(TEXT, "Hello brave new world", spans)[1] == {
        "from": 16,
        "to": 21,
        "italic": True,
    }
    # Text written within a span widens it, and text written in place of it is in it.
    assert moved("Hello br-ave world") == [("br-ave", brave), ("world", world)]
    assert moved("Hello BRAVE world") == [("BRAVE", brave), ("world", world)]
    # A span whose text is taken away is no more, so none is left past the end of the text.
    assert moved("Hello world") == [("world", world)]
    assert moved("Hello brave") == [("brave", brave)]
    assert moved("Hello") == []
    # Text written in place of the end of one span and the start of the next is in the first.
    halves = [{"from": 0, "to": 1, "bold": True}, {"from": 1, "to": 2, "italic": True}]
    assert titles.moved("ab", "XY", halves) == [{"from": 0, "to": 2, "bold": True}]


# ---------------------------------------------------------------------------------------------
# video_edit
# ---------------------------------------------------------------------------------------------


async def test_a_title_is_put_with_its_parts_filled_and_its_runs_found(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    style: dict[str, Any] = {
        **STYLE,
        "stroke": {"width": 6},
        "shadow": {},
        "background": {"color": "#202020", "radius": 0},
        "spans": [{"text": "brave", "bold": True}, {"from": 12, "to": 17, "color": "#FF0000FF"}],
    }
    source: dict[str, Any] = {"kind": "title", "text": TEXT, "length": 25}
    await edited(desk, [{"edit": "insert", "source": {**source, "style": style}, "at": 60}])
    written: dict[str, Any] = {
        **STYLE,
        "stroke": {"color": "#000000ff", "width": 6},
        "shadow": {"color": "#00000080", "x": 4, "y": 4, "blur": 4},
        "background": {"color": "#202020ff", "padding": 16, "radius": 0},
        "spans": [
            {"from": 6, "to": 11, "bold": True},
            {"from": 12, "to": 17, "color": "#ff0000ff"},
        ],
    }
    (sent,) = fake.transactions[-1]
    assert sent["source"] == {**source, "style": written}
    # The engine keeps them as they are sent, and so does the fake engine.
    put = fake.project["timelines"][0]["tracks"][0]["clips"][-1]
    assert put["start"] == 60
    assert put["source"] == {"kind": "title", "text": TEXT, "style": written}
    # A part given null is left out, and a style without them is sent as it is given.
    shadowless = {**source, "style": {**STYLE, "shadow": None}}
    await edited(desk, [{"edit": "append", "source": shadowless}])
    assert fake.transactions[-1][0]["source"]["style"] == STYLE
    await edited(desk, [{"edit": "append", "source": {**source, "style": STYLE}}])
    assert fake.transactions[-1][0]["source"] == {**source, "style": STYLE}
    # A title's source set whole has its runs found in its own text.
    italic = {**STYLE, "spans": [{"text": "words", "italic": True}]}
    words: dict[str, Any] = {"kind": "title", "text": "New words", "style": italic}
    await edited(desk, [{"op": "set_clip", "id": "c1", "source": words}])
    assert clip(fake, "c1")["source"]["text"] == "New words"
    assert style_of(fake)["spans"] == [{"from": 4, "to": 9, "italic": True}]


async def test_what_the_engine_would_refuse_of_a_title_is_told_before_anything_is_sent(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    source: dict[str, Any] = {"kind": "title", "text": TEXT, "length": 25}
    for style, why in [
        (
            {**STYLE, "stroke": {"width": 300}},
            "title.stroke.width is 0 to 256 px, and 300 is not (the engine refuses it as "
            "out_of_range stroke)",
        ),
        ({**STYLE, "shadow": {"y": -2000}}, "(the engine refuses it as out_of_range shadow)"),
        ({**STYLE, "background": {"padding": 2000}}, "out_of_range background"),
        ({**STYLE, "spans": [{"text": "brave", "scale": 20}]}, "spans[0].scale is 0.1 to 10"),
        (
            {**STYLE, "spans": [{"from": 12, "to": 18}]},
            "spans[0] is from 12 to 18: a span is within the text, from 0 to its 17 characters",
        ),
        ({**STYLE, "spans": [{"text": "Brave"}]}, "'Brave' is found nowhere in the text"),
    ]:
        insert = {"edit": "insert", "source": {**source, "style": style}, "at": 60}
        told = refused(await desk.call("video_edit", edits=[insert]))
        assert why in told, (style, told)
    # Of several edits, the one that is wrong is named by its place.
    wrong = {**source, "style": {**STYLE, "stroke": {"width": -1}}}
    told = refused(
        await desk.call(
            "video_edit",
            edits=[
                {"op": "set_clip", "id": "c2", "gain_db": -3},
                {"op": "set_clip", "id": "c1", "source": wrong},
            ],
        )
    )
    assert "edits[1]: title.stroke.width is 0 to 256 px, and -1 is not" in told
    assert fake.transactions == []
    assert fake.steps == []


async def test_set_title_changes_a_title_from_what_it_has(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    # Runs named by what they say are laid over the spans that the title has.
    runs = [{"text": "o b", "color": "#ff0000ff"}, {"text": "world", "italic": True}]
    told = await edited(desk, [{"edit": "set_title", "clip": "c1", "spans": runs}])
    spans = [
        {"from": 4, "to": 6, "color": "#ff0000ff"},
        {"from": 6, "to": 7, "color": "#ff0000ff", "bold": True},
        {"from": 7, "to": 11, "bold": True},
        {"from": 12, "to": 17, "italic": True},
    ]
    title: dict[str, Any] = {"kind": "title", "text": TEXT, "style": {**STYLE, "spans": spans}}
    assert fake.transactions[-1] == [
        {"op": "set_clip", "timeline": "t1", "id": "c1", "source": title}
    ]
    assert told["revision"] == 1
    assert clip(fake, "c1")["source"] == title
    # A new text takes the spans along: those of "brave" go with it, and "world" moves back.
    await edited(desk, [{"edit": "set_title", "clip": "c1", "text": "Hello world"}])
    assert clip(fake, "c1")["source"]["text"] == "Hello world"
    assert style_of(fake)["spans"] == [
        {"from": 4, "to": 6, "color": "#ff0000ff"},
        {"from": 6, "to": 11, "italic": True},
    ]
    # The style is patched: a part given in part is laid over the title's, or over a new one.
    patch = {"size": 64, "stroke": {"width": 2}, "shadow": {"x": -3}}
    await edited(desk, [{"edit": "set_title", "clip": "c1", "style": patch}])
    white = {"stroke": {"color": "#ffffff"}}
    await edited(desk, [{"edit": "set_title", "clip": "c1", "style": white}])
    style = style_of(fake)
    assert (style["size"], style["font"], style["align"]) == (64, "", "center")
    assert style["stroke"] == {"color": "#ffffffff", "width": 2}
    assert style["shadow"] == {"color": "#00000080", "x": -3, "y": 4, "blur": 4}
    assert len(style["spans"]) == 2
    # Each call was one transaction of one operation, one step to undo.
    assert all(len(ops) == 1 for ops in fake.transactions)
    assert len(fake.steps) == len(fake.transactions) == 4


async def test_set_title_takes_parts_and_spans_away_and_goes_with_other_edits(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    given = {"stroke": {"width": 2, "color": "#ffffff"}, "shadow": {"x": -3}}
    await edited(desk, [{"edit": "set_title", "clip": "c1", "style": given}])
    assert style_of(fake)["stroke"] == {"color": "#ffffffff", "width": 2}
    # null takes a part away, and spans null every span; the rest is kept.
    away = {"edit": "set_title", "clip": "c1", "style": {"shadow": None}, "spans": None}
    await edited(desk, [{"op": "set_clip", "id": "c2", "gain_db": -3}, away])
    gain, title = fake.transactions[-1]
    assert gain == {"op": "set_clip", "timeline": "t1", "id": "c2", "gain_db": -3}
    assert (title["op"], title["id"]) == ("set_clip", "c1")
    assert style_of(fake) == {**STYLE, "stroke": {"color": "#ffffffff", "width": 2}}
    assert clip(fake, "c2")["gain_db"] == -3
    # Each call was one step, which is undone whole.
    assert len(fake.steps) == len(fake.transactions) == 2
    result(await desk.call("video_undo"))
    assert style_of(fake)["shadow"] == {"color": "#00000080", "x": -3, "y": 4, "blur": 4}
    assert style_of(fake)["spans"] == [{"from": 6, "to": 11, "bold": True}]
    assert clip(fake, "c2")["gain_db"] == 0.0


async def test_set_title_is_refused_before_anything_is_sent(studio: tuple[Desk, Fake]) -> None:
    desk, fake = studio
    for item, why in [
        (
            {"clip": "c2", "text": "Hi"},
            "c2 is not a title: set_title changes the text and the style of a title",
        ),
        ({"clip": "c7", "text": "Hi"}, "there is no clip c7 on timeline t1"),
        ({"clip": "$t", "text": "Hi"}, "and $t is put in it"),
        ({"clip": "c1"}, "set_title needs text, style or spans"),
        ({"clip": "c1", "colour": "#fff"}, "set_title has no colour"),
        ({"clip": "c1", "style": {"bold": True}}, "a title's style has no bold; it has font"),
        ({"clip": "c1", "style": {"stroke": {"width": 999}}}, "out_of_range stroke"),
        (
            {"clip": "c1", "spans": [{"text": "brave"}], "style": {"spans": None}},
            "spans are given once",
        ),
        (
            {"clip": "c1", "text": "Hi", "spans": [{"text": "brave", "bold": True}]},
            "spans[0]: 'brave' is found nowhere in the text 'Hi'",
        ),
    ]:
        told = refused(await desk.call("video_edit", edits=[{"edit": "set_title", **item}]))
        assert why in told, (item, told)
    assert fake.transactions == []
    assert fake.steps == []


# ---------------------------------------------------------------------------------------------
# video_clip_keys
# ---------------------------------------------------------------------------------------------


async def test_the_width_of_a_stroke_takes_keys_at_the_frames_of_the_title(
    studio: tuple[Desk, Fake],
) -> None:
    desk, fake = studio
    width = {"clip": "c1", "path": "title.stroke.width"}
    # A title without a stroke has no width to key: it is told how to give it one.
    told = refused(
        await desk.call("video_clip_keys", **width, action="put", frame=45, value=8),
        "VIDEO_KEYS",
    )
    assert (
        "title c1 has no stroke, so no title.stroke.width to key: give it one first with "
        'video_edit, {"edit": "set_title", "clip": "c1", "style": {"stroke": {}}} for a new '
        "stroke"
    ) in told
    assert fake.transactions == []
    await edited(desk, [{"edit": "set_title", "clip": "c1", "style": {"stroke": {}}}])
    # c1 is on frames 40 to 59: frame 45 of the timeline is frame 5 of the title.
    keyed = result(await desk.call("video_clip_keys", **width, action="put", frame=45, value=8))
    first = {"frame": 5, "value": 8, "interp": "linear"}
    assert fake.transactions[-1] == [
        {
            "op": "splice_clip_keys",
            "timeline": "t1",
            "id": "c1",
            "path": "title.stroke.width",
            "at": 0,
            "remove": 0,
            "keys": [first],
        }
    ]
    assert style_of(fake)["stroke"] == {"color": "#000000ff", "width": {"keys": [first]}}
    assert keyed["key"] == {"timeline_frame": 45, "frame": 5}
    assert keyed["key_frames"] == "frames of the clip, from 0"
    assert keyed["setting"] == {"keys": [{"timeline_frame": 45, **first}]}
    # A later key goes after it; the last key taken away leaves its value as the number.
    result(await desk.call("video_clip_keys", **width, action="put", frame=59, value=0))
    assert [key["frame"] for key in style_of(fake)["stroke"]["width"]["keys"]] == [5, 19]
    result(await desk.call("video_clip_keys", **width, action="remove", frame=59))
    result(await desk.call("video_clip_keys", **width, action="remove", frame=45))
    assert fake.transactions[-1] == [
        {
            "op": "set_clip_keys",
            "timeline": "t1",
            "id": "c1",
            "path": "title.stroke.width",
            "param": 8,
        }
    ]
    assert style_of(fake)["stroke"] == {"color": "#000000ff", "width": 8}
    # The other parts are keyed alike, within their ranges, and named as a model may name them.
    sends = len(fake.transactions)
    put = {"action": "put", "frame": 45, "value": 4}
    for asked, why in [
        (
            {"path": "title.shadow.x", **put, "value": 2000},
            "title.shadow.x is 2000, out of range (Shadow offset X of a title, right, in pixels: "
            "-1024 to 1024, 4 in a new shadow)",
        ),
        (
            {"path": "title.background.radius", **put, "value": 600},
            "title.background.radius is 600, out of range (Background corner radius of a title, "
            "in pixels: 0 to 512, 8 in a new background)",
        ),
        ({"path": "title.background.radius", **put}, "title c1 has no background"),
        ({"path": "title.stroke", **put}, "(did you mean title.stroke.width?)"),
        ({"path": "shadow.blur", **put}, "(did you mean title.shadow.blur?)"),
        ({"path": "title.shadow.X", **put}, "(did you mean title.shadow.x?)"),
        ({"path": "title.X", **put}, "(did you mean title.x?)"),
        ({"path": "title.stroke.width", **put, "clip": "c2"}, "c2 is not a title"),
    ]:
        told = refused(await desk.call("video_clip_keys", **{"clip": "c1", **asked}), "VIDEO_KEYS")
        assert why in told, (asked, told)
    assert len(fake.transactions) == sends


def test_the_tools_tell_of_the_parts_and_the_runs_of_a_title() -> None:
    tools = {tool.name: tool for tool in tools_of(Videos(Config(), None))}
    told = tools["video_edit"].schema.description
    for word in (
        "stroke:{color,width}", "shadow:{color,x,y,blur}", "background:{color,padding,radius}",
        "spans style runs of the text", "occurrence?:2", "set_title {clip,text?,style?,spans?}",
        "its runs follow a new text", "title.stroke.width, title.shadow.x|y|blur",
    ):  # fmt: skip
        assert word in told, word
    keys = tools["video_clip_keys"].schema.description
    assert "title.stroke.width, title.shadow.x|y|blur, title.background.padding|radius" in keys
