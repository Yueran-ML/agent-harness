"""The service that makes conversations shorter.

This module is a plugin. Loading it provides
:data:`columbina.compaction.COMPACTION`, and unless the config says
otherwise, it makes a conversation shorter by itself at two moments.

**Before a step**, if the conversation has come near what its model takes.
The step has not begun, so every call has its result and nothing is being
written. How near is too near follows from the window of the model ``W``,
the room that requests leave for the reply ``O``, and a headroom ``B`` that
is the smaller of a number of tokens and a part of the window::

    floor(min(W * threshold_ratio, W - O - B))

A model whose window is not configured is not watched in this way.

**After the model has refused a request as too long.** The estimate was
wrong, or nothing watched. What the service said is read for numbers: how
much the model takes and how much the request came to. So much, and the
headroom, has to go. The request is then made again, once in a step, and
only if the model now sees less.

Making it shorter is done in this order.

1. The long results of tools that the model has read are looked for. If
   cutting them down to their ends is enough, that is all that is done. It
   costs no request. Where the program has a place for what is too large
   for a conversation, the whole of a result is saved there before it is
   cut, and what takes its place says where: the model can read again what
   it turns out to need.
2. Otherwise the oldest part is summed up by the model, as it stands: with
   the results whole, the summary is better, and the request is one whose
   beginning the service has seen before. See :mod:`.region` for what goes
   and :mod:`.summary` for how it is asked for. A part that comes to no more
   than the frame that a summary stands in is not summed up: no summary of
   it could be shorter, and asking for one costs a request.
3. If that is still not enough, the results that are left are cut down. So
   are they if no summary came of it.

A failure never ends the turn that it happened in. The end of the attempt is
written to the log with the reason, and the conversation goes on as it was,
but for what was cut down. After two failures in a row a conversation is not
made shorter by itself any more until an attempt succeeds, which the user
can ask for. A request that is refused is still followed by an attempt.

The log tells what happened in this order: a start, then the results that
were cut and the summary with the message that carries it, then an end. The
message that carries the summary is the one event that takes the oldest part
out of what the model sees. If the program ends before it, nothing has
changed, and the start is left without an end.
"""

import asyncio
import logging
import math
import re
from collections.abc import Callable, Collection
from dataclasses import dataclass
from typing import Self
from weakref import WeakKeyDictionary, WeakSet

from pydantic import BaseModel, Field, model_validator

from columbina.agent import (
    PRE_STEP,
    REQUEST,
    REQUEST_ERROR,
    Agent,
    Enter,
    ErrorDecision,
    RequestDraft,
    RequestFailure,
    Retry,
    StepDecision,
    StepInfo,
)
from columbina.compaction import (
    COMPACTION,
    EVENTS,
    SUMMARY_SOURCE,
    CompactionEndEvent,
    CompactionStartEvent,
    CompactionSummaryEvent,
    Outcome,
    Why,
)
from columbina.compaction.basic.region import Region, choose
from columbina.compaction.basic.summary import SummaryError, carried, summarise
from columbina.compaction.basic.trim import Cut, cuts, located, whole
from columbina.compaction.basic.trim import apply as cut_down
from columbina.fs import FS
from columbina.kernel import Context, Next
from columbina.llm import LLM, CallConfig, Llm, Message, ToolSchema
from columbina.session import (
    SESSIONS,
    TURN,
    EventData,
    Projection,
    RequestHeaderEvent,
    Session,
    SessionEvent,
    ToolResultEvent,
    UserMessageEvent,
)
from columbina.spill import SPILL, Saved
from columbina.token_meter import TOKEN_METER, Measure, TokenMeter, estimate_message

__all__ = ["Basic", "Budget", "Config", "Refusal", "apply", "budget", "inject", "name", "refusal"]

name = "compaction"
inject = (SESSIONS, LLM, TOKEN_METER)

logger = logging.getLogger(__name__)

_TOO_LONG = "CONTEXT_WINDOW_EXCEEDED"

_Coming = tuple[int, int, int]
"""The request that is about to be made: its turn, its step, and which attempt of the step."""
_FAILURES_IN_A_ROW = 2
"""After so many failures in a row, a conversation is not made shorter by itself."""

_LINE_BETWEEN = 128
"""How many characters the two ends of a result leave for the line that stands between them."""


class Config(BaseModel):
    """The config of the service that makes conversations shorter.

    Attributes:
        auto: Whether a conversation is made shorter by itself, before a
            step and after a refusal. Without this, only when it is asked
            for.
        threshold_ratio: The part of the window that a conversation may fill.
        headroom_tokens: How many tokens are kept free for a reply, at most.
            A summary may be as long.
        headroom_ratio: The part of the window that is kept free for a
            reply, at most.
        retain_ratio: The part of the window, less the room for the reply,
            that stays as it is when the rest is summed up.
        trim_over: How many characters a block of text in a result may have
            before it is cut down.
        trim_head: How many characters of its beginning are kept.
        trim_tail: How many characters of its end are kept.
    """

    auto: bool = True
    threshold_ratio: float = Field(default=0.8, gt=0, le=1)
    headroom_tokens: int = Field(default=65_536, gt=0)
    headroom_ratio: float = Field(default=0.25, gt=0, lt=1)
    retain_ratio: float = Field(default=0.16, gt=0, lt=1)
    trim_over: int = Field(default=8192, gt=0)
    trim_head: int = Field(default=4096, ge=0)
    trim_tail: int = Field(default=1024, ge=0)

    @model_validator(mode="after")
    def _ends_are_less_than_the_whole(self) -> Self:
        if self.trim_head + self.trim_tail + _LINE_BETWEEN > self.trim_over:
            raise ValueError(
                f"trim_head and trim_tail come to {self.trim_head + self.trim_tail} characters, "
                f"which is not {_LINE_BETWEEN} less than trim_over ({self.trim_over}): with the "
                "line that says what was left out, a result that was cut down would be cut "
                "down again"
            )
        return self


@dataclass(frozen=True, slots=True)
class Budget:
    """What follows from how much a model takes.

    Attributes:
        window: How many tokens the model takes.
        headroom: How many tokens are kept free. A summary may be as long.
        threshold: How many tokens a conversation may come to before it is
            made shorter.
        retain: How many tokens of the newest messages stay as they are.
    """

    window: int
    headroom: int
    threshold: int
    retain: int


def budget(model: CallConfig, config: Config, window: int | None = None) -> Budget | None:
    """Work out what follows from how much a model takes.

    Args:
        model: How the model is called.
        config: The config of this service.
        window: How many tokens the model takes, if that is known better
            than the call config says.

    Returns:
        The numbers, or ``None`` if nothing says how much the model takes.
    """
    window = window or model.context_window
    if window is None:
        return None
    reply = model.max_output_tokens or 0
    headroom = max(math.floor(min(config.headroom_tokens, window * config.headroom_ratio)), 1)
    return Budget(
        window=window,
        headroom=headroom,
        threshold=math.floor(min(window * config.threshold_ratio, window - reply - headroom)),
        retain=math.floor((window - reply) * config.retain_ratio),
    )


@dataclass(frozen=True, slots=True)
class Refusal:
    """What a service said when it refused a conversation as too long.

    Attributes:
        window: How many tokens the model takes.
        requested: How many the request came to, with the room that it
            asked for the reply.
        messages: How many of them the conversation came to.
    """

    window: int
    requested: int
    messages: int


_WINDOW = re.compile(r"maximum context length is (\d+) tokens")
_REQUESTED = re.compile(r"(?:requested|resulted in) (\d+) tokens")
_MESSAGES = re.compile(r"\((\d+) in the messages")


def refusal(message: str) -> Refusal | None:
    """Read the numbers out of what a service said when it refused a request as too long.

    Returns:
        The numbers, or ``None`` if the service did not say them this way.
    """
    window, requested = _WINDOW.search(message), _REQUESTED.search(message)
    if window is None or requested is None:
        return None
    messages = _MESSAGES.search(message) or requested
    return Refusal(int(window[1]), int(requested[1]), int(messages[1]))


def _last_header(
    state: RequestHeaderEvent | None, event: SessionEvent[EventData]
) -> RequestHeaderEvent | None:
    return event.data if isinstance(event.data, RequestHeaderEvent) else state


_HEADER: Projection[RequestHeaderEvent | None] = Projection(
    "compaction-request-header", None, _last_header
)


async def _as_drafted(draft: RequestDraft) -> CallConfig:
    return draft.config


def _describe(error: BaseException) -> str:
    text = str(error)
    return f"{type(error).__name__}: {text}" if text else type(error).__name__


def _answered(session: Session, seqs: Collection[int]) -> set[str]:
    """Return the ids of the calls whose results are among some events of a session.

    A call and its results are never parted, so the results tell which
    calls a part of a conversation holds.
    """
    results = (session.events[seq].data for seq in seqs)
    return {data.message.call_id for data in results if isinstance(data, ToolResultEvent)}


@dataclass(frozen=True, slots=True)
class _Plan:
    """What an attempt to make a conversation shorter goes by.

    Attributes:
        limits: What follows from how much the model takes.
        scale: How many tokens of the service an estimated token comes to.
        size: What the conversation comes to, counted that way.
        shed: How many tokens have to go, if the service has said so.
        target: What the conversation may come to afterwards. ``None`` if a
            summary is wanted whatever it comes to.
    """

    limits: Budget
    scale: float
    size: int
    shed: int
    target: int | None


def _plan(
    model: CallConfig, config: Config, why: Why, said: Refusal | None, measured: Measure
) -> _Plan:
    # What the service counted is believed before what was estimated.
    scale = measured.rate or 1.0
    if said is not None and measured.estimated:
        scale = said.messages / measured.estimated
    size = math.ceil(measured.estimated * scale)
    limits = budget(model, config, None if said is None else said.window)
    if limits is None:
        # Nothing says how much the model takes. The conversation is its own measure.
        limits = budget(model, config, max(size, 1))
        assert limits is not None
    if why == "pressure":
        return _Plan(limits, scale, size, shed=0, target=limits.threshold)
    if said is None:
        # The user asked for it, or the service gave no numbers. A summary is what is wanted.
        return _Plan(limits, scale, size, shed=0, target=None)
    shed = said.requested - said.window + limits.headroom
    return _Plan(limits, scale, size, shed=shed, target=size - shed)


class Basic:
    """Makes conversations shorter by cutting down old results and summing up the oldest part.

    Args:
        config: The config of the service.
        llm: The service that asks models.
        meter: The estimate of the size of sessions.
    """

    __slots__ = ("_config", "_failures", "_llm", "_meter", "_refused", "_running", "_told")

    def __init__(self, config: Config, llm: Llm, meter: TokenMeter) -> None:
        self._config = config
        self._llm = llm
        self._meter = meter
        self._running: WeakSet[Session] = WeakSet()
        self._failures: WeakKeyDictionary[Session, int] = WeakKeyDictionary()
        self._refused: WeakKeyDictionary[Session, tuple[int, int]] = WeakKeyDictionary()
        self._told = False

    async def compact(self, agent: Agent, why: Why) -> Outcome:
        """Make the conversation of an agent shorter, and say how that ended."""
        return await self._compact(agent, why, None, None)

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Make the conversation shorter if it has come near what its model takes."""
        decision = await proceed(info)
        # Nothing is done for a step that something else keeps from beginning.
        coming = (info.turn, info.step, 1)
        if isinstance(decision, Enter) and await self._pressed(info.agent, coming):
            await self._compact(info.agent, "pressure", None, coming)
        return decision

    async def after_a_refusal(
        self, failed: RequestFailure, proceed: Next[RequestFailure, ErrorDecision]
    ) -> ErrorDecision:
        """Make the conversation shorter if the model refused it as too long, and ask again."""
        session = failed.agent.session
        step = (failed.turn, failed.step)
        if failed.failure.code != _TOO_LONG or self._refused.get(session) == step:
            return await proceed(failed)
        self._refused[session] = step
        said = refusal(failed.failure.message)
        # The request that is made again is the next attempt of the same step.
        coming = (failed.turn, failed.step, failed.attempt + 1)
        outcome = await self._compact(failed.agent, "overflow", said, coming)
        # The same request would be refused again. Only less is worth another try.
        return Retry() if outcome.changed else await proceed(failed)

    async def _model(self, agent: Agent, coming: _Coming | None) -> CallConfig:
        """Return how the next request of an agent would call its model.

        The listeners of ``agent/request`` are asked, as the loop asks them
        before every request. One of them may send the requests to another
        model than the agent was created with. That model is then the one
        whose window counts, and the one that writes the summary.

        Args:
            agent: The agent.
            coming: The turn, the step and the attempt of the request that is
                about to be made, as far as they are known. A listener may
                go by them. Between two turns nothing is known, and the next
                request is taken to be the first of the turn that follows.
        """
        turn, step, attempt = coming or (TURN.of(agent.session).turn + 1, 1, 1)
        draft = RequestDraft(agent, turn, step, attempt, agent.spec.model)
        return await agent.ctx.waterfall(REQUEST, draft, _as_drafted)

    async def _pressed(self, agent: Agent, coming: _Coming) -> bool:
        """Tell whether a conversation is to be made shorter before its next step."""
        session = agent.session
        model = await self._model(agent, coming)
        limits = budget(model, self._config)
        if limits is None or limits.threshold <= 0:
            if not self._told:
                self._told = True
                logger.warning(
                    "the model %s has no context window that leaves room for a conversation "
                    "in its config, so a conversation is not made shorter before it is refused "
                    "as too long",
                    model.model,
                )
            return False
        if self._meter.measure(session).tokens <= limits.threshold:
            return False
        return self._failures.get(session, 0) < _FAILURES_IN_A_ROW

    async def _compact(
        self, agent: Agent, why: Why, said: Refusal | None, coming: _Coming | None
    ) -> Outcome:
        session = agent.session
        measured = self._meter.measure(session)
        if session in self._running:
            return Outcome(
                before=measured.tokens,
                after=measured.tokens,
                error="this conversation is being made shorter already",
            )
        # From here on nobody else begins: asking for the model may take a moment.
        self._running.add(session)
        config = self._config
        try:
            model = await self._model(agent, coming)
            plan = _plan(model, config, why, said, measured)
            session.append(CompactionStartEvent(why=why, tokens=measured.tokens))
        except BaseException:
            # It has not begun. Whoever asks next finds nothing in the way.
            self._running.discard(session)
            raise

        def size(message: Message) -> int:
            return math.ceil(estimate_message(message) * plan.scale)

        trimmed = summarised = 0
        error: str | None = None
        try:
            found = cuts(
                session, over=config.trim_over, head=config.trim_head, tail=config.trim_tail
            )
            # A result is cut down to save tokens. One that would be shorter in characters
            # and cost no less stays as it is.
            found = [cut for cut in found if size(cut.shorter.message) < size(cut.whole.message)]
            saved = sum(size(cut.whole.message) - size(cut.shorter.message) for cut in found)
            enough = plan.target is not None and plan.size - saved <= plan.target
            if not enough:
                limits = plan.limits
                region = choose(
                    session,
                    retain=limits.retain,
                    half=limits.window // 2,
                    shed=plan.shed,
                    size=size,
                )
                # No summary could be shorter than the frame that it stands in. A part that
                # comes to no more than that is left alone, and the model is not asked.
                if region is not None and region.tokens > size(carried("")):
                    try:
                        summarised = await self._sum_up(
                            agent.session, model, region, limits.headroom, size
                        )
                    except SummaryError as failed:
                        error = str(failed)
                    else:
                        self._forget(agent, _answered(session, region.seqs))
                        now = math.ceil(self._meter.measure(session).estimated * plan.scale)
                        enough = plan.target is None or now <= plan.target
            if not summarised or not enough:
                # Cutting alone is enough, or there is no summary, or it was not enough.
                for cut in found:
                    # One at a time: what was cut stays counted if the next cannot be written.
                    trimmed += cut_down(session, [await self._located(agent, cut)])
                    self._forget(agent, {cut.whole.message.call_id})
            if not (trimmed or summarised or error) and why != "manual":
                error = "nothing of this conversation can go"
        except asyncio.CancelledError:
            self._end(session, why, measured.tokens, trimmed, summarised, "cancelled", counts=False)
            raise
        except Exception as failed:
            logger.exception("making the conversation of the session %r shorter failed", session.id)
            error = _describe(failed)
        counts = why == "pressure"
        return self._end(session, why, measured.tokens, trimmed, summarised, error, counts=counts)

    def _forget(self, agent: Agent, call_ids: Collection[str]) -> None:
        """Tell the seam of the files that the model no longer sees what some calls showed it.

        A file that one of them read counts as not read from then on, so the
        model reads it again before it changes it: what it would write
        otherwise is what it remembers of a text that is no longer before it.
        """
        fs = agent.ctx.get(FS)
        if fs is not None and call_ids:
            fs.unshown(agent.session, call_ids)

    async def _located(self, agent: Agent, cut: Cut) -> Cut:
        """Save the whole of a result that is about to be cut, and have the cut say where it is.

        The cut comes back as it was where nothing can be saved, where the
        saving fails, and where the result is no whole: one that was too
        large when it was made stands in the conversation as its two ends,
        with a last line about where its whole is. That line is kept by the
        cut, and the two ends are not saved as if they were everything.

        The line costs a few tokens that were not counted when it was worked
        out whether cutting is enough. The room that is kept free is many
        times that.
        """
        store = agent.ctx.get(SPILL)
        text = whole(cut)
        if store is None or Saved.spoken_of_in(text):
            return cut
        session = agent.session
        try:
            saved = await asyncio.to_thread(store.save_text, session, agent.cwd, "result.txt", text)
        except OSError as error:
            logger.error(
                "a result that is cut out of the conversation of the session %r could not be "
                "saved: %s",
                session.id,
                error,
            )
            return cut
        config = self._config
        return located(
            cut, saved.told(), over=config.trim_over, head=config.trim_head, tail=config.trim_tail
        )

    async def _sum_up(
        self,
        session: Session,
        model: CallConfig,
        region: Region,
        headroom: int,
        size: Callable[[Message], int],
    ) -> int:
        """Have the oldest part summed up and put the summary in its place.

        Returns:
            How many messages the summary took the place of.

        Raises:
            SummaryError: No summary came of it. Nothing has changed.
        """
        header = _HEADER.of(session)
        tools: tuple[ToolSchema, ...] = () if header is None else header.tools
        # A summary may be as long as the room that is kept for a reply, and no longer than
        # what the config lets any reply be.
        limit = min(headroom, model.max_output_tokens or headroom)
        summary = await summarise(
            self._llm,
            session,
            region,
            config=model.model_copy(update={"max_output_tokens": limit}),
            tools=tools,
            size=size,
        )
        message = carried(summary.text, summary.dropped)
        session.append(
            CompactionSummaryEvent(
                summary=summary.text,
                replaces=region.seqs,
                region_tokens=region.tokens,
                summary_tokens=size(message),
                usage=summary.usage,
            )
        )
        session.append(
            UserMessageEvent(message=message, source=SUMMARY_SOURCE), replaces=region.seqs
        )
        return len(region.seqs)

    def _end(
        self,
        session: Session,
        why: Why,
        before: int,
        trimmed: int,
        summarised: int,
        error: str | None,
        *,
        counts: bool,
    ) -> Outcome:
        """Write down how an attempt ended, and keep count of the ones that failed."""
        self._running.discard(session)
        outcome = Outcome(
            trimmed=trimmed,
            summarised=summarised,
            before=before,
            after=self._meter.measure(session).tokens,
            error=error,
        )
        session.append(CompactionEndEvent(why=why, outcome=outcome))
        if error is None:
            self._failures.pop(session, None)
        elif counts:
            self._failures[session] = self._failures.get(session, 0) + 1
        return outcome


def apply(ctx: Context, config: Config) -> None:
    """Provide the service, and make conversations shorter by themselves unless told not to."""
    service = Basic(config, ctx.use(LLM), ctx.use(TOKEN_METER))
    ctx.use(SESSIONS).declare(ctx, *EVENTS)
    if config.auto:
        ctx.on(PRE_STEP, service.before_a_step, prepend=True)
        ctx.on(REQUEST_ERROR, service.after_a_refusal)
    ctx.provide(COMPACTION, service)
