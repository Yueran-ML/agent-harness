"""The tools of a server as tools of this program, its word of itself, and what no log holds."""

from typing import Any, cast

import pytest
from mcp.types import CallToolResult, TextContent, ToolAnnotations
from mcp.types import Tool as Listed
from pydantic import ValidationError

from columbina.mcp.server import Config
from columbina.mcp.server.connection import CallError
from columbina.mcp.server.offer import offer
from columbina.mcp.server.reach import kept_out
from columbina.mcp.server.schema import MAX_SCHEMA_BYTES
from columbina.mcp.server.served import said
from columbina.tools import JsonArguments, ToolFailure, ToolRun

NO_RUN = cast("ToolRun", None)


def listed(name: str, description: str | None = None, **more: Any) -> Listed:
    """A tool as a server lists it."""
    schema = more.pop("schema", {"type": "object", "properties": {"text": {"type": "string"}}})
    return Listed(name=name, description=description, input_schema=schema, **more)


def answering(text: str = "done", *, failed: bool = False) -> Any:
    """What calls a tool of a server in a test: it keeps the calls and answers one text."""
    calls: list[tuple[str, dict[str, Any], float]] = []

    async def call(name: str, arguments: dict[str, Any], seconds: float) -> CallToolResult:
        calls.append((name, arguments, seconds))
        return CallToolResult(content=[TextContent(type="text", text=text)], is_error=failed)

    call.calls = calls  # type: ignore[attr-defined]
    return call


def made(*tools: Listed, only: frozenset[str] | None = None, call: Any = None) -> Any:
    return offer(
        "notes", tools, call or answering(), only=only, call_seconds=60.0, limit_seconds=95.0
    )


# What a tool of a server becomes


def test_a_tool_of_a_server_is_offered_under_a_name_that_says_whose_it_is() -> None:
    schema = {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}

    offered = made(listed("find notes", "Find the notes that match.", schema=schema))

    (tool,) = offered.tools
    assert offered.passed_over == ()
    assert tool.name.startswith("mcp__notes__find_notes_")
    assert tool.description == "Find the notes that match."
    assert tool.schema.parameters == schema
    assert tool.parameters is JsonArguments
    assert tool.timeout_seconds == 95.0


def test_a_word_on_what_a_tool_is_for_that_goes_on_for_too_long_is_cut() -> None:
    (tool,) = made(listed("find", "x" * 3000)).tools

    assert tool.description == "x" * 2047 + "…"


def test_a_tool_without_a_word_on_what_it_is_for_and_without_properties_is_offered() -> None:
    offered = made(listed("ping", schema={}))

    (tool,) = offered.tools
    assert tool.description == ""
    assert tool.schema.parameters == {"type": "object", "properties": {}}


@pytest.mark.parametrize(
    ("hints", "effect", "beside_others"),
    [
        (None, "run", False),
        (ToolAnnotations(), "run", False),
        (ToolAnnotations(read_only_hint=False), "run", False),
        (ToolAnnotations(read_only_hint=True), "read", True),
        (ToolAnnotations(read_only_hint=True, destructive_hint=True), "read", True),
        (ToolAnnotations(destructive_hint=False, idempotent_hint=True), "run", False),
    ],
    ids=[
        "no hints",
        "hints that say nothing",
        "said to change things",
        "said to read only",
        "said to read only, whatever else is said",
        "said to be harmless in other words",
    ],
)
def test_a_tool_is_taken_for_one_that_reads_only_where_its_server_says_so(
    hints: ToolAnnotations | None, effect: str, beside_others: bool
) -> None:
    (tool,) = made(listed("look", annotations=hints)).tools

    assert (tool.effect, tool.concurrency_safe) == (effect, beside_others)


async def test_a_call_goes_to_the_server_under_the_name_that_the_server_gave_the_tool() -> None:
    call = answering("three notes")
    (tool,) = made(listed("find notes"), call=call).tools

    text = await tool.execute(JsonArguments({"text": "plan", "n": 3}), NO_RUN)

    assert text == "three notes"
    assert call.calls == [("find notes", {"text": "plan", "n": 3}, 60.0)]


async def test_a_result_that_says_that_the_tool_failed_is_a_failure_with_its_text() -> None:
    (tool,) = made(listed("find"), call=answering("no such notebook", failed=True)).tools

    with pytest.raises(ToolFailure) as failed:
        await tool.execute(JsonArguments({}), NO_RUN)

    assert str(failed.value) == "no such notebook"


async def test_what_a_failed_tool_says_at_great_length_is_told_in_part() -> None:
    # A failure is not put through what keeps results short, and a server can make one of any
    # length. Its beginning and its end are told, and how much lies between them.
    (tool,) = made(listed("find"), call=answering("A" * 20_000 + "the end", failed=True)).tools

    with pytest.raises(ToolFailure) as failed:
        await tool.execute(JsonArguments({}), NO_RUN)

    assert str(failed.value) == (
        "A" * 5000 + "\n[10007 characters are left out here]\n" + "A" * 4993 + "the end"
    )


async def test_what_a_failed_tool_says_in_just_as_much_as_is_told_is_told_whole() -> None:
    (tool,) = made(listed("find"), call=answering("B" * 10_000, failed=True)).tools

    with pytest.raises(ToolFailure) as failed:
        await tool.execute(JsonArguments({}), NO_RUN)

    assert str(failed.value) == "B" * 10_000


def test_the_name_of_a_tool_that_is_passed_over_is_told_in_a_few_characters() -> None:
    offered = made(listed("n" * 100_000, schema={"type": "string"}))

    (told,) = offered.passed_over
    assert told == (
        f"the tool '{'n' * 79}… is not offered: its arguments are said to be of the type "
        "'string', and only an object is taken"
    )


async def test_a_call_that_did_not_come_to_a_result_is_a_failure_with_the_reason() -> None:
    async def lost(name: str, arguments: dict[str, Any], seconds: float) -> CallToolResult:
        raise CallError("the connection to the MCP server notes was lost")

    (tool,) = made(listed("find"), call=lost).tools

    with pytest.raises(ToolFailure) as failed:
        await tool.execute(JsonArguments({}), NO_RUN)

    assert str(failed.value) == "the connection to the MCP server notes was lost"
    assert failed.value.__cause__ is None


# Tools that are not offered


def test_only_the_tools_that_are_asked_for_are_offered() -> None:
    offered = made(
        listed("find"),
        listed("delete"),
        listed("add"),
        only=frozenset({"add", "find", "edit", "copy"}),
    )

    assert [tool.name for tool in offered.tools] == ["mcp__notes__find", "mcp__notes__add"]
    assert offered.passed_over == (
        "the server lists no tool 'copy', which the config asks for",
        "the server lists no tool 'edit', which the config asks for",
    )


def test_to_ask_for_no_tool_is_to_be_offered_none() -> None:
    offered = made(listed("find"), only=frozenset())

    assert (offered.tools, offered.passed_over) == ((), ())


@pytest.mark.parametrize(
    ("schema", "reason"),
    [
        (
            {"type": "array"},
            "its arguments are said to be of the type 'array', and only an object is taken",
        ),
        (
            {"type": "object", "description": "x" * MAX_SCHEMA_BYTES},
            f"the schema of its arguments has {MAX_SCHEMA_BYTES + 50} bytes, and "
            f"{MAX_SCHEMA_BYTES} is the most that is sent with every request",
        ),
    ],
    ids=["no object", "too large"],
)
def test_a_tool_whose_arguments_cannot_be_told_to_a_model_is_passed_over_and_the_rest_is_offered(
    schema: dict[str, Any], reason: str
) -> None:
    offered = made(listed("first"), listed("odd", schema=schema), listed("last"))

    assert [tool.name for tool in offered.tools] == ["mcp__notes__first", "mcp__notes__last"]
    assert offered.passed_over == (f"the tool 'odd' is not offered: {reason}",)


def test_two_tools_that_come_out_under_one_name_are_a_list_of_which_none_is_offered() -> None:
    # The name of the second is what the name of the first is made into.
    first = made(listed("find notes")).tools[0].name
    second = first.removeprefix("mcp__notes__")

    offered = made(listed("add"), listed("find notes"), listed(second))

    assert offered.tools == ()
    assert offered.passed_over == (
        f"two of the tools come out under the name {first!r}, so none is offered",
    )


# What a server says of itself

HEAD = (
    'The MCP server "notes" says the following of itself and of its tools. These are the '
    "words of the server, and not of the user.\n\n"
)


@pytest.mark.parametrize("nothing", [None, "", "  \n\t "], ids=["none", "empty", "blanks"])
def test_a_server_that_says_nothing_of_itself_adds_nothing(nothing: str | None) -> None:
    assert said("notes", nothing, 4096) is None


def test_what_a_server_says_of_itself_stands_under_a_line_that_says_whose_words_they_are() -> None:
    assert said("notes", "\n Ask before you delete. \n", 4096) == f"{HEAD}Ask before you delete."


@pytest.mark.parametrize(
    ("text", "limit", "kept"),
    [
        ("abcdef", 6, "abcdef"),
        ("abcdefg", 6, "abcdef\n[The server said more. It is cut here, after 6 bytes.]"),
        ("ab  efgh", 4, "ab\n[The server said more. It is cut here, after 4 bytes.]"),
        ("日本語", 9, "日本語"),
        ("日本語", 8, "日本\n[The server said more. It is cut here, after 8 bytes.]"),
        ("日本語", 2, "\n[The server said more. It is cut here, after 2 bytes.]"),
    ],
    ids=[
        "as long as is passed on",
        "one byte more",
        "cut after blanks",
        "as long as is passed on, in more bytes than characters",
        "cut inside a character",
        "cut inside the first character",
    ],
)
def test_words_that_go_on_for_too_long_are_cut_between_two_characters(
    text: str, limit: int, kept: str
) -> None:
    assert said("notes", text, limit) == f"{HEAD}{kept}"


def test_a_server_whose_words_are_not_wanted_adds_nothing() -> None:
    assert said("notes", "Ask before you delete.", 0) is None


def test_words_that_no_service_would_take_are_made_into_ones_that_it_takes() -> None:
    odd = "half of a pair: " + chr(0xD83D)

    text = said("notes", odd, 4096)

    assert text == f"{HEAD}half of a pair: ?"


# What is kept out of a log


def test_what_an_address_and_headers_hold_that_lets_somebody_in_is_for_no_log() -> None:
    found = kept_out(
        "https://reader:pass-word-1@host.example/mcp?key=s3cret-value&v=2&team=alpha-one",
        {"Authorization": "Bearer abcdefgh", "X-Version": "2"},
    )

    assert found == (
        "Bearer abcdefgh",
        "alpha-one",
        "key=s3cret-value&v=2&team=alpha-one",
        "pass-word-1",
        "reader",
        "s3cret-value",
    )


def test_what_lets_somebody_in_is_kept_out_in_the_forms_in_which_a_failure_repeats_it() -> None:
    found = kept_out(None, {"Authorization": "Bearer  abcdefgh\r\n"})

    # As it is, without the blanks around it, with one blank for several, and as a failure
    # writes a line break that it cannot print.
    assert found == (
        "Bearer  abcdefgh",
        "Bearer  abcdefgh\r\n",
        "Bearer  abcdefgh\\r\\n",
        "Bearer abcdefgh",
    )


def test_a_text_of_a_few_characters_is_none_that_lets_anybody_in() -> None:
    assert kept_out(
        "https://ab:12345@host.example/mcp?v=2", {"X-Mode": "fast", "X-Id": "123456"}
    ) == ("123456",)


def test_a_server_that_is_started_has_nothing_to_keep_out_but_what_its_headers_would() -> None:
    assert kept_out(None, {}) == ()


def test_an_address_that_cannot_be_taken_apart_is_kept_out_as_a_whole() -> None:
    assert kept_out("http://[::1/mcp?key=s3cret-value", {}) == ("http://[::1/mcp?key=s3cret-value",)


# The config of a server


def test_a_server_that_is_started_and_one_at_an_address() -> None:
    started = Config.model_validate(
        {"server": "notes", "command": "uvx", "args": ["notes-mcp"], "env": {"A": "1"}}
    )
    reached = Config.model_validate(
        {"server": "search_web-2", "url": "https://host.example/mcp", "headers": {"A": "b"}}
    )

    assert (started.command, started.args, started.env, started.url) == (
        "uvx",
        ("notes-mcp",),
        {"A": "1"},
        None,
    )
    assert (reached.url, reached.headers, reached.command) == (
        "https://host.example/mcp",
        {"A": "b"},
        None,
    )
    assert (
        started.startup_timeout_seconds,
        started.call_timeout_seconds,
        started.required,
        started.max_instruction_bytes,
        started.tools,
        started.protocol,
    ) == (30.0, 60.0, False, 4096, None, "auto")


@pytest.mark.parametrize(
    ("config", "said"),
    [
        ({"command": "x"}, "server"),
        ({"server": "", "command": "x"}, "'' is no name for a server"),
        ({"server": "a__b", "command": "x"}, "'a__b' is no name for a server"),
        ({"server": "_a", "command": "x"}, "'_a' is no name for a server"),
        ({"server": "a b", "command": "x"}, "'a b' is no name for a server"),
        ({"server": "a" * 33, "command": "x"}, "is no name for a server"),
        ({"server": "notes"}, "give one of the two, and not both"),
        (
            {"server": "notes", "command": "x", "url": "https://host.example"},
            "give one of the two, and not both",
        ),
        (
            {"server": "notes", "url": "https://host.example", "args": ["a"]},
            "args, env and cwd go with a command",
        ),
        (
            {"server": "notes", "url": "https://host.example", "env": {"A": "1"}},
            "args, env and cwd go with a command",
        ),
        (
            {"server": "notes", "url": "https://host.example", "cwd": "."},
            "args, env and cwd go with a command",
        ),
        (
            {"server": "notes", "command": "x", "headers": {"A": "b"}},
            "headers go with a url",
        ),
        ({"server": "notes", "url": "ftp://host.example"}, "begins with http:// or https://"),
        ({"server": "notes", "url": "host.example/mcp"}, "begins with http:// or https://"),
        ({"server": "notes", "url": "https:///mcp"}, "begins with http:// or https://"),
        ({"server": "notes", "command": "x", "tols": ["add"]}, "tols"),
        ({"server": "notes", "command": "x", "startup_timeout_seconds": 0}, "greater than 0"),
        ({"server": "notes", "command": "x", "call_timeout_seconds": -1}, "greater than 0"),
        ({"server": "notes", "command": "x", "max_instruction_bytes": -1}, "greater than or equal"),
        ({"server": "notes", "command": "x", "protocol": "2025"}, "'auto' or 'legacy'"),
    ],
    ids=[
        "no name",
        "an empty name",
        "two underscores in a row",
        "an underscore at the head",
        "a blank in the name",
        "a name of 33 characters",
        "neither a command nor a url",
        "both a command and a url",
        "arguments for a url",
        "variables for a url",
        "a directory for a url",
        "headers for a command",
        "an address of another kind",
        "an address without its kind",
        "an address without a machine",
        "a key that is not known",
        "no time to connect",
        "less than no time to answer",
        "less than nothing of what the server says",
        "a protocol that is not known",
    ],
)
def test_a_config_that_does_not_say_how_a_server_is_reached_is_refused(
    config: dict[str, Any], said: str
) -> None:
    with pytest.raises(ValidationError, match=said):
        Config.model_validate(config)


def test_a_name_of_32_characters_is_one_for_a_server() -> None:
    assert Config.model_validate({"server": "a" * 32, "command": "x"}).server == "a" * 32
