"""Checkpoints: the files of the working directory as they stood before each turn, to go back to.

This module is a plugin. Loading it has the working directory of every
conversation kept before each of its turns, and provides :data:`CHECKPOINTS`,
which goes back to one of them. The command ``/rewind`` is a plugin of its
own, see :mod:`columbina.commands.rewind`.

**Where.** What is kept goes into a repository of git of its own, in the home
of the program, one for each working directory: ``checkpoints/<key>``. The
repository of the project, if there is one, is not touched: none of its
branches, its index or its stash. What the project ignores with its
``.gitignore`` is not kept, and neither is what is named in
:data:`EXCLUDED`, nor what lies in a repository inside the working directory.
Without git on the search path nothing is kept, and the log says so once.
git is not taken from the working directory, nor from what lies in it.

**When.** Before the first step of every turn of a conversation, and before
the files are taken back to an earlier turn, so that going back can be undone.
A subagent works within a turn of the conversation that delegated to it, and
what it changes is in the next checkpoint of that conversation. Each
checkpoint is written into the log, as :class:`CheckpointEvent`. Keeping one
takes so many seconds at most: a directory that holds more than can be kept
in that time is passed over for the rest of the run, and the log says so.

**Going back.** The files are put back as they stood: what was changed since
is changed back, what was made since is taken away, and what was taken away
is made again. What git does not keep is not touched, and neither is what lies
outside the working directory. A command that changed something elsewhere, a
database or a service, is not undone by this.
"""

import logging
import os
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import ClassVar

from pydantic import BaseModel, Field

from columbina import process, programs
from columbina.agent import PRE_STEP, StepDecision, StepInfo
from columbina.kernel import Context, Next, ServiceKey
from columbina.session import SESSIONS, EventData, Session
from columbina.session.store import workspace_key

__all__ = [
    "CHECKPOINTS",
    "EXCLUDED",
    "CheckpointError",
    "CheckpointEvent",
    "Checkpoints",
    "Config",
    "apply",
    "checkpoints_of",
    "inject",
    "name",
]

name = "checkpoints"
inject = (SESSIONS,)

logger = logging.getLogger("columbina.checkpoints")

EXCLUDED = (
    ".git/",
    ".columbina/",
    "node_modules/",
    ".venv/",
    "venv/",
    "__pycache__/",
    ".pytest_cache/",
    ".mypy_cache/",
    ".ruff_cache/",
    ".DS_Store",
)
"""What is never kept, beside what the project ignores."""

_SETTINGS = (
    "-c",
    "core.autocrlf=false",
    "-c",
    "core.safecrlf=false",
    "-c",
    "core.longpaths=true",
    "-c",
    "core.quotepath=false",
    "-c",
    "gc.auto=0",
)
"""What git is told every time: files are kept as they are, whatever the user set for git."""

_WHO = {
    "GIT_AUTHOR_NAME": "Columbina",
    "GIT_AUTHOR_EMAIL": "checkpoints@columbina.invalid",
    "GIT_COMMITTER_NAME": "Columbina",
    "GIT_COMMITTER_EMAIL": "checkpoints@columbina.invalid",
}


class CheckpointEvent(EventData):
    """The files of the working directory were kept.

    Attributes:
        turn: The turn before whose first step they were kept, or the turn
            that was the last one when they were kept before going back.
        commit: Where git keeps them.
        before_going_back: Whether they were kept because the files were
            about to be taken back to an earlier turn.
    """

    event_type: ClassVar[str] = "checkpoint"
    ignorable: ClassVar[bool] = True

    turn: int
    commit: str
    before_going_back: bool = False


class CheckpointError(Exception):
    """The files could not be kept or put back, and why."""


class Config(BaseModel):
    """The config of the plugin.

    Attributes:
        home: Where the repositories of the checkpoints are kept.
        seconds: How long keeping the files may take.
    """

    home: Path
    seconds: float = Field(default=30.0, gt=0)


@dataclass(frozen=True, slots=True)
class _Place:
    """The repository of the checkpoints of one working directory."""

    git: str
    repository: Path
    work: Path

    def argv(self, *arguments: str, made: bool = True) -> list[str]:
        """Return how git is run on the repository, or, before it is made, to make it."""
        if not made:
            return [self.git, *_SETTINGS, *arguments]
        return [
            self.git,
            *_SETTINGS,
            f"--git-dir={self.repository}",
            f"--work-tree={self.work}",
            *arguments,
        ]


def _environment() -> dict[str, str]:
    env = process.cleaned(os.environ)
    for variable in list(env):
        # Whatever says where git looks would send it to the repository of the project.
        if variable.upper().startswith("GIT_"):
            del env[variable]
    return {**env, **_WHO}


class Checkpoints:
    """Keeps the files of working directories, and puts them back."""

    __slots__ = ("_config", "_find", "_passed_over")

    def __init__(self, config: Config, find: Callable[[Sequence[Path]], str | None]) -> None:
        self._config = config
        self._find = find
        self._passed_over: set[Path] = set()

    def _place(self, cwd: Path) -> _Place:
        work = cwd.resolve()
        # A program that lies in the working directory could be put there by what runs there.
        git = self._find([work])
        if git is None:
            raise CheckpointError("git is not on the search path, so no files are kept")
        return _Place(git, self._config.home / workspace_key(work), work)

    async def _run(self, place: _Place, *arguments: str, made: bool = True) -> str:
        """Run git on the repository of the checkpoints, and return what it wrote."""
        seconds = self._config.seconds
        try:
            done = await process.run(
                place.argv(*arguments, made=made),
                cwd=place.work,
                env=_environment(),
                timeout_seconds=seconds,
                streams="apart",
            )
        except OSError as error:
            raise CheckpointError(f"git could not be started: {error}") from None
        if done.timed_out:
            raise CheckpointError(f"git took more than {seconds:g} seconds")
        if done.exit_code != 0:
            said = (done.error_output or done.output).strip().splitlines()
            raise CheckpointError(f"git failed: {said[-1] if said else done.exit_code}")
        return done.output.strip()

    async def _ready(self, place: _Place) -> None:
        """Make the repository of a working directory, if it is not there."""
        if (place.repository / "HEAD").is_file():
            return
        place.repository.mkdir(parents=True, exist_ok=True)
        await self._run(place, "init", "--bare", "--quiet", str(place.repository), made=False)
        info = place.repository / "info"
        info.mkdir(exist_ok=True)
        (info / "exclude").write_text("\n".join(EXCLUDED) + "\n", encoding="utf-8")

    async def keep(self, cwd: Path) -> str:
        """Keep the files of a working directory, and return where they are kept.

        Raises:
            CheckpointError: They could not be kept.
        """
        place = self._place(cwd)
        await self._ready(place)
        await self._run(place, "add", "--all", "--", ".")
        await self._run(
            place, "commit", "--quiet", "--allow-empty", "--no-verify", "-m", "checkpoint"
        )
        return await self._run(place, "rev-parse", "HEAD")

    async def go_back(self, cwd: Path, commit: str) -> None:
        """Put the files of a working directory back as they were kept.

        The files have to have been kept just before, so that what was made
        since is known and is taken away.

        Raises:
            CheckpointError: They could not be put back.
        """
        place = self._place(cwd)
        await self._run(place, "read-tree", "-u", "--reset", commit)

    async def changed(self, cwd: Path, commit: str) -> list[str]:
        """Return the files that differ between what was kept and what is there now.

        Raises:
            CheckpointError: git could not tell.
        """
        place = self._place(cwd)
        said = await self._run(place, "diff", "--name-only", commit, "HEAD")
        return [line for line in said.splitlines() if line]

    async def before_a_step(
        self, info: StepInfo, proceed: Next[StepInfo, StepDecision]
    ) -> StepDecision:
        """Keep the files before the first step of a turn of a conversation."""
        session = info.agent.session
        if info.step == 1 and session.delegation_depth == 0:
            await self._kept_for(session, info.agent.spec.cwd, info.turn)
        return await proceed(info)

    async def _kept_for(self, session: Session, cwd: Path, turn: int) -> None:
        if cwd in self._passed_over:
            return
        try:
            commit = await self.keep(cwd)
        except CheckpointError as error:
            self._passed_over.add(cwd)
            logger.warning(
                "the files of %s are not kept before each turn in this run: %s", cwd, error
            )
            return
        session.append(CheckpointEvent(turn=turn, commit=commit))


def checkpoints_of(session: Session) -> Sequence[CheckpointEvent]:
    """Return the checkpoints of a session, in the order in which they were kept."""
    return [event.data for event in session.events if isinstance(event.data, CheckpointEvent)]


CHECKPOINTS = ServiceKey[Checkpoints]("checkpoints")
"""What keeps the files of working directories, and puts them back."""


def _git(outside: Sequence[Path] = ()) -> str | None:
    """Return the program git, from no directory that ``outside`` names."""
    return programs.found("git", os.environ.get("PATH", ""), outside=outside)


def apply(ctx: Context, config: Config) -> None:
    """Keep the files before every turn, where there is git, and say once where there is none."""
    ctx.use(SESSIONS).declare(ctx, CheckpointEvent)
    checkpoints = Checkpoints(config, _git)
    ctx.provide(CHECKPOINTS, checkpoints)
    ctx.on(PRE_STEP, checkpoints.before_a_step)
