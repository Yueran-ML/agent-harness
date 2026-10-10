"""A tool: what a model is told about it, and how its arguments are read."""

import datetime
from pathlib import Path
from typing import Any, cast

import pytest
from pydantic import BaseModel, ConfigDict, Field

from columbina.kernel import Context
from columbina.llm import ToolSchema
from columbina.session import CORE_EVENTS, Session
from columbina.tools import JsonArguments, Tool, ToolFailure, ToolRun


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


# A tool whose parameters are described elsewhere

FOREIGN: dict[str, Any] = {
    "type": "object",
    "title": "SearchArguments",
    "properties": {
        "query": {"type": "string", "title": "Query", "description": "What to look for."},
        "limit": {"$ref": "#/$defs/Limit"},
    },
    "required": ["query"],
    "$defs": {"Limit": {"type": "integer", "minimum": 1, "title": "Limit"}},
    "x-vendor": {"anything": [1, 2.5, None, True]},
}


async def passed_on(args: JsonArguments, run: ToolRun) -> dict[str, Any]:
    return args.root


def foreign(**options: Any) -> Tool[JsonArguments, dict[str, Any]]:
    return Tool(
        name="search",
        description="Search somewhere else.",
        parameters=JsonArguments,
        execute=passed_on,
        **options,
    )


def test_a_tool_can_bring_the_schema_of_its_parameters_and_the_model_is_told_it_as_it_is() -> None:
    tool = foreign(input_schema=FOREIGN)

    assert tool.schema == ToolSchema(
        name="search", description="Search somewhere else.", parameters=FOREIGN
    )
    # Nothing of it is tidied up: it is not ours to change.
    assert tool.schema.parameters["title"] == "SearchArguments"
    assert tool.schema.parameters["x-vendor"] == {"anything": [1, 2.5, None, True]}


def test_what_a_model_is_told_does_not_change_when_the_schema_that_was_given_does() -> None:
    given: dict[str, Any] = {"type": "object", "properties": {"a": {"type": "string"}}}
    tool = foreign(input_schema=given)

    # Whoever gave it goes on using it for something else.
    given["properties"]["b"] = {"type": "string"}

    assert tool.schema.parameters == {"type": "object", "properties": {"a": {"type": "string"}}}


def test_the_arguments_of_such_a_tool_are_whatever_object_the_model_wrote() -> None:
    tool = foreign(input_schema=FOREIGN)

    args = tool.parse('{"query": "x", "limit": "many", "more": {"deep": [1, null, 2.5, true]}}')

    # Whether they fit the schema is for whoever gets them to say.
    assert args.root == {"query": "x", "limit": "many", "more": {"deep": [1, None, 2.5, True]}}
    assert tool.parse("").root == {}


@pytest.mark.parametrize(
    ("arguments", "kind"), [("[1]", "a list"), ('"query"', "a string"), ("null", "null")]
)
def test_the_arguments_of_such_a_tool_still_have_to_be_an_object(arguments: str, kind: str) -> None:
    with pytest.raises(ToolFailure, match=f"must be a JSON object, not {kind}") as raised:
        foreign(input_schema=FOREIGN).parse(arguments)

    assert raised.value.code == "INVALID_ARGS"


def test_a_schema_of_its_own_and_a_model_of_the_parameters_do_not_go_together() -> None:
    async def read(args: ReadArgs, run: ToolRun) -> str:
        return args.path

    with pytest.raises(TypeError, match="brings a schema of its own, so its parameters"):
        Tool(
            name="read",
            description="Read.",
            parameters=ReadArgs,
            execute=read,
            input_schema=FOREIGN,
        )


def test_arguments_of_which_nothing_is_known_need_a_schema_for_the_model() -> None:
    with pytest.raises(TypeError, match="takes whatever object the model writes, so it needs"):
        foreign()


def test_a_schema_that_is_no_object_schema_is_refused() -> None:
    with pytest.raises(ValueError, match="the schema of the tool 'search' must describe an object"):
        foreign(input_schema={"type": "string"})


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


# What a call does to the world


def test_a_tool_says_nothing_about_its_effect_unless_it_is_told() -> None:
    plain = Tool(name="read", description="Read.", parameters=ReadArgs, execute=read_file)

    assert plain.effect == "none"
    assert plain.target is None


@pytest.mark.parametrize("effect", ["none", "read", "write", "run"])
def test_a_tool_can_say_what_a_call_does_to_the_world(effect: Any) -> None:
    told = Tool(
        name="read", description="Read.", parameters=ReadArgs, execute=read_file, effect=effect
    )

    assert told.effect == effect


def test_an_effect_that_does_not_exist_is_refused() -> None:
    with pytest.raises(ValueError, match="'destroy' is not an effect"):
        Tool(
            name="read",
            description="Read.",
            parameters=ReadArgs,
            execute=read_file,
            effect=cast("Any", "destroy"),
        )


def test_what_a_tool_is_told_about_a_call_cannot_be_changed(ctx: Context, tmp_path: Path) -> None:
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})
    run = ToolRun(call_id="call-1", cwd=tmp_path, session=session, target="a.txt")
    change: Any = setattr

    for name, value in (
        ("cwd", tmp_path / "elsewhere"),
        ("call_id", "another"),
        ("session", None),
        ("target", "b.txt"),
    ):
        with pytest.raises(AttributeError):
            change(run, name, value)

    assert (run.call_id, run.cwd, run.session, run.target) == ("call-1", tmp_path, session, "a.txt")


def test_a_tool_is_told_of_no_target_unless_there_is_one(ctx: Context, tmp_path: Path) -> None:
    session = Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})

    assert ToolRun(call_id="call-1", cwd=tmp_path, session=session).target is None
