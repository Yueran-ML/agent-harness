"""A tool: what a model is told about it, and how its arguments are read."""

import datetime
from pathlib import Path
from typing import cast

import pytest
from pydantic import BaseModel, ConfigDict, Field

from tack.kernel import Context
from tack.llm import ToolSchema
from tack.session import CORE_EVENTS, Session
from tack.tools import Tool, ToolFailure, ToolRun


class ReadArgs(BaseModel):
    """Arguments of the read tool."""

    path: str = Field(description="The file to read.")
    offset: int = Field(default=0, description="The first line to return.")
    limit: int | None = None


class Heading(BaseModel):
    title: str
    level: int = 1


class NoteArgs(BaseModel):
    title: str
    heading: Heading | None = None
    tags: list[str] = []


async def read_file(args: ReadArgs, run: ToolRun) -> str:
    return f"contents of {args.path}"


async def take_note(args: NoteArgs, run: ToolRun) -> int:
    return len(args.tags)


READ = Tool(name="read", description="Read a text file.", parameters=ReadArgs, execute=read_file)
NOTE = Tool(name="note", description="Take a note.", parameters=NoteArgs, execute=take_note)


def test_a_tool_tells_a_model_its_name_its_purpose_and_its_parameters() -> None:
    assert READ.schema == ToolSchema(
        name="read",
        description="Read a text file.",
        parameters={
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The file to read."},
                "offset": {
                    "type": "integer",
                    "default": 0,
                    "description": "The first line to return.",
                },
                "limit": {"anyOf": [{"type": "integer"}, {"type": "null"}], "default": None},
            },
            "required": ["path"],
        },
    )


def test_the_schema_drops_titles_but_keeps_a_parameter_that_is_called_title() -> None:
    parameters = NOTE.schema.parameters

    assert parameters == {
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "heading": {"anyOf": [{"$ref": "#/$defs/Heading"}, {"type": "null"}], "default": None},
            "tags": {"type": "array", "items": {"type": "string"}, "default": []},
        },
        "required": ["title"],
        "$defs": {
            "Heading": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "level": {"type": "integer", "default": 1},
                },
                "required": ["title"],
            }
        },
    }


@pytest.mark.parametrize("name", ["", "read file", "read.file", "读", "x" * 65])
def test_a_tool_needs_a_name_that_every_provider_accepts(name: str) -> None:
    with pytest.raises(ValueError, match="is not a usable tool name"):
        Tool(name=name, description="Read.", parameters=ReadArgs, execute=read_file)


@pytest.mark.parametrize("name", ["read", "read_file", "Read-2", "x" * 64])
def test_letters_digits_dashes_and_underscores_make_a_name(name: str) -> None:
    assert Tool(name=name, description="Read.", parameters=ReadArgs, execute=read_file).name == name


def test_the_parameters_of_a_tool_must_be_a_pydantic_model() -> None:
    with pytest.raises(TypeError, match="the parameters of the tool 'read' must be a pydantic"):
        Tool(
            name="read",
            description="Read.",
            parameters=cast("type[ReadArgs]", dict),
            execute=read_file,
        )


def test_the_arguments_are_read_into_the_model_of_the_parameters() -> None:
    args = READ.parse('{"path": "a.txt", "limit": 10}')

    assert args == ReadArgs(path="a.txt", limit=10)


def test_arguments_of_the_wrong_json_type_are_converted_when_they_can_be() -> None:
    assert READ.parse('{"path": "a.txt", "offset": "5"}').offset == 5


@pytest.mark.parametrize("arguments", ["", "  ", "{}"])
def test_missing_arguments_stand_for_an_empty_object(arguments: str) -> None:
    class Nothing(BaseModel):
        verbose: bool = False

    async def run_it(args: Nothing, run: ToolRun) -> str:
        return "done"

    tool = Tool(name="status", description="Show status.", parameters=Nothing, execute=run_it)

    assert tool.parse(arguments) == Nothing()


def test_arguments_that_are_not_json_are_refused_with_an_explanation() -> None:
    with pytest.raises(ToolFailure) as caught:
        READ.parse('{"path": "a.txt"')

    assert caught.value.code == "INVALID_ARGS"
    assert str(caught.value).startswith("The arguments are not valid JSON: ")
    assert "line 1 column 17" in str(caught.value)


@pytest.mark.parametrize(
    ("arguments", "kind"),
    [('["a.txt"]', "a list"), ('"a.txt"', "a string"), ("3", "a number"), ("null", "null")],
)
def test_arguments_that_are_not_an_object_are_refused(arguments: str, kind: str) -> None:
    with pytest.raises(ToolFailure) as caught:
        READ.parse(arguments)

    assert caught.value.code == "INVALID_ARGS"
    assert str(caught.value) == f"The arguments must be a JSON object, not {kind}."


def test_arguments_that_do_not_fit_are_refused_field_by_field() -> None:
    with pytest.raises(ToolFailure) as caught:
        READ.parse('{"offset": "soon", "limit": 3}')

    assert caught.value.code == "INVALID_ARGS"
    assert str(caught.value) == (
        "The arguments do not fit the tool read:\n"
        "- path: Field required\n"
        "- offset: Input should be a valid integer, unable to parse string as an integer"
    )


def test_a_tool_says_whether_a_call_may_run_beside_others() -> None:
    def only_small(args: ReadArgs) -> bool:
        return args.limit is not None

    exclusive = Tool(name="a", description="A.", parameters=ReadArgs, execute=read_file)
    shared = Tool(
        name="b", description="B.", parameters=ReadArgs, execute=read_file, concurrency_safe=True
    )
    choosy = Tool(
        name="c",
        description="C.",
        parameters=ReadArgs,
        execute=read_file,
        concurrency_safe=only_small,
    )

    assert not exclusive.is_concurrency_safe(ReadArgs(path="a.txt"))
    assert shared.is_concurrency_safe(ReadArgs(path="a.txt"))
    assert choosy.is_concurrency_safe(ReadArgs(path="a.txt", limit=5))
    assert not choosy.is_concurrency_safe(ReadArgs(path="a.txt"))


def test_a_failure_carries_a_code_for_programs() -> None:
    assert ToolFailure("no such file").code == "TOOL_ERROR"
    assert ToolFailure("not allowed", code="DENIED").code == "DENIED"
    assert str(ToolFailure("no such file")) == "no such file"


async def test_a_run_tells_a_tool_about_its_call(ctx: Context, tmp_path: Path) -> None:
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})

    run = ToolRun(call_id="call-1", cwd=tmp_path, session=session)

    assert run.call_id == "call-1"
    assert run.cwd == tmp_path
    assert run.session is session
    assert not run.concludes_turn
    run.conclude_turn()
    assert run.concludes_turn


def test_arguments_are_read_as_json_so_that_strict_parameters_work() -> None:
    class Strict(BaseModel):
        model_config = ConfigDict(strict=True)

        when: datetime.date
        points: tuple[int, ...]

    async def plot(args: Strict, run: ToolRun) -> str:
        return "ok"

    tool = Tool(name="plot", description="Plot.", parameters=Strict, execute=plot)

    args = tool.parse('{"when": "2026-10-09", "points": [1, 2]}')

    assert args == Strict(when=datetime.date(2026, 10, 9), points=(1, 2))
    with pytest.raises(ToolFailure, match=r"points\.0"):
        tool.parse('{"when": "2026-10-09", "points": ["1"]}')
