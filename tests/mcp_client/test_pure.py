"""What a tool of an MCP server looks like to a model: its name, its schema, what it gave back."""

import base64
import json
import re
import string

import pytest
from hypothesis import assume, given
from hypothesis import strategies as st
from mcp.types import (
    AudioContent,
    BlobResourceContents,
    CallToolResult,
    EmbeddedResource,
    ImageContent,
    ResourceLink,
    TextContent,
    TextResourceContents,
)

from columbina.mcp.server.content import NOTHING, shown
from columbina.mcp.server.names import MAX_NAME, PREFIX, SERVER, tool_name
from columbina.mcp.server.schema import (
    MAX_DESCRIPTION,
    MAX_SCHEMA_BYTES,
    described,
    refused,
    repaired,
)
from columbina.tools import JsonArguments, Tool, ToolRun

A_TOOL_NAME = re.compile(r"[A-Za-z0-9_-]{1,64}")
PLAIN = string.ascii_letters + string.digits + "-"

servers = (
    st.lists(st.text(alphabet=PLAIN, min_size=1, max_size=8), min_size=1, max_size=4)
    .map("_".join)
    .filter(lambda name: len(name) <= 32)
)
"""Names that a server may have here."""

# Names


def test_a_tool_is_named_after_its_server_and_by_its_own_name() -> None:
    assert tool_name("files", "read_file") == "mcp__files__read_file"
    assert tool_name("my-server", "get-item") == "mcp__my-server__get-item"
    assert tool_name("a_b", "C9") == "mcp__a_b__C9"
    assert PREFIX == "mcp__"


def test_a_name_of_just_the_length_that_is_allowed_stays_as_it_is() -> None:
    fits, too_long = "x" * 56, "x" * 57

    assert tool_name("s", fits) == "mcp__s__" + fits
    assert len(tool_name("s", fits)) == MAX_NAME == 64
    # The first 51 characters, an underscore, and twelve digits that are its alone.
    assert tool_name("s", too_long) == "mcp__s__" + "x" * 43 + "_065b956fb9c7"
    assert len(tool_name("s", too_long)) == 64


def test_a_character_that_no_name_may_have_becomes_an_underscore_and_the_name_gets_digits() -> None:
    # The digits are those of the server, a zero byte and the name of the tool.
    assert tool_name("s", "a.b") == "mcp__s__a_b_407e8e5cd262"
    assert tool_name("s", "a.b") == tool_name("s", "a.b")


def test_names_that_were_made_alike_are_told_apart() -> None:
    names = [tool_name("s", raw) for raw in ("a_b", "a-b", "a.b", "a b", "a/b", "a" + chr(0xE9))]

    assert names[:2] == ["mcp__s__a_b", "mcp__s__a-b"]
    assert len(set(names)) == len(names)
    assert all(name.startswith("mcp__s__a_b_") for name in names[2:5])


def test_the_digits_are_those_of_the_server_and_the_tool_together() -> None:
    # Without something between the two names, both pairs would be the same letters.
    assert tool_name("ab", "c.d")[-12:] != tool_name("a", "bc.d")[-12:]
    assert tool_name("a", "c.d")[-12:] != tool_name("b", "c.d")[-12:]


def test_a_name_that_is_changed_and_long_is_cut_and_keeps_its_server() -> None:
    server = "s" * 32
    name = tool_name(server, "read file " * 20)

    assert len(name) == 64
    assert name.startswith(f"mcp__{server}__read_file_re")
    assert re.fullmatch(r"[0-9a-f]{12}", name[-12:])
    assert name[51] == "_"


@pytest.mark.parametrize(
    "raw",
    ["", "_", "__", "a__b", chr(0xD800), "名字", "a\nb", " "],
    ids=[
        "nothing",
        "an underscore",
        "two",
        "two inside",
        "half a pair",
        "han",
        "a line",
        "a blank",
    ],
)
def test_any_name_that_a_server_gives_a_tool_becomes_a_name_that_a_tool_may_have(raw: str) -> None:
    name = tool_name("s", raw)

    assert A_TOOL_NAME.fullmatch(name)
    assert name.startswith("mcp__s__")


def test_the_name_of_a_tool_is_one_that_the_tools_take() -> None:
    async def nothing(arguments: JsonArguments, run: ToolRun) -> str:
        return ""

    for raw in ("read_file", "x" * 57, "a.b", ""):
        Tool(
            name=tool_name("s" * 32, raw),
            description="",
            parameters=JsonArguments,
            input_schema=repaired({}),
            execute=nothing,
        )


@given(server=servers, raw=st.text(max_size=80))
def test_whatever_a_tool_is_called_its_name_is_one_that_a_service_takes(
    server: str, raw: str
) -> None:
    name = tool_name(server, raw)

    assert A_TOOL_NAME.fullmatch(name)
    assert name.startswith(f"mcp__{server}__")


@given(server=servers, one=st.text(max_size=80), other=st.text(max_size=80))
def test_two_tools_of_a_server_never_share_a_name(server: str, one: str, other: str) -> None:
    assume(one != other)
    # A tool whose own name already ends in the digits of another one is left aside: a server
    # would have to work them out to get there, and a list with such a pair in it is not used.
    assume(not re.search(r"_[0-9a-f]{12}\Z", one + "|" + other))

    assert tool_name(server, one) != tool_name(server, other)


@given(
    one=servers,
    other=servers,
    first=st.text(alphabet=PLAIN + "_", max_size=20),
    second=st.text(alphabet=PLAIN + "_", max_size=20),
)
def test_two_servers_never_share_the_name_of_a_tool(
    one: str, other: str, first: str, second: str
) -> None:
    assume(one != other)

    assert tool_name(one, first) != tool_name(other, second)


@pytest.mark.parametrize(
    "name",
    ["a", "files", "my-server", "a_b", "A1-b_c", "-", "9", "x" * 32, "a_b_c"],
)
def test_what_a_server_may_be_called(name: str) -> None:
    assert SERVER.fullmatch(name)


@pytest.mark.parametrize(
    "name",
    ["", "x" * 33, "a__b", "_a", "a_", "_", "a b", "a.b", "a" + chr(0xE9), "a\n", "a/b"],
    ids=[
        "nothing",
        "too long",
        "two underscores",
        "an underscore first",
        "an underscore last",
        "an underscore alone",
        "a blank",
        "a dot",
        "a letter with an accent",
        "the end of a line",
        "a slash",
    ],
)
def test_what_a_server_may_not_be_called(name: str) -> None:
    assert SERVER.fullmatch(name) is None


# What a tool is for, and its arguments


def test_what_a_tool_is_for_is_told_as_the_server_says_it_up_to_a_length() -> None:
    exact, longer = "x" * MAX_DESCRIPTION, "x" * (MAX_DESCRIPTION + 1)

    assert MAX_DESCRIPTION == 2048
    assert described("Reads a file.\nIt is read whole.") == "Reads a file.\nIt is read whole."
    assert described(None) == ""
    assert described("") == ""
    assert described(exact) == exact
    assert described(longer) == "x" * (MAX_DESCRIPTION - 1) + "…"
    assert len(described(longer)) == MAX_DESCRIPTION


def test_a_schema_is_given_what_a_service_asks_of_it_and_is_otherwise_left_alone() -> None:
    bare = {"required": ["path"], "$defs": {"unit": {"enum": ["c", "f"]}}}
    whole = {
        "type": "object",
        "properties": {"path": {"type": "string", "title": "Path"}},
        "additionalProperties": False,
    }

    assert repaired({}) == {"type": "object", "properties": {}}
    assert repaired(bare) == {
        "required": ["path"],
        "$defs": {"unit": {"enum": ["c", "f"]}},
        "type": "object",
        "properties": {},
    }
    assert repaired(whole) == whole
    assert repaired({"type": "object"}) == {"type": "object", "properties": {}}
    assert repaired({"properties": {"a": {}}}) == {"properties": {"a": {}}, "type": "object"}
    # What the server sent is not changed by this.
    assert bare == {"required": ["path"], "$defs": {"unit": {"enum": ["c", "f"]}}}


def test_a_schema_that_says_what_it_takes_is_not_made_to_say_something_else() -> None:
    assert repaired({"type": "string"}) == {"type": "string", "properties": {}}
    assert repaired({"type": None})["type"] is None


def test_a_tool_whose_arguments_are_an_object_is_offered() -> None:
    assert refused(repaired({})) is None
    assert refused({"type": "object", "properties": {"a": {"type": "string"}}}) is None


@pytest.mark.parametrize(
    ("kind", "said"),
    [("string", "'string'"), (["object", "null"], "['object', 'null']"), (None, "None")],
)
def test_a_tool_whose_arguments_are_no_object_is_not_offered(kind: object, said: str) -> None:
    assert refused({"type": kind, "properties": {}}) == (
        f"its arguments are said to be of the type {said}, and only an object is taken"
    )
    assert refused({"properties": {}}) == (
        "its arguments are said to be of the type None, and only an object is taken"
    )


def test_a_schema_that_is_too_large_to_be_sent_with_every_request_is_not_offered() -> None:
    def of_size(size: int) -> dict[str, object]:
        empty = len(json.dumps({"type": "object", "description": ""}, separators=(",", ":")))
        return {"type": "object", "description": "x" * (size - empty)}

    assert MAX_SCHEMA_BYTES == 16384
    assert refused(of_size(MAX_SCHEMA_BYTES)) is None
    assert refused(of_size(MAX_SCHEMA_BYTES + 1)) == (
        "the schema of its arguments has 16385 bytes, and 16384 is the most that is sent with "
        "every request"
    )


def test_the_size_of_a_schema_is_counted_in_the_bytes_that_are_sent() -> None:
    def of_letters(count: int, letter: str) -> dict[str, object]:
        return {"type": "object", "description": letter * count}

    plain = len(json.dumps(of_letters(0, "x"), separators=(",", ":")))
    room = MAX_SCHEMA_BYTES - plain

    # A letter that takes three bytes is counted as three, and is not written as an escape.
    assert refused(of_letters(room // 3, "名")) is None
    assert refused(of_letters(room // 3 + 1, "名")) is not None
    assert refused(of_letters(room, "x")) is None


def test_a_schema_that_is_no_json_is_not_offered() -> None:
    assert refused({"type": "object", "properties": {"a": {1, 2}}}) == (
        "the schema of its arguments cannot be written as JSON"
    )
    around: dict[str, object] = {"type": "object"}
    around["properties"] = around
    assert refused(around) == "the schema of its arguments cannot be written as JSON"


# What a call gave back


def coded(data: bytes) -> str:
    return base64.b64encode(data).decode()


def text(words: str) -> TextContent:
    return TextContent(type="text", text=words)


def result(*blocks: object, structured: object = None, failed: bool = False) -> CallToolResult:
    return CallToolResult.model_validate(
        {"content": list(blocks), "structured_content": structured, "is_error": failed}
    )


PICTURE = ImageContent(type="image", data=coded(b"abc"), mime_type="image/png")
SOUND = AudioContent(type="audio", data=coded(b"ab"), mime_type="audio/wav")
LINK = ResourceLink(type="resource_link", name="notes", uri="file:///notes.txt")
A_TEXT = EmbeddedResource(
    type="resource", resource=TextResourceContents(uri="file:///a.txt", text="hello\nworld")
)
A_FILE = EmbeddedResource(
    type="resource",
    resource=BlobResourceContents(uri="file:///a.bin", blob=coded(b"a"), mime_type="x/y"),
)


def test_a_text_is_read_as_it_is_and_several_stand_on_lines_of_their_own() -> None:
    assert shown(result(text("one"))) == "one"
    assert shown(result(text("one"), text("two\nthree"), text(""))) == "one\ntwo\nthree\n"


def test_a_picture_and_a_sound_are_a_line_that_says_what_was_left_out() -> None:
    assert shown(result(PICTURE)) == "[a picture is left out here: image/png, 3 bytes]"
    assert shown(result(SOUND)) == "[a sound is left out here: audio/wav, 2 bytes]"


@pytest.mark.parametrize(
    ("data", "large"),
    [
        (b"", "0 bytes"),
        (b"a", "1 byte"),
        (b"ab", "2 bytes"),
        (b"abc", "3 bytes"),
        (b"abcd", "4 bytes"),
    ],
)
def test_how_large_what_was_left_out_is_is_said_in_its_bytes(data: bytes, large: str) -> None:
    picture = ImageContent(type="image", data=coded(data), mime_type="image/png")

    assert shown(result(picture)) == f"[a picture is left out here: image/png, {large}]"


def test_a_link_to_a_resource_is_its_name_and_its_address() -> None:
    assert shown(result(LINK)) == "Resource link: notes (file:///notes.txt)"


def test_a_resource_that_is_text_is_its_address_and_the_text() -> None:
    assert shown(result(A_TEXT)) == "file:///a.txt\nhello\nworld"


def test_a_resource_that_is_no_text_is_a_line_that_says_what_was_left_out() -> None:
    unnamed = EmbeddedResource(
        type="resource", resource=BlobResourceContents(uri="file:///b.bin", blob=coded(b"abcde"))
    )

    assert shown(result(A_FILE)) == "[the resource file:///a.bin is left out here: x/y, 1 byte]"
    assert shown(result(unnamed)) == (
        "[the resource file:///b.bin is left out here: of no named type, 5 bytes]"
    )


def test_what_was_given_back_stands_in_the_order_in_which_it_came() -> None:
    assert shown(result(PICTURE, text("first"), LINK, A_TEXT, SOUND, A_FILE)) == "\n".join(
        [
            "[a picture is left out here: image/png, 3 bytes]",
            "first",
            "Resource link: notes (file:///notes.txt)",
            "file:///a.txt",
            "hello",
            "world",
            "[a sound is left out here: audio/wav, 2 bytes]",
            "[the resource file:///a.bin is left out here: x/y, 1 byte]",
        ]
    )


def test_structured_content_is_read_where_there_is_no_text() -> None:
    structured = {"temperature": 21.5, "unit": "摄氏度", "ok": True, "tags": ["a", None]}
    as_json = '{"temperature": 21.5, "unit": "摄氏度", "ok": true, "tags": ["a", null]}'

    assert shown(result(structured=structured)) == as_json
    assert shown(result(PICTURE, SOUND, LINK, A_FILE, structured=structured)) == "\n".join(
        [
            "[a picture is left out here: image/png, 3 bytes]",
            "[a sound is left out here: audio/wav, 2 bytes]",
            "Resource link: notes (file:///notes.txt)",
            "[the resource file:///a.bin is left out here: x/y, 1 byte]",
            as_json,
        ]
    )
    # Structured content that says nothing in particular is content all the same.
    assert shown(result(structured={})) == "{}"
    assert shown(result(structured=0)) == "0"
    assert shown(result(structured=[])) == "[]"


def test_structured_content_is_not_read_beside_a_text() -> None:
    structured = {"temperature": 21.5}

    assert (
        shown(result(text("It is 21.5 degrees."), structured=structured)) == "It is 21.5 degrees."
    )
    assert shown(result(A_TEXT, structured=structured)) == "file:///a.txt\nhello\nworld"
    assert shown(result(PICTURE, text(""), structured=structured)) == (
        "[a picture is left out here: image/png, 3 bytes]\n"
    )
    # A text counts wherever it stands among what was given back.
    assert shown(result(text("21.5"), PICTURE, structured=structured)) == (
        "21.5\n[a picture is left out here: image/png, 3 bytes]"
    )


def test_structured_content_that_is_no_json_is_read_as_what_it_says_of_itself() -> None:
    assert shown(result(structured={"when": complex(1, 2)})) == '{"when": "(1+2j)"}'


def test_a_result_of_nothing_says_so() -> None:
    assert NOTHING == "[the tool gave back nothing that can be shown]"
    assert shown(result()) == NOTHING
    assert shown(result(structured=None)) == NOTHING


def test_a_result_that_the_server_calls_an_error_is_read_like_any_other() -> None:
    assert shown(result(text("No such file."), failed=True)) == "No such file."
    assert shown(result(failed=True)) == NOTHING
