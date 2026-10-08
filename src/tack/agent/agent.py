"""What every agent offers, and the service that creates agents."""

from abc import ABC, abstractmethod
from collections.abc import Callable
from pathlib import Path
from typing import Literal

from tack.agent.inbox import InboxItem, enqueue
from tack.data import Data
from tack.kernel import Context, Registry, ServiceKey
from tack.llm.types import CallConfig, TextBlock, UserMessage
from tack.session import Session, Sessions
from tack.session.store import check_session_id, new_session_id

CancelCause = Literal["user", "parent", "hook", "budget", "disposed"]
"""Who cancelled what an agent was doing."""


class AgentError(Exception):
    """An agent could not be created."""


class AgentSpec(Data):
    """What an agent is created from.

    Attributes:
        cwd: The working directory of the agent.
        model: The model that the agent talks to, and how.
        system_prompt: The instructions that open the conversation.
        loop: The name of the loop that drives the agent.
    """

    cwd: Path
    model: CallConfig
    system_prompt: str = ""
    loop: str = "default"


class Agent(ABC):
    """A conversation that a model carries on, one turn at a time.

    An agent is given input in three ways. A task opens a turn of its own. A
    steer reaches a turn that is running at its next step, and opens a turn if
    none is running. Injected context only waits for other input. All of it
    goes into the session log first, so nothing is lost with the process.

    How turns are run is up to the loop that implements this class. Code that
    uses an agent depends on this interface only, which is what makes the loop
    replaceable.

    Attributes:
        ctx: The context of the scope of the agent. Disposing its fiber ends
            the agent.
        spec: What the agent was created from.
        session: The session log of the agent.
    """

    def __init__(self, ctx: Context, spec: AgentSpec, session: Session) -> None:
        self.ctx = ctx
        self.spec = spec
        self.session = session

    @property
    def id(self) -> str:
        """The id of the agent, which is the id of its session."""
        return self.session.id

    @property
    def cwd(self) -> Path:
        """The working directory of the agent."""
        return self.spec.cwd

    def followup(self, content: str | UserMessage, *, source: str = "user") -> None:
        """Give the agent a task. It opens a turn of its own, after the ones that wait."""
        enqueue(self.session, "next-turn", _item(content, source))
        self.wake()

    def steer(self, content: str | UserMessage, *, source: str = "steer") -> None:
        """Redirect the agent.

        A turn that is running reads this at its next step. If none is
        running, a turn opens.
        """
        enqueue(self.session, "next-step", _item(content, source))
        self.wake()

    def inject(self, content: str | UserMessage, *, source: str = "inject") -> None:
        """Attach context. It is read with the next input and does not start anything itself."""
        enqueue(self.session, "next-step", _item(content, source))

    async def close(self) -> None:
        """End the agent: stop what it is doing and close its session."""
        await self.ctx.fiber.dispose()

    @abstractmethod
    def wake(self) -> None:
        """Tell the agent that input may be waiting for it."""

    @abstractmethod
    def cancel(self, cause: CancelCause = "user", *, keep_inbox: bool = False) -> None:
        """Stop the turn that is running, if any. This only asks: wait with :meth:`when_idle`.

        Args:
            cause: Who is cancelling. It is recorded with the end of the turn.
            keep_inbox: Whether input that waits is kept. By default it is
                dropped, so that the agent comes to rest.
        """

    @abstractmethod
    async def when_idle(self) -> None:
        """Wait until the agent is at rest: no turn is running and none is about to begin."""


def _item(content: str | UserMessage, source: str) -> InboxItem:
    if isinstance(content, str):
        content = UserMessage(content=(TextBlock(text=content),))
    return InboxItem(message=content, source=source)


AgentFactory = Callable[[Context, AgentSpec, Session], Agent]
"""What a loop registers: it makes an agent in a scope, from a spec, on a session."""


class Agents:
    """The agent service: creates agents and knows the ones that are running.

    Attributes:
        loops: The loops that can drive an agent, by name.
        live: The agents that are running, by id.
    """

    __slots__ = ("_sessions", "live", "loops")

    def __init__(self, sessions: Sessions) -> None:
        self.loops: Registry[AgentFactory] = Registry("agent loop")
        self.live: Registry[Agent] = Registry("agent")
        self._sessions = sessions

    async def create(
        self, ctx: Context, spec: AgentSpec, *, session_id: str | None = None
    ) -> Agent:
        """Create an agent with a new session. It belongs to the fiber of ``ctx``.

        Raises:
            AgentError: The loop that the spec names is not there, or an agent
                with the id is running.
            ValueError: The id cannot name a session.
        """
        factory = self._loop(spec)
        if session_id is None:
            session_id = new_session_id(self._sessions.clock())
        scope = self._scope(ctx, check_session_id(session_id))
        try:
            session = self._sessions.create(scope, spec.cwd, session_id=session_id)
            return self._start(scope, factory, spec, session)
        except BaseException:
            await scope.fiber.dispose()
            raise

    async def resume(self, ctx: Context, spec: AgentSpec, session_id: str) -> Agent:
        """Create an agent that takes up a stored session. It belongs to the fiber of ``ctx``.

        Raises:
            AgentError: The loop that the spec names is not there, or an agent
                with the id is running.
            SessionError: The session cannot be resumed.
            ValueError: The id cannot name a session.
        """
        factory = self._loop(spec)
        scope = self._scope(ctx, check_session_id(session_id))
        try:
            session = self._sessions.resume(scope, spec.cwd, session_id)
            return self._start(scope, factory, spec, session)
        except BaseException:
            await scope.fiber.dispose()
            raise

    def _loop(self, spec: AgentSpec) -> AgentFactory:
        factory = self.loops.get(spec.loop)
        if factory is None:
            raise AgentError(f"there is no agent loop named {spec.loop!r}")
        return factory

    def _scope(self, ctx: Context, agent_id: str) -> Context:
        if agent_id in self.live:
            raise AgentError(f"an agent with the id {agent_id!r} is running")
        return ctx.scope(agent_id)

    def _start(
        self, scope: Context, factory: AgentFactory, spec: AgentSpec, session: Session
    ) -> Agent:
        agent = factory(scope, spec, session)
        self.live.add(scope, agent.id, agent)
        return agent


AGENTS = ServiceKey[Agents]("agents")
"""The agent service."""
