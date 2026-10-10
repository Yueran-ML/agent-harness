"""Which part of a conversation a summary is to take the place of."""

from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.compaction import SUMMARY_SOURCE
from columbina.compaction.basic.region import Region, choose
from columbina.kernel import Context
from columbina.llm import (
    AssistantMessage,
    Message,
    SystemMessage,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    Usage,
    UserMessage,
)
from columbina.session import (
    CORE_EVENTS,
    AssistantMessageEvent,
    Session,
    SystemMessageEvent,
    ToolResultEvent,
    TurnStartEvent,
    UserMessageEvent,
)

KINDS = {kind.event_type: kind for kind in CORE_EVENTS}
FAR = 10**9


def letters(message: Message) -> int:
    """A message is as large as its text is long, so that the tests can count."""
    return sum(len(block.text) for block in message.content if isinstance(block, TextBlock))


class Talk:
    """A conversation that a test writes by hand. Every message is made of so many letters."""

    def __init__(self, ctx: Context, *, system: bool = True) -> None:
        self.session = Session(ctx, "s-1", KINDS)
        self.session.append(TurnStartEvent(turn=1))
        self._calls = 0
        if system:
            message = SystemMessage(content=(TextBlock(text="s" * 50),))
            self.session.append(SystemMessageEvent(message=message))

    def says(self, size: int, source: str = "user") -> int:
        """Somebody says something in the role of the user. Returns where it stands in the log."""
        message = UserMessage(content=(TextBlock(text="u" * size),))
        return self.session.append(UserMessageEvent(message=message, source=source)).seq

    def replies(self, size: int) -> int:
        message = AssistantMessage(content=(TextBlock(text="a" * size),))
        event = AssistantMessageEvent(message=message, usage=Usage(), finish_reason="stop")
        return self.session.append(event).seq

    def calls(self, size: int, *results: int) -> list[int]:
        """The model calls a tool once for each result, and the results follow."""
        ids = [f"call-{self._calls + number}" for number in range(len(results))]
        self._calls += len(results)
        blocks = tuple(ToolCallBlock(call_id=one, name="read", arguments="{}") for one in ids)
        message = AssistantMessage(content=(TextBlock(text="a" * size), *blocks))
        asked = AssistantMessageEvent(message=message, usage=Usage(), finish_reason="tool-calls")
        seqs = [self.session.append(asked).seq]
        for one, result in zip(ids, results, strict=True):
            answer = ToolMessage(call_id=one, content=(TextBlock(text="r" * result),))
            seqs.append(self.session.append(ToolResultEvent(message=answer)).seq)
        return seqs

    def choose(self, *, retain: int, half: int = FAR, shed: int = 0) -> Region | None:
        return choose(self.session, retain=retain, half=half, shed=shed, size=letters)


async def test_the_oldest_part_goes_and_the_newest_stays(ctx: Context) -> None:
    talk = Talk(ctx)
    first = [talk.says(100), talk.replies(100), talk.says(100)]
    talk.replies(100)
    talk.says(100)
    talk.replies(100)

    # Three hundred letters are to stay: the last three messages come to that.
    assert talk.choose(retain=300) == Region(seqs=tuple(first), tokens=300)
    # One letter more, and a fourth message has to stay for it.
    assert talk.choose(retain=301) == Region(seqs=tuple(first[:2]), tokens=200)
    assert talk.choose(retain=201) == Region(seqs=tuple(first), tokens=300)


async def test_a_conversation_that_is_no_longer_than_what_is_to_stay_has_no_such_part(
    ctx: Context,
) -> None:
    talk = Talk(ctx)
    talk.says(100)
    talk.replies(100)

    assert talk.choose(retain=200) is None
    assert talk.choose(retain=5000) is None
    assert talk.choose(retain=199) is None
    assert Talk(Context()).choose(retain=0) is None


async def test_the_instructions_and_the_context_at_the_top_stay_where_they_are(
    ctx: Context,
) -> None:
    talk = Talk(ctx)
    talk.says(500, "context/instructions")
    talk.says(100, "context/runtime")
    old = [talk.says(100), talk.replies(100), talk.says(70, "context/runtime")]
    talk.says(100)
    talk.replies(100)

    # Context that came later is a part of the conversation like any other.
    assert talk.choose(retain=100) == Region(seqs=tuple(old), tokens=270)


async def test_a_conversation_without_instructions_begins_with_what_goes(ctx: Context) -> None:
    talk = Talk(ctx, system=False)
    old = [talk.says(100), talk.replies(100)]
    talk.says(100)

    assert talk.choose(retain=100) == Region(seqs=tuple(old), tokens=200)


async def test_a_call_and_its_results_are_not_parted(ctx: Context) -> None:
    talk = Talk(ctx)
    old = [talk.says(100)]
    group = talk.calls(50, 100, 100)
    talk.replies(100)

    # What is to stay is reached in the middle of the results. They stay with their call.
    # (With nothing as half of what the model takes, the words of the user do not have to stay.)
    assert talk.choose(retain=150, half=0) == Region(seqs=tuple(old), tokens=100)
    assert talk.choose(retain=250, half=0) == Region(seqs=tuple(old), tokens=100)
    # And they go with it.
    assert talk.choose(retain=100, half=0) == Region(seqs=(*old, *group), tokens=350)


async def test_what_the_user_said_last_stays_in_their_own_words(ctx: Context) -> None:
    talk = Talk(ctx)
    old = [talk.says(100), talk.replies(100)]
    task = talk.says(100)
    talk.calls(50, 1000)
    talk.replies(100)

    # The result and the reply would be enough to stay. The task that they belong to stays too.
    assert talk.choose(retain=150, half=1250) == Region(seqs=tuple(old), tokens=200)
    # Unless what follows it is more than half of what the model takes.
    assert talk.choose(retain=150, half=1249) == Region(seqs=(*old, task), tokens=300)


async def test_a_steer_is_what_the_user_said_too_and_other_sources_are_not(ctx: Context) -> None:
    talk = Talk(ctx)
    old = [talk.says(100), talk.replies(100)]
    steer = talk.says(100, "steer")
    hook = talk.says(100, "hook")
    talk.replies(100)

    assert talk.choose(retain=100) == Region(seqs=tuple(old), tokens=200)
    assert steer not in old
    assert hook > steer


async def test_an_earlier_summary_is_summed_up_with_what_followed_it(ctx: Context) -> None:
    talk = Talk(ctx)
    summary = talk.says(200, SUMMARY_SOURCE)
    old = [summary, talk.says(100), talk.replies(100)]
    talk.says(100)
    talk.replies(100)

    assert talk.choose(retain=200) == Region(seqs=tuple(old), tokens=400)


async def test_a_summary_alone_is_not_summed_up_again(ctx: Context) -> None:
    talk = Talk(ctx)
    talk.says(200, SUMMARY_SOURCE)
    talk.says(100)
    talk.replies(100)

    assert talk.choose(retain=200) is None


async def test_more_goes_if_so_much_has_to_go(ctx: Context) -> None:
    talk = Talk(ctx)
    first = [talk.says(100), talk.replies(100), talk.says(100)]
    second = [talk.replies(100), *talk.calls(50, 100, 100)]
    last = talk.replies(100)

    def chosen(shed: int) -> Region | None:
        return talk.choose(retain=450, half=0, shed=shed)

    assert chosen(0) == Region(seqs=tuple(first), tokens=300)
    # The model refused the conversation as too long by so much. That counts for more than
    # what was to stay.
    assert chosen(300) == Region(seqs=tuple(first), tokens=300)
    assert chosen(301) == Region(seqs=(*first, second[0]), tokens=400)
    # A call goes with its results or not at all.
    assert chosen(401) == Region(seqs=(*first, *second), tokens=650)
    # The last thing stays whatever has to go: it is what the model is asked about.
    assert chosen(FAR) == Region(seqs=(*first, *second), tokens=650)
    assert last not in (*first, *second)


async def test_so_much_has_to_go_even_if_the_words_of_the_user_go_with_it(ctx: Context) -> None:
    talk = Talk(ctx)
    old = [talk.says(100), talk.replies(100)]
    task = talk.says(100)
    talk.replies(100)

    assert talk.choose(retain=100) == Region(seqs=tuple(old), tokens=200)
    assert talk.choose(retain=100, shed=201) == Region(seqs=(*old, task), tokens=300)


async def test_results_that_the_model_has_yet_to_read_stay_with_their_call(ctx: Context) -> None:
    talk = Talk(ctx)
    old = [talk.says(100), talk.replies(100)]
    talk.calls(50, 100, 100)

    assert talk.choose(retain=0, shed=FAR) == Region(seqs=tuple(old), tokens=200)


@settings(max_examples=200, deadline=None)
@given(data=st.data())
def test_whatever_the_conversation_the_part_that_goes_is_its_beginning_and_ends_well(
    data: st.DataObject,
) -> None:
    talk = Talk(Context(), system=data.draw(st.booleans()))
    sizes = st.integers(min_value=0, max_value=300)
    for _ in range(data.draw(st.integers(min_value=0, max_value=12))):
        kind = data.draw(st.sampled_from(["user", "steer", "context/x", SUMMARY_SOURCE, "reply"]))
        if kind == "reply":
            talk.replies(data.draw(sizes))
        else:
            talk.says(data.draw(sizes), kind)
        if data.draw(st.booleans()):
            results = data.draw(st.lists(sizes, min_size=1, max_size=3))
            talk.calls(data.draw(sizes), *results)
    retain = data.draw(st.integers(min_value=0, max_value=1500))
    half = data.draw(st.integers(min_value=0, max_value=3000))
    shed = data.draw(st.sampled_from([0, 0, 200, FAR]))
    session = talk.session

    region = talk.choose(retain=retain, half=half, shed=shed)

    events = [session.events[seq] for seq in session.surface]
    sized = [letters(message) for message in session.messages]
    if region is None:
        return
    # It is a run of neighbours, so one message can take its place.
    first = session.surface.index(region.seqs[0])
    stop = first + len(region.seqs)
    assert session.surface[first:stop] == region.seqs
    assert region.tokens == sum(sized[first:stop])
    # Before it stand the instructions and the context that came with them, and nothing else.
    for event in events[:first]:
        assert isinstance(event.data, SystemMessageEvent) or (
            isinstance(event.data, UserMessageEvent) and event.data.source.startswith("context/")
        )
    # Something stays, and what stays does not begin with a result without its call.
    assert stop < len(events)
    assert not isinstance(events[stop].data, ToolResultEvent)
    # There is something in it that is no summary.
    assert any(
        not (isinstance(event.data, UserMessageEvent) and event.data.source == SUMMARY_SOURCE)
        for event in events[first:stop]
    )
    if shed == 0:
        # Without need, no less stays than was to stay.
        assert sum(sized[stop:]) >= retain
        human = [
            at
            for at, event in enumerate(events)
            if isinstance(event.data, UserMessageEvent) and event.data.source in ("user", "steer")
        ]
        if human and first <= human[-1] < stop:
            assert sum(sized[human[-1] :]) > half
