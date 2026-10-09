"""The command that shows and changes what a session may do.

This module is a plugin. Loading it registers the command ``/permissions``.

Without a word after it, the command shows the mode of the sandbox, the
policy of approval, the rules, and what was allowed for the rest of the
session. With the name of a preset after it, it gives the session the mode
and the policy of that preset. The model is told of the change when its next
turn begins.
"""

from columbina.agent import Agent
from columbina.approval import APPROVAL, Approval
from columbina.commands import COMMANDS, Command
from columbina.kernel import Context
from columbina.permissions import PERMISSIONS, PRESETS, Permissions, Rule
from columbina.sandbox import SANDBOX, Sandbox

__all__ = ["apply", "inject", "name"]

name = "command-permissions"
inject = (COMMANDS, PERMISSIONS, SANDBOX, APPROVAL)


def _rules(kind: str, rules: tuple[Rule, ...]) -> str:
    return f"{kind}: {', '.join(str(rule) for rule in rules) or '(none)'}"


def apply(ctx: Context, config: object) -> None:
    """Register the command that shows and changes what a session may do."""
    permissions: Permissions = ctx.use(PERMISSIONS)
    sandbox: Sandbox = ctx.use(SANDBOX)
    approval: Approval = ctx.use(APPROVAL)

    async def shown(agent: Agent) -> str:
        session = agent.session
        # Before the first turn the log names neither, and the preset says what they will be.
        mode, policy = permissions.in_force(session)
        lines = [f"sandbox mode: {mode}", f"approval: {policy}"]
        rules = await permissions.prepare(session, agent.cwd)
        if rules is None:
            lines.append("The rules of this project cannot be read, so nothing is allowed.")
        else:
            lines.append(_rules("deny", rules.deny))
            lines.append(_rules("ask", rules.ask))
            lines.append(_rules("allow", rules.allow))
        for tool, target in permissions.granted(session):
            what = tool if target is None else f"{tool}: {target}"
            lines.append(f"allowed for the rest of this session: {what}")
        lines.append(f"To change the mode and the policy: /permissions {' | '.join(PRESETS)}")
        return "\n".join(lines)

    async def run(agent: Agent, rest: str) -> str:
        if not rest:
            return await shown(agent)
        if rest not in PRESETS:
            return f"There is no preset {rest!r}. The presets are: {', '.join(PRESETS)}."
        permissions.set_preset(agent.session, rest)
        session = agent.session
        return (
            f"sandbox mode: {sandbox.mode(session)}\napproval: {approval.policy(session)}\n"
            "The agent is told when its next turn begins."
        )

    told = "Show what the session may do, or change it to a preset."
    ctx.use(COMMANDS).registry.add(ctx, "permissions", Command(run, told))
