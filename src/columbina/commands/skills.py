"""The commands that list the skills and hand one to the agent.

This module is a plugin. Loading it registers the commands ``/skills`` and
``/skill``.

A model loads a skill by itself when it meets a task that the skill is for,
see :mod:`columbina.skills`. A person can do the same by hand: ``/skill``
with the name of a skill gives the agent its text, and whatever follows the
name is the task to do by it. That is also the only way to a skill that says
that a model may not load it by itself.

``/skills`` shows what there is: every skill with what it is for and where
it was found, the skills that were passed over for another of the same
name, and what looks like a skill and cannot be used, with the reason.

A skill is not a command of its own. Commands and skills have their own
names, so a new skill never takes the place of a command.
"""

from columbina.agent import Agent
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.skills import SKILLS, Skills, loaded, one_line
from columbina.skills.files import Found, SkillError

__all__ = ["SOURCE", "apply", "inject", "name"]

name = "command-skills"
inject = (COMMANDS, SKILLS)

SOURCE = "skill"
"""The source of the message in which a person hands a skill to the agent."""

_USAGE = (
    "Say which skill: /skill NAME, and after the name what is to be done by it. /skills lists them."
)


def _listed(found: Found) -> str:
    """List what there is for a person: the skills, and what stands in the way of others."""
    lines: list[str] = []
    width = max((len(named) for named in found.skills), default=0)
    for named, skill in found.skills.items():
        only = (
            ""
            if skill.model_invocable and skill.user_invocable
            else "  [only the agent loads it]"
            if skill.model_invocable
            else "  [only with /skill]"
            if skill.user_invocable
            else "  [nobody can load it]"
        )
        lines.append(
            f"{named.ljust(width)}  ({skill.root.label})  {one_line(skill.description)}{only}"
        )
    if found.shadowed:
        lines.append(
            "Passed over, because a place that counts before theirs has a skill of that name:"
        )
        lines.extend(
            f"  {skill.name}  ({skill.root.label})  {skill.directory}" for skill in found.shadowed
        )
    if found.invalid:
        lines.append("Not usable:")
        lines.extend(f"  {invalid.directory}: {invalid.reason}" for invalid in found.invalid)
    return "\n".join(lines) if lines else "There are no skills."


def apply(ctx: Context, config: object) -> None:
    """Register the command that lists the skills and the one that hands a skill over."""
    skills: Skills = ctx.use(SKILLS)

    async def show(agent: Agent, rest: str) -> str:
        return _listed(await skills.found(agent.session, agent.cwd))

    async def hand_over(agent: Agent, rest: str) -> str:
        named, _, task = rest.partition(" ")
        if not named:
            return _USAGE
        found = await skills.found(agent.session, agent.cwd)
        skill = found.skills.get(named)
        if skill is None:
            return f"There is no skill named {named!r}. /skills lists the skills."
        try:
            now, text = await skills.body(skill)
        except SkillError as error:
            return f"The skill {named} cannot be used: {error}."
        except OSError as error:
            return f"The skill {named} cannot be read: {error.strerror or error}."
        if not now.user_invocable:
            return f"The skill {named} is for the agent to load by itself, and is not handed over."
        agent.inject(
            f"The user wants you to go by this skill.\n\n{loaded(now, text)}", source=SOURCE
        )
        task = task.strip()
        if task:
            agent.followup(task)
            return ""
        return f"The agent has the skill {named} now, and reads it with what you say next."

    commands = ctx.use(COMMANDS)
    commands.registry.add(
        ctx, "skills", Command(show, "List the skills, and what cannot be used as one.")
    )
    commands.registry.add(
        ctx, "skill", Command(hand_over, "Hand a skill to the agent: /skill NAME [TASK].")
    )
