"""What a model said, reply by reply."""

from columbina.agent import CUT_OFF, replies
from columbina.llm import AssistantMessage, TextBlock, ToolCallBlock, Usage, UserMessage
from columbina.session import AssistantMessageEvent, EventData, TurnEndEvent, UserMessageEvent


def reply(text: str, *, cut: bool = False) -> AssistantMessageEvent:
    """What a model said in one reply, which ended by itself or was cut off."""
    content = (TextBlock(text=text),) if text else ()
    return AssistantMessageEvent(
        message=AssistantMessage(content=content),
        usage=Usage(),
        finish_reason="max-tokens" if cut else "stop",
    )


def calling() -> AssistantMessageEvent:
    """A reply that says nothing and calls a tool."""
    call = ToolCallBlock(call_id="call-1", name="read", arguments="{}")
    return AssistantMessageEvent(
        message=AssistantMessage(content=(call,)), usage=Usage(), finish_reason="tool-calls"
    )


def message(text: str, source: str = "user") -> UserMessageEvent:
    return UserMessageEvent(message=UserMessage(content=(TextBlock(text=text),)), source=source)


GO_ON = message("Go on.", CUT_OFF)


def test_every_reply_is_told_as_it_was_said() -> None:
    events: list[EventData] = [message("Hello."), reply("Hi."), message("And?"), reply("Well.")]

    assert replies(events) == ["Hi.", "Well."]


def test_a_reply_that_was_cut_off_is_one_with_the_reply_that_took_it_up() -> None:
    events: list[EventData] = [
        message("Write."),
        reply("This sentence never", cut=True),
        GO_ON,
        reply(" ends, they say."),
    ]

    assert replies(events) == ["This sentence never ends, they say."]


def test_a_reply_that_was_taken_up_more_than_once_is_still_one() -> None:
    events: list[EventData] = [
        reply("one", cut=True),
        GO_ON,
        reply(" two", cut=True),
        GO_ON,
        reply(" three"),
        reply("four"),
    ]

    assert replies(events) == ["one two three", "four"]


def test_a_reply_that_was_cut_off_and_not_taken_up_stands_by_itself() -> None:
    # The turn ended there, or the user said something else instead.
    events: list[EventData] = [
        reply("This sentence never", cut=True),
        TurnEndEvent(turn=1, reason="max-tokens"),
        message("Shorter, please."),
        reply("Sorry."),
    ]

    assert replies(events) == ["This sentence never", "Sorry."]


def test_only_the_reply_that_comes_next_goes_on_from_one_that_was_cut_off() -> None:
    events: list[EventData] = [
        reply("I write it down.", cut=True),
        GO_ON,
        calling(),
        reply("It is written."),
    ]

    assert replies(events) == ["I write it down.", "It is written."]


def test_what_a_model_is_told_beside_does_not_come_between() -> None:
    # A conversation may be made shorter just then, and its context is given once more.
    told = message("These are the rules.", "context/instructions")
    events: list[EventData] = [reply("one", cut=True), GO_ON, told, reply(" two")]

    assert replies(events) == ["one two"]


def test_only_being_told_to_go_on_makes_the_next_reply_one_with_that_which_was_cut_off() -> None:
    # Something else that a model is told is no word to go on. A turn may have ended there.
    told = message("These are the rules.", "context/instructions")
    events: list[EventData] = [reply("one", cut=True), told, reply("two")]

    assert replies(events) == ["one", "two"]


def test_what_somebody_says_after_that_comes_between() -> None:
    events: list[EventData] = [
        reply("one", cut=True),
        GO_ON,
        message("No, stop.", "steer"),
        reply("Stopped."),
    ]

    assert replies(events) == ["one", "Stopped."]


def test_being_told_to_go_on_joins_nothing_to_a_reply_that_ended_by_itself() -> None:
    # Nobody writes such a log. Whoever reads one does not make one answer of two.
    events: list[EventData] = [reply("Done."), GO_ON, reply("More.")]

    assert replies(events) == ["Done.", "More."]


def test_a_reply_that_says_nothing_is_told_as_nothing() -> None:
    events: list[EventData] = [reply("Looking."), calling()]

    assert replies(events) == ["Looking.", ""]
