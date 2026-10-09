"""Requests as text."""

from hypothesis import given
from hypothesis import strategies as st

from columbina.llm import (
    AssistantMessage,
    CallConfig,
    LlmRequest,
    ReasoningBlock,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    UserMessage,
    render_request,
    render_requests,
)

MODEL = CallConfig(provider="deepseek", model="deepseek-flash")


def tool(name: str) -> ToolSchema:
    return ToolSchema(name=name, description="x", parameters={"type": "object"})


def asking(text: str) -> LlmRequest:
    return LlmRequest(config=MODEL, messages=(UserMessage(content=(TextBlock(text=text),)),))


# One request


def test_a_message_is_a_line_for_its_role_and_a_line_for_each_block() -> None:
    request = LlmRequest(
        config=MODEL,
        tools=(tool("read"), tool("grep")),
        messages=(
            SystemMessage(content=(TextBlock(text="You are helpful."),)),
            UserMessage(content=(TextBlock(text="Read a.txt."), TextBlock(text="Then b.txt."))),
            AssistantMessage(
                content=(
                    ReasoningBlock(text="I need the file.", replay_state={"adapter": "deepseek"}),
                    TextBlock(text="I will look."),
                    ToolCallBlock(call_id="call-1", name="read", arguments='{"path": "a.txt"}'),
                    ToolCallBlock(call_id="call-2", name="read", arguments='{"path": '),
                )
            ),
            ToolMessage(call_id="call-1", content=(TextBlock(text="line 1\nline 2"),)),
            ToolMessage(call_id="call-2", content=(TextBlock(text="no such file"),), is_error=True),
            AssistantMessage(content=()),
        ),
    )

    assert render_request(request).split("\n") == [
        "to deepseek/deepseek-flash, tools: read, grep",
        "1 system",
        "    text: 'You are helpful.'",
        "2 user",
        "    text: 'Read a.txt.'",
        "    text: 'Then b.txt.'",
        "3 assistant",
        "    reasoning: 'I need the file.'",
        "    text: 'I will look.'",
        """    call read '{"path": "a.txt"}' as call-1""",
        """    call read '{"path": ' as call-2""",
        "4 tool, result of call-1",
        "    text: 'line 1\\nline 2'",
        "5 tool, failed result of call-2",
        "    text: 'no such file'",
        "6 assistant",
    ]


def test_a_request_without_tools_says_so() -> None:
    rendered = render_request(asking("hi"))

    assert rendered == "to deepseek/deepseek-flash, no tools\n1 user\n    text: 'hi'"


def test_no_tools_and_a_tool_called_none_are_told_apart() -> None:
    bare = asking("hi")
    named = bare.model_copy(update={"tools": (tool("none"),)})

    assert render_request(bare).splitlines()[0] == "to deepseek/deepseek-flash, no tools"
    assert render_request(named).splitlines()[0] == "to deepseek/deepseek-flash, tools: none"


def test_what_would_not_show_is_written_as_an_escape() -> None:
    line_separator, wide_space = chr(0x2028), chr(0xA0)
    text = f"tab\there, break\r\nhere, a\\nb, line{line_separator}here, space{wide_space}here "

    assert render_request(asking(text)).split("\n")[2] == (
        "    text: 'tab\\there, break\\r\\nhere, a\\\\nb, line\\u2028here, space\\xa0here '"
    )


def test_what_can_be_printed_stays_as_it_is() -> None:
    text = 'Read "笔记.txt" and say héllo to {everyone} 🙂'

    assert render_request(asking(text)).split("\n")[2] == f"    text: '{text}'"


def test_a_name_that_holds_a_line_break_stays_on_its_line() -> None:
    call = ToolCallBlock(call_id="id\t1", name="read\nfile", arguments="{}")
    request = LlmRequest(
        config=CallConfig(provider="deep\nseek", model="flash\\2"),
        tools=(tool("read\rfile"),),
        messages=(AssistantMessage(content=(call,)), ToolMessage(call_id="id\t1", content=())),
    )

    assert render_request(request).split("\n") == [
        "to deep\\nseek/flash\\\\2, tools: read\\rfile",
        "1 assistant",
        "    call read\\nfile '{}' as id\\t1",
        "2 tool, result of id\\t1",
    ]


@given(texts=st.lists(st.text(), min_size=1, max_size=4))
def test_no_text_changes_the_number_of_lines(texts: list[str]) -> None:
    blocks = tuple(TextBlock(text=text) for text in texts)
    calls = tuple(ToolCallBlock(call_id=text, name=text, arguments=text) for text in texts)
    request = LlmRequest(
        config=CallConfig(provider=texts[0], model=texts[0]),
        tools=tuple(tool(text) for text in texts),
        messages=(
            UserMessage(content=blocks),
            AssistantMessage(content=(*blocks, *calls)),
            ToolMessage(call_id=texts[0], content=blocks),
        ),
    )

    rendered = render_request(request)

    assert len(rendered.splitlines()) == 1 + 3 + 4 * len(texts)
    assert all(line.isprintable() for line in rendered.split("\n"))


@given(first=st.text(), second=st.text())
def test_two_texts_are_never_written_the_same(first: str, second: str) -> None:
    same = render_request(asking(first)) == render_request(asking(second))

    assert same == (first == second)


# Several requests


def said(
    *texts: str, model: str = "deepseek-flash", tools: tuple[ToolSchema, ...] = ()
) -> LlmRequest:
    """A request whose messages are user messages with these texts."""
    return LlmRequest(
        config=CallConfig(provider="deepseek", model=model),
        tools=tools,
        messages=tuple(UserMessage(content=(TextBlock(text=text),)) for text in texts),
    )


def test_a_request_that_only_adds_to_the_one_before_is_written_as_what_it_adds() -> None:
    grown = said("one", "two", "three")

    assert render_requests([said("one"), grown, grown]).split("\n") == [
        "request 1 to deepseek/deepseek-flash, no tools",
        "1 user",
        "    text: 'one'",
        "",
        "request 2 to deepseek/deepseek-flash, no tools",
        "    as request 1 up to message 1",
        "2 user",
        "    text: 'two'",
        "3 user",
        "    text: 'three'",
        "",
        "request 3 to deepseek/deepseek-flash, no tools",
        "    as request 2 up to message 3",
    ]


def test_a_request_that_changes_what_was_said_is_written_in_full() -> None:
    rendered = render_requests([said("one", "two"), said("one", "TWO", "three"), said("one")])

    assert rendered.split("\n") == [
        "request 1 to deepseek/deepseek-flash, no tools",
        "1 user",
        "    text: 'one'",
        "2 user",
        "    text: 'two'",
        "",
        "request 2 to deepseek/deepseek-flash, no tools",
        "1 user",
        "    text: 'one'",
        "2 user",
        "    text: 'TWO'",
        "3 user",
        "    text: 'three'",
        "",
        "request 3 to deepseek/deepseek-flash, no tools",
        "1 user",
        "    text: 'one'",
    ]


def test_every_request_names_its_model_and_its_tools() -> None:
    rendered = render_requests(
        [said("one"), said("one", "two", model="deepseek-v4-pro", tools=(tool("read"),))]
    )

    assert rendered.split("\n") == [
        "request 1 to deepseek/deepseek-flash, no tools",
        "1 user",
        "    text: 'one'",
        "",
        "request 2 to deepseek/deepseek-v4-pro, tools: read",
        "    as request 1 up to message 1",
        "2 user",
        "    text: 'two'",
    ]


def test_requests_without_messages_are_written_without_any() -> None:
    assert render_requests([]) == ""
    assert render_requests([said(), said()]) == (
        "request 1 to deepseek/deepseek-flash, no tools\n\n"
        "request 2 to deepseek/deepseek-flash, no tools"
    )
