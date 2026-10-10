"""Skills: instructions for one kind of task, which a model loads when it meets such a task.

This module is a plugin. Loading it provides :data:`SKILLS`, registers the
source of context ``skills`` and the tool ``skill``, and lets a model read
the files of the skills that its session was told of.

A skill is written once, by the people an agent works for, and lies in a
directory of its own. A model is told what skills there are, in a line each,
and loads the text of one when a task calls for it. So a hundred skills cost
a hundred lines, and the text of a skill costs something only when it is used.

The skills are looked for whenever somebody asks: when a turn begins, when a
model loads one, when a person wants the list. So a skill that is written
while a session runs is there for the next turn, and nothing has to be
started again. What a model was told is written anew only when it differs
from what it was told before: a change to the text of a skill changes no
line of the list.

A place that cannot be looked into at one moment is taken as it was found
the time before. One failed look at a disk should not make a model believe
that its skills are gone.

The text of a skill may point to files beside it, and a skill of the user
lies outside the working directory, where the seam of the file system lets
nothing be read. So this plugin gives the seam one grant: a file in the
directory of a skill that the session was told of may be read. It is never
written by that grant. The grant goes by where a file really is, so a link
in such a directory opens nothing that lies elsewhere.

What a skill is on disk, and where skills are looked for, is in
:mod:`columbina.skills.files`.
"""

import asyncio
import logging
import weakref
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from pydantic import BaseModel, Field, field_validator

from columbina.agent import AGENTS, CONTEXT_PREFIX, Agent, ContextSource
from columbina.fs import FS, Access, locate
from columbina.kernel import Context, ServiceKey
from columbina.project import root_of
from columbina.session import EventData, Projection, Session, SessionEvent, UserMessageEvent
from columbina.skills import files
from columbina.skills.files import Found, Root, Scan, Skill, SkillError
from columbina.tools import TOOLS, Tool, ToolFailure, ToolRun

__all__ = [
    "MAX_DESCRIPTION",
    "MIN_DESCRIPTION",
    "SKILLS",
    "TOOL",
    "Config",
    "SkillArgs",
    "Skills",
    "apply",
    "catalog",
    "inject",
    "loaded",
    "name",
    "one_line",
    "skill_tool",
]

name = "skills"
inject = (AGENTS, TOOLS)

logger = logging.getLogger(__name__)

TOOL = "skill"
"""The name of the tool that loads a skill."""

MAX_DESCRIPTION = 500
"""How many characters of what a skill is for a model is told at most."""

MIN_DESCRIPTION = 80
"""How short that is made at most, when there are too many skills to tell of each in full."""

_SOURCE = CONTEXT_PREFIX + "skills"

_INTRO = (
    "A skill is a set of instructions for one kind of task, written by the people you work "
    "for. These skills are there. When a task is of the kind that a skill is for, call the "
    f"tool `{TOOL}` with its name before you do anything else, and do the task the way it says."
)
_GONE = "No skill is there any more. The skills that you were told of before cannot be loaded."


class Config(BaseModel):
    """The config of the skills.

    Attributes:
        home: The directory of skills in the home of Columbina.
        user: The directory of skills that the user shares between tools.
        extra: Further directories of skills. They count last.
        catalog_max_chars: How long the list that a model is given may be.
    """

    home: Path | None = None
    user: Path | None = None
    extra: tuple[Path, ...] = ()
    catalog_max_chars: int = Field(default=12000, ge=1000)

    @field_validator("home", "user", "extra")
    @classmethod
    def _whole(cls, given: Path | tuple[Path, ...] | None) -> Path | tuple[Path, ...] | None:
        # From which directory a path that begins nowhere would be meant, nobody can tell.
        places = () if given is None else given if isinstance(given, tuple) else (given,)
        for place in places:
            if not place.is_absolute():
                raise ValueError(f"{place} does not begin at a root. Give the whole path")
        return given


def _given(state: bool, event: SessionEvent[EventData]) -> bool:
    data = event.data
    return state or (isinstance(data, UserMessageEvent) and data.source == _SOURCE)


_GIVEN: Projection[bool] = Projection("skills-given", False, _given)
"""Whether a session was ever told of skills."""


def one_line(description: str, width: int = MAX_DESCRIPTION) -> str:
    """Return what a skill is for on one line, in no more characters than ``width``."""
    told = " ".join(description.split())
    return told if len(told) <= width else f"{told[: width - 1]}…"


def _line(skill: Skill, width: int) -> str:
    return f"- {skill.name}: {one_line(skill.description, width)}"


def _left_out(count: int) -> str:
    more = "1 more skill is" if count == 1 else f"{count} more skills are"
    return (
        f"[{more} there and not listed, because the list would be too long. The user sees "
        "all of them with /skills.]"
    )


def catalog(skills: Sequence[Skill], limit: int) -> str | None:
    """Return what a model is told about the skills that it may load.

    Each skill has a line. If all lines together are too long, every skill
    is told of in fewer characters, all alike. If the shortest lines are too
    long still, the last skills are left out, and the model is told how many.

    Args:
        skills: The skills, in the order in which they are listed.
        limit: How many characters the text may have. The words before the
            list and the word of what was left out are given whatever it is.

    Returns:
        The text, or ``None`` if there is no skill that a model may load.
    """
    listed = [skill for skill in skills if skill.model_invocable]
    if not listed:
        return None

    def said(lines: Sequence[str]) -> str:
        left = len(listed) - len(lines)
        return "\n".join([_INTRO, *lines, *([_left_out(left)] if left else [])])

    def at(width: int) -> str:
        return said([_line(skill, width) for skill in listed])

    if len(whole := at(MAX_DESCRIPTION)) <= limit:
        return whole
    if len(at(MIN_DESCRIPTION)) <= limit:
        # The widest lines that fit. A wider line never makes the text shorter.
        low, high = MIN_DESCRIPTION, MAX_DESCRIPTION - 1
        while low < high:
            middle = (low + high + 1) // 2
            if len(at(middle)) <= limit:
                low = middle
            else:
                high = middle - 1
        return at(low)
    # The most skills that fit. One skill fewer never makes the text longer.
    short = [_line(skill, MIN_DESCRIPTION) for skill in listed]
    low, high = 0, len(short) - 1
    while low < high:
        middle = (low + high + 1) // 2
        if len(said(short[:middle])) <= limit:
            low = middle
        else:
            high = middle - 1
    return said(short[:low])


def loaded(skill: Skill, text: str) -> str:
    """Return a skill as a model is given it: what it is, where it lies, and its text.

    The directory is told because the text may point to files in it.
    """
    head = (
        f"Skill: {skill.name}\n"
        f"Directory of the skill: {skill.directory}\n"
        "A path in what follows that does not begin at a root is meant from that directory."
    )
    return f"{head}\n\n{text}" if text else head


@dataclass(frozen=True, slots=True)
class _Known:
    """What was found for a session when its skills were last looked for.

    Attributes:
        scans: What each place held.
        directories: The directories of the skills, as the system spells
            them. Their files may be read. They are kept as text: two paths
            that differ by the case of a letter are one path to Windows and
            may be two directories on its disk.
        said: What was written to the log about these places, so that it is
            not written again for as long as it stays true.
    """

    scans: dict[Root, Scan]
    directories: frozenset[str]
    said: frozenset[tuple[str, ...]]


class Skills:
    """The skills that there are for a session, looked for anew whenever somebody asks."""

    __slots__ = ("_config", "_known")

    def __init__(self, config: Config) -> None:
        self._config = config
        self._known: weakref.WeakKeyDictionary[Session, _Known] = weakref.WeakKeyDictionary()

    def _look(self, cwd: Path, last: dict[Root, Scan]) -> tuple[dict[Root, Scan], dict[Root, str]]:
        """Look into every place where skills may lie. This blocks.

        Returns:
            What each place holds, and why a place could not be looked into.
            Such a place is taken as it was found the time before.
        """
        config = self._config
        project = root_of(locate(cwd, ".").path)
        scans: dict[Root, Scan] = {}
        failed: dict[Root, str] = {}
        for root in files.roots(project, config.home, config.user, config.extra):
            try:
                scans[root] = files.scan(root)
            except Exception as error:
                # Whoever asks may be the loop at the beginning of a turn, and a source of
                # context that fails ends the turn. No place is worth that.
                if not isinstance(error, OSError):
                    logger.error("looking for skills in %s failed", root.path, exc_info=error)
                failed[root] = str(error) or type(error).__name__
                scans[root] = last.get(root, Scan(root))
        return scans, failed

    async def found(self, session: Session, cwd: Path) -> Found:
        """Look for the skills of a session anew, and return what there is.

        From then on the files of these skills may be read in that session.

        Args:
            session: The session that asks.
            cwd: Its working directory.
        """
        known = self._known.get(session)
        last = {} if known is None else known.scans
        scans, failed = await asyncio.to_thread(self._look, cwd, last)
        found = files.merged(list(scans.values()))
        said: set[tuple[str, ...]] = {
            ("place", str(root.path), why) for root, why in failed.items()
        }
        said |= {("invalid", str(one.directory), one.reason) for one in found.invalid}
        said |= {("passed over", str(one.directory)) for one in found.shadowed}
        # Each of these is written to the log once, and again only after it had stopped being so.
        for kind, place, *why in sorted(said if known is None else said - known.said):
            if kind == "place":
                logger.warning(
                    "the skills in %s cannot be looked for now, so what was found there "
                    "before is kept: %s",
                    place,
                    why[0],
                )
            elif kind == "invalid":
                logger.warning("%s is no skill that can be used: %s", place, why[0])
            else:
                logger.info("the skill in %s is passed over for one of the same name", place)
        directories = frozenset(str(skill.directory) for skill in found.skills.values())
        self._known[session] = _Known(scans, directories, frozenset(said))
        return found

    async def body(self, skill: Skill) -> tuple[Skill, str]:
        """Read a skill anew, and return it as it is now with its text.

        Raises:
            SkillError: The skill is no longer there, or can no longer be used.
            OSError: Its file cannot be read.
        """
        return await asyncio.to_thread(files.body, skill)

    def grants(self, access: Access) -> bool:
        """Tell whether an access reads a file of a skill that its session was told of."""
        if access.kind != "read":
            return False
        known = self._known.get(access.session)
        if known is None:
            return False
        path = access.where.path
        return not known.directories.isdisjoint(map(str, (path, *path.parents)))


SKILLS = ServiceKey[Skills]("skills")
"""The skills."""


class SkillArgs(BaseModel):
    """The arguments of the tool that loads a skill."""

    name: str = Field(description="The name of the skill, as the list of skills gives it.")


def skill_tool(skills: Skills) -> Tool[SkillArgs, str]:
    """Make the tool that loads a skill."""

    async def load(args: SkillArgs, run: ToolRun) -> str:
        found = await skills.found(run.session, run.cwd)
        skill = found.skills.get(args.name)
        if skill is None:
            there = [name for name, one in found.skills.items() if one.model_invocable]
            listed = f"The skills are: {', '.join(there)}." if there else "There are no skills."
            raise ToolFailure(
                f"There is no skill named {args.name!r}. {listed}", code="UNKNOWN_SKILL"
            )
        try:
            now, text = await skills.body(skill)
        except SkillError as error:
            raise ToolFailure(
                f"The skill {skill.name} cannot be used: {error}.", code="INVALID_SKILL"
            ) from None
        except OSError as error:
            raise ToolFailure(
                f"The skill {skill.name} cannot be read: {error.strerror or error}.",
                code="UNREADABLE_SKILL",
            ) from None
        # What the file says now counts: it was read a moment ago, and the list is older.
        if not now.model_invocable:
            raise ToolFailure(
                f"The skill {now.name} is not loaded by a model itself. The user can hand it "
                f"to you with /skill {now.name}.",
                code="NOT_FOR_A_MODEL",
            )
        return loaded(now, text)

    return Tool(
        name=TOOL,
        description=(
            "Load a skill by its name: the instructions for one kind of task. The skills that "
            "there are stand in the conversation, each with what it is for. Load a skill "
            "before you begin a task that it is for."
        ),
        parameters=SkillArgs,
        execute=load,
        concurrency_safe=True,
    )


def apply(ctx: Context, config: Config) -> None:
    """Provide the skills, tell a model of them, and let it load one and read its files."""
    skills = Skills(config)

    async def render(agent: Agent) -> str | None:
        found = await skills.found(agent.session, agent.cwd)
        said = catalog(list(found.skills.values()), config.catalog_max_chars)
        if said is None and _GIVEN.of(agent.session):
            return _GONE
        return said

    seam = ctx.get(FS)
    if seam is not None:
        seam.grants.add(ctx, "skills", skills.grants)
    ctx.use(TOOLS).add(ctx, skill_tool(skills))
    # After what is told about the run, and before the list of things to do.
    ctx.use(AGENTS).context.add(ctx, "skills", ContextSource(render, order=120, kind="replace"))
    ctx.provide(SKILLS, skills)
