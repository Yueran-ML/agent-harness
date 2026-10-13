"""Permissions: the rules about what tools may do, and who is asked when no rule decides.

This module is a plugin. Loading it provides :data:`PERMISSIONS`.

**Where rules come from.** There are three kinds of rules: ``deny``, ``ask``
and ``allow``. How a rule is written is told in :mod:`.rules`.

- The config of this plugin, which is where the rules of the command line go.
- The file of the user, by default ``permissions.yml`` in the home of this
  program. It may hold all three kinds.
- The file of the project, ``.columbina/permissions.yml`` in the working
  directory. It may make things stricter and nothing else: its ``deny`` and
  ``ask`` count, and an ``allow`` in it is passed over with a warning. A
  repository that somebody cloned must not be able to approve its own
  commands.

**How a call is decided.** The first of these that applies settles it:

1. A ``deny`` rule speaks about the call: it is refused.
2. An ``ask`` rule speaks about it: somebody is asked.
3. An ``allow`` rule speaks about it: it goes ahead.
4. No rule speaks about it. A call that runs a command is asked about,
   unless the session has full access, or the sandbox holds the command
   fully. Every other call goes ahead, and what it may do to files is up to
   the seam of the file system.

A command is held fully where the sandbox runs it in something that lets it
do no more than the mode of the session allows, and send nothing away. A
tool says of itself that the sandbox runs what it runs. How well the sandbox
holds is known when the call is decided, and may be otherwise when the
command is run. So such a call is let through with a note for the sandbox,
which runs the command only if it is still held fully then.

A command is taken apart first (see :mod:`.commands`). A ``deny`` or ``ask``
rule counts if it speaks about the whole line or about any of its simple
commands. An ``allow`` rule lets a command go ahead only if every one of its
simple commands is allowed, and only if the command is of the plain shape
that can be taken apart at all. A rule that allows a tool without a pattern
allows whatever the tool is given.

**Where a rule counts.** A call is decided here, by a guard of the tool
pipeline, before it runs. That is about the file that the call names. A
search names one directory and then opens many files, so the rules about
files count a second time, at the seam, for every file that any tool
touches: a file that a ``deny`` rule speaks about is not touched, and a file
that an ``ask`` rule speaks about is passed over by a search, which asks
nobody.

**The preset.** A session has a mode of the sandbox and a policy of
approval. The preset of the config names both, and they are written into the
log of a session when its first turn begins, unless the log names them
already.

The rules of a project are read when the first call of a session is decided.
Until then, and if the file cannot be read, the session touches no file and
runs no tool: rules that are not known cannot be kept.
"""

import asyncio
import logging
import weakref
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, cast

from pydantic import BaseModel

from columbina import plain_yaml
from columbina.approval import APPROVAL, Approval, ApprovalRequest, Policy
from columbina.fs import FS, Access, Fs, Refuse, bound, locate
from columbina.kernel import Context, Registry, ServiceKey
from columbina.permissions.commands import Splitter, split_bash, split_powershell
from columbina.permissions.rules import (
    Place,
    Rule,
    covers_command,
    covers_file,
    parse_rule,
    place_of,
)
from columbina.plain_yaml import PlainYamlError
from columbina.sandbox import SANDBOX, Mode, Sandbox, Strength
from columbina.session import SESSION_EVENT, SESSIONS, Committed, Session, TurnStartEvent
from columbina.tools import TOOLS, Deny, Effect, Invocation, ToolFailure, Tools

__all__ = [
    "PERMISSIONS",
    "PRESETS",
    "PROJECT_FILE",
    "Config",
    "Decided",
    "Permissions",
    "Preset",
    "Rule",
    "RuleSet",
    "Splitter",
    "apply",
    "decide",
    "inject",
    "name",
    "read_rules",
    "split_bash",
    "split_powershell",
]

name = "permissions"
inject = (TOOLS, FS, SANDBOX, APPROVAL, SESSIONS)

logger = logging.getLogger(__name__)

PROJECT_FILE = ".columbina/permissions.yml"
"""Where the rules of a project are, from its working directory."""

MAX_FILE_BYTES = 256 * 1024
"""A file of rules that is larger is not read."""

_NOT_READ: dict[plain_yaml.Kind, str] = {
    "syntax": "it is not YAML that can be read",
    "depth": "it is nested too deeply",
    "value": "it gives a value that cannot be read",
    "alias": "it uses a value again by its name, which is not read here",
}
"""What is said of a file of rules that cannot be read. Nothing of what it holds is repeated."""

_SUMMARY_CHARACTERS = 200

Preset = Literal["read-only", "workspace-write", "full-access"]
"""A mode of the sandbox and a policy of approval under one name."""

_PRESETS: dict[Preset, tuple[Mode, Policy]] = {
    "read-only": ("read-only", "ask"),
    "workspace-write": ("workspace-write", "ask"),
    "full-access": ("danger-full-access", "never"),
}

PRESETS: tuple[str, ...] = tuple(_PRESETS)
"""The names of the presets."""

Verdict = Literal["deny", "ask", "allow"]
"""What is done with a call."""


@dataclass(frozen=True, slots=True)
class RuleSet:
    """Rules of the three kinds.

    Attributes:
        deny: The rules that refuse.
        ask: The rules that have somebody asked.
        allow: The rules that let a call go ahead.
    """

    deny: tuple[Rule, ...] = ()
    ask: tuple[Rule, ...] = ()
    allow: tuple[Rule, ...] = ()

    def __add__(self, other: "RuleSet") -> "RuleSet":
        return RuleSet(self.deny + other.deny, self.ask + other.ask, self.allow + other.allow)


@dataclass(frozen=True, slots=True)
class Decided:
    """What is done with a call, and why.

    Attributes:
        verdict: What is done.
        rule: The rule that settled it, or ``None`` if no rule spoke.
        why: The reason, in words for the model and for a person who is
            asked.
        held: Whether the call goes ahead for no other reason than that the
            sandbox holds its command fully. Whoever lets such a call
            through has that looked at again when the command is run.
    """

    verdict: Verdict
    rule: Rule | None
    why: str
    held: bool = False


class Config(BaseModel):
    """The config of the permissions.

    Attributes:
        preset: The mode of the sandbox and the policy of approval of a
            session whose log names none.
        user_file: The file with the rules of the user. One that is not
            there holds no rules.
        deny: Rules that refuse.
        ask: Rules that have somebody asked.
        allow: Rules that let a call go ahead.
    """

    preset: Preset = "workspace-write"
    user_file: Path | None = None
    deny: tuple[str, ...] = ()
    ask: tuple[str, ...] = ()
    allow: tuple[str, ...] = ()


def _parsed(texts: object, kind: str, source: str) -> tuple[Rule, ...]:
    """Read the rules of one kind from what a file or a config holds under that name."""
    if texts is None:
        return ()
    if not isinstance(texts, list | tuple):
        raise ValueError(f"{source}: {kind} has to be a list of rules")
    rules: list[Rule] = []
    for text in cast("Sequence[object]", texts):
        if not isinstance(text, str):
            raise ValueError(f"{source}: a rule under {kind} is not text")
        try:
            rules.append(parse_rule(text))
        except ValueError as error:
            raise ValueError(f"{source}: {error}") from None
    return tuple(rules)


def read_rules(text: str, source: str) -> RuleSet:
    """Read the rules that a file holds.

    The file is YAML with up to three lists, named ``deny``, ``ask`` and
    ``allow``. Each entry is one rule.

    Args:
        text: What the file holds.
        source: What to call the file in an error.

    Raises:
        ValueError: The file does not hold rules in that form. The message
            names the file and says what is wrong.
    """
    try:
        held = plain_yaml.load(text)
    except PlainYamlError as error:
        where = "" if error.line is None else f" at line {error.line}"
        raise ValueError(f"{source}: {_NOT_READ[error.kind]}{where}") from None
    if held is None:
        return RuleSet()
    if not isinstance(held, dict):
        raise ValueError(f"{source}: it has to hold the lists deny, ask and allow")
    kinds = cast("dict[object, object]", held)
    unknown = sorted(str(key) for key in kinds if key not in ("deny", "ask", "allow"))
    if unknown:
        raise ValueError(
            f"{source}: it holds {', '.join(unknown)}, and only deny, ask and allow are known"
        )
    return RuleSet(
        _parsed(kinds.get("deny"), "deny", source),
        _parsed(kinds.get("ask"), "ask", source),
        _parsed(kinds.get("allow"), "allow", source),
    )


def decide(
    rules: RuleSet,
    tool: str,
    effect: Effect,
    target: str | None,
    place: Place | None,
    *,
    mode: Mode,
    splitter: Splitter | None,
    granted: bool = False,
    held: Strength = "none",
) -> Decided:
    """Say what is done with a call.

    Args:
        rules: The rules that count.
        tool: The name of the tool.
        effect: What the tool does.
        target: What the call acts on, as the tool says it. A tool that runs
            something may name nothing, as a tool of a server does. Its call
            is then asked about as a call of the tool, and not as a command.
        place: Where the file is that the call names, for a tool that reads
            or writes one.
        mode: The mode of the sandbox of the session.
        splitter: What takes a command of this tool apart, if the tool runs
            commands and has registered one.
        granted: Whether this very call was allowed for the rest of the
            session. That counts after the rules that refuse and before
            those that ask. It does not count for a command that cannot be
            taken apart. For a tool that runs something and names no
            command, such a yes is for every call of the tool.
        held: How well the sandbox holds what the tool runs. It is ``none``
            for a tool whose commands the sandbox does not run. A command
            that is held fully needs no yes, unless a rule asks for one.
    """
    if effect != "run":
        for rule in rules.deny:
            if covers_file(rule, tool, effect, place):
                return Decided("deny", rule, _said_by("deny", rule))
        if granted:
            return Decided("allow", None, _GRANTED)
        later: tuple[tuple[Verdict, tuple[Rule, ...]], ...] = (
            ("ask", rules.ask),
            ("allow", rules.allow),
        )
        for verdict, kind in later:
            for rule in kind:
                if covers_file(rule, tool, effect, place):
                    return Decided(verdict, rule, _said_by(verdict, rule))
        return Decided("allow", None, "No rule speaks about this call.")
    command = target or ""
    apart = None if splitter is None else splitter(command)
    spoken_of = (command, *(apart or ()))
    for rule in rules.deny:
        if any(covers_command(rule, tool, text) for text in spoken_of):
            return Decided("deny", rule, _said_by("deny", rule))
    # A tool that names nothing and has nothing that takes commands apart is no tool for
    # commands. A yes for it is a yes for the tool, as a rule that names it would be.
    as_a_whole = target is None and splitter is None
    if granted and (apart is not None or as_a_whole):
        return Decided("allow", None, _GRANTED)
    for rule in rules.ask:
        if any(covers_command(rule, tool, text) for text in spoken_of):
            return Decided("ask", rule, _said_by("ask", rule))
    for rule in rules.allow:
        if rule.pattern is None and covers_command(rule, tool, command):
            # The tool is allowed, whatever it is given.
            return Decided("allow", rule, _said_by("allow", rule))
    if apart is not None:
        allowing = [
            next((rule for rule in rules.allow if covers_command(rule, tool, simple)), None)
            for simple in apart
        ]
        if all(rule is not None for rule in allowing):
            return Decided("allow", allowing[0], "Rules allow every command in the line.")
    if mode == "danger-full-access":
        return Decided("allow", None, "The session has full access.")
    if held == "full":
        return Decided("allow", None, _HELD, held=True)
    if target is None:
        # Nothing of a command here: a tool that does what somebody else's program likes,
        # such as a tool of a server. What is said of commands would lead astray.
        return Decided("ask", None, "No rule allows calls of this tool.")
    if apart is None and rules.allow:
        return Decided(
            "ask",
            None,
            "No rule can allow this command: it is not of the plain shape that can be taken "
            "apart with certainty.",
        )
    return Decided("ask", None, "No rule allows this command, and nothing holds it here.")


_GRANTED = "It was allowed for the rest of this session."
_HELD = "The sandbox holds the command fully, so nobody needs to be asked."


def _said_by(verdict: Verdict, rule: Rule) -> str:
    if verdict == "deny":
        return f"The rule {rule} is against it."
    if verdict == "ask":
        return f"The rule {rule} wants somebody to be asked first."
    return f"The rule {rule} allows it."


@dataclass(frozen=True, slots=True)
class _Unreadable:
    """The rules of a project could not be read.

    Attributes:
        why: What is wrong, in words for the model and the user.
    """

    why: str


def _project_rules(cwd: Path) -> RuleSet | _Unreadable:
    """Read the rules of the project in a working directory. This blocks.

    The file belongs to the repository, which somebody else may have written.
    It is opened only where it lies in the working directory, and nothing of
    what it holds is repeated when it cannot be read: it could be a link to a
    file that is nobody's business.
    """
    source = f"the rules of the project in {PROJECT_FILE}"
    try:
        where = locate(cwd, PROJECT_FILE)
    except ToolFailure as failure:
        return _Unreadable(f"{source} cannot be found: {failure}")
    if not where.inside:
        return _Unreadable(f"{source} lead out of the working directory, so they were not read.")
    try:
        file, status = bound.open_for_reading(where.path)
    except FileNotFoundError:
        return RuleSet()
    except OSError as error:
        return _Unreadable(f"{source} cannot be opened: {type(error).__name__}.")
    with file:
        if status.st_size > MAX_FILE_BYTES:
            return _Unreadable(f"{source} are larger than {MAX_FILE_BYTES // 1024} KiB.")
        raw = file.read(MAX_FILE_BYTES + 1)
    try:
        written = raw.decode("utf-8")
    except UnicodeDecodeError:
        return _Unreadable(f"{source} are not text in UTF-8.")
    try:
        held = read_rules(written, source)
    except ValueError as error:
        return _Unreadable(f"{error}.")
    if held.allow:
        # A repository does not approve its own commands.
        logger.warning(
            "%s hold rules under allow, which a project cannot give, so they were passed over",
            source,
        )
    return RuleSet(held.deny, held.ask)


def _summary(tool: str, target: str | None) -> str:
    """Return one line that says what a call is, for a log and for a person."""
    if target is None:
        return tool
    line = " ".join(target.split())
    if len(line) > _SUMMARY_CHARACTERS:
        line = f"{line[:_SUMMARY_CHARACTERS]} ... [{len(line) - _SUMMARY_CHARACTERS} more]"
    return f"{tool}: {line}"


class Permissions:
    """The rules, and what decides about calls and files by them.

    Attributes:
        splitters: What takes the commands of a tool apart, by the name of
            the tool. A tool that runs commands registers its own.
        sandboxed: The tools whose commands the sandbox runs, by their
            names. Such a tool registers itself. What a tool that is not
            here runs is held by nothing, whatever the sandbox holds.
    """

    __slots__ = (
        "_approval",
        "_fs",
        "_grants",
        "_preset",
        "_ready",
        "_rules",
        "_sandbox",
        "_tools",
        "sandboxed",
        "splitters",
    )

    def __init__(
        self,
        rules: RuleSet,
        preset: Preset,
        fs: Fs,
        sandbox: Sandbox,
        approval: Approval,
        tools: Tools,
    ) -> None:
        self.splitters: Registry[Splitter] = Registry("splitter of commands")
        self.sandboxed: Registry[bool] = Registry("tool that the sandbox runs")
        self._rules = rules
        self._preset: Preset = preset
        self._fs = fs
        self._sandbox = sandbox
        self._approval = approval
        self._tools = tools
        self._ready: weakref.WeakKeyDictionary[Session, RuleSet | _Unreadable] = (
            weakref.WeakKeyDictionary()
        )
        self._grants: weakref.WeakKeyDictionary[Session, set[tuple[str, str | None]]] = (
            weakref.WeakKeyDictionary()
        )

    def rules(self, session: Session) -> RuleSet | None:
        """Return the rules that count for a session, or ``None`` if they are not known.

        They are not known before the first call of the session was decided,
        and they stay unknown if the file of the project cannot be read.
        """
        ready = self._ready.get(session)
        return ready if isinstance(ready, RuleSet) else None

    async def prepare(self, session: Session, cwd: Path) -> RuleSet | None:
        """Read the rules of the project of a session, unless that was done.

        Returns:
            The rules that count for the session, or ``None`` if the file of
            the project cannot be read.
        """
        if session not in self._ready:
            project = await asyncio.to_thread(_project_rules, cwd)
            if isinstance(project, RuleSet):
                self._ready[session] = self._rules + project
            else:
                logger.error("%s", project.why)
                self._ready[session] = project
        return self.rules(session)

    def begin(self, session: Session) -> None:
        """Write the preset into the log of a session, as far as the log names none."""
        mode, policy = _PRESETS[self._preset]
        if self._sandbox.recorded(session) is None:
            self._sandbox.set_mode(session, mode)
        if self._approval.recorded(session) is None:
            self._approval.set_policy(session, policy)

    def in_force(self, session: Session) -> tuple[Mode, Policy]:
        """Return the mode of the sandbox and the policy of approval that a session goes by.

        What the log of the session names counts. What it does not name yet
        is that of the preset, which is written into the log when the first
        turn begins. So this is right before that turn too, which is when
        somebody looks at what a session may do.
        """
        mode, policy = _PRESETS[self._preset]
        return (
            self._sandbox.recorded(session) or mode,
            self._approval.recorded(session) or policy,
        )

    def standing(self, session: Session, tool: str, target: str | None) -> str | None:
        """Say what a yes for the rest of a session would cover, if one can be given.

        Such a yes is for one call as it stands: the same tool, acting on
        the same thing. For a command that is the same line, word for word,
        and only a line that can be taken apart, because a line with a
        variable in it may do another thing the next time.

        A tool that runs something and is no tool for commands, such as a
        tool of a server, names nothing that one of its calls could be told
        from another by. A yes for it is for every call of the tool, which
        is what a rule that names the tool allows as well.

        Args:
            session: The session.
            tool: The name of the tool.
            target: What the call acts on, as the tool says it.

        Returns:
            Words that finish the sentence "do not ask again about ...", or
            ``None`` if this call has to be asked about every time.
        """
        known = self._tools.registry.get(tool)
        if known is None:
            return None
        if known.effect == "run":
            splitter = self.splitters.get(tool)
            if target is None and splitter is None:
                return f"any call of {tool}"
            if target is None or splitter is None or splitter(target) is None:
                return None
            return "this command, word for word"
        return f"any call of {tool}" if target is None else f"{tool} on {target}"

    def grant(self, session: Session, tool: str, target: str | None) -> None:
        """Allow one call, as it stands, for the rest of a session.

        The yes is kept in memory. It is gone when the program ends, and a
        session that is taken up again is asked anew.

        Raises:
            ValueError: No such yes can be given for this call. See
                :meth:`standing`.
        """
        if self.standing(session, tool, target) is None:
            raise ValueError(f"a call of {tool} like this one has to be asked about every time")
        self._grants.setdefault(session, set()).add((tool, target))

    def granted(self, session: Session) -> tuple[tuple[str, str | None], ...]:
        """Return what was allowed for the rest of a session: tools, and what they act on."""
        held = self._grants.get(session, ())
        return tuple(sorted(held, key=lambda one: (one[0], one[1] or "")))

    def set_preset(self, session: Session, preset: str) -> None:
        """Give a session the mode of the sandbox and the policy of approval of a preset.

        Both are written into the log of the session, so they hold when the
        session is taken up again.

        Raises:
            ValueError: There is no preset of that name.
        """
        chosen = next((held for named, held in _PRESETS.items() if named == preset), None)
        if chosen is None:
            raise ValueError(f"there is no preset {preset!r}")
        mode, policy = chosen
        self._sandbox.set_mode(session, mode)
        self._approval.set_policy(session, policy)

    def _unknown(self, session: Session) -> Refuse:
        """Say why nothing is done in a session whose rules are not known."""
        ready = self._ready.get(session)
        if isinstance(ready, _Unreadable):
            return Refuse(
                f"{ready.why[0].upper()}{ready.why[1:]} Nothing is done until that is put "
                "right. Tell the user.",
                "RULES_UNREADABLE",
            )
        return Refuse(
            "The rules of this session have not been read yet, so nothing is touched.",
            "RULES_NOT_READ",
        )

    def restricts(self, access: Access) -> Refuse | None:
        """Refuse an access to a file that the rules do not let happen."""
        rules = self.rules(access.session)
        if rules is None:
            return self._unknown(access.session)
        place = place_of(access.where)
        for rule in rules.deny:
            if covers_file(rule, access.tool, access.kind, place):
                return Refuse(f"The rule {rule} keeps {access.where.written} from it.", "DENIED")
        if not access.named:
            for rule in rules.ask:
                if covers_file(rule, access.tool, access.kind, place):
                    return Refuse(
                        f"The rule {rule} wants somebody to be asked before "
                        f"{access.where.written} is touched, and a search asks nobody.",
                        "DENIED",
                    )
        return None

    async def guard(self, ctx: Context, invocation: Invocation) -> Deny | None:
        """Refuse a call that the rules do not let happen, after asking if they say so."""
        run = invocation.run
        session = run.session
        tool = invocation.tool
        rules = await self.prepare(session, run.cwd)
        if rules is None:
            unknown = self._unknown(session)
            return Deny(unknown.reason, unknown.code)
        place = None
        if tool.effect in ("read", "write") and invocation.target is not None:
            try:
                place = place_of(self._fs.locate(session, run.cwd, invocation.target))
            except ToolFailure:
                # What the call acts on is no path. No rule about files speaks about it.
                place = None
        decided = decide(
            rules,
            tool.name,
            tool.effect,
            invocation.target,
            place,
            mode=self._sandbox.mode(session),
            splitter=self.splitters.get(tool.name),
            granted=(tool.name, invocation.target) in self._grants.get(session, ()),
            held=self._sandbox.strength(session) if tool.name in self.sandboxed else "none",
        )
        if decided.verdict == "allow":
            if decided.held:
                # Let through for what the sandbox says now. It looks again when it runs
                # the command, and does not run it if it holds less well by then.
                self._sandbox.expect(session, run.call_id)
            return None
        if decided.verdict == "deny":
            return Deny(decided.why, "DENIED")
        outcome = await self._approval.ask(
            ctx,
            ApprovalRequest(
                session=session,
                call_id=run.call_id,
                tool=tool.name,
                summary=_summary(tool.name, invocation.target),
                reason=decided.why,
                target=invocation.target,
                standing=self.standing(session, tool.name, invocation.target),
            ),
        )
        if outcome == "allowed-once":
            return None
        if outcome == "rejected":
            asked = (
                "Nobody is asked in this session, so it was not done."
                if self._approval.policy(session) == "never"
                else "It was asked about, and the answer was no."
            )
            return Deny(f"{decided.why} {asked}", "APPROVAL_REJECTED")
        if outcome == "cancelled":
            return Deny(
                f"{decided.why} It was asked about, and the question was withdrawn.",
                "APPROVAL_CANCELLED",
            )
        return Deny(
            f"{decided.why} Nobody is here who could be asked, so it was not done.",
            "APPROVAL_UNAVAILABLE",
        )


PERMISSIONS = ServiceKey[Permissions]("permissions")
"""The permissions."""


def _user_rules(path: Path | None) -> RuleSet:
    """Read the rules of the user. A file that is not there holds none.

    Raises:
        ValueError: The file is there and does not hold rules.
    """
    if path is None:
        return RuleSet()
    source = f"the rules in {path}"
    try:
        raw = path.read_bytes()
    except FileNotFoundError:
        return RuleSet()
    except OSError as error:
        raise ValueError(f"{source} cannot be read: {error}") from error
    if len(raw) > MAX_FILE_BYTES:
        raise ValueError(f"{source} are larger than {MAX_FILE_BYTES // 1024} KiB")
    try:
        return read_rules(raw.decode("utf-8"), source)
    except UnicodeDecodeError:
        raise ValueError(f"{source} are not text in UTF-8") from None


def apply(ctx: Context, config: Config) -> None:
    """Provide the permissions, and have them asked about every call and every file.

    Raises:
        ValueError: A rule of the config or of the file of the user is no
            rule. Nothing is loaded then: rules that cannot be read are not
            passed over.
    """
    written = RuleSet(
        _parsed(config.deny, "deny", "the config of the permissions"),
        _parsed(config.ask, "ask", "the config of the permissions"),
        _parsed(config.allow, "allow", "the config of the permissions"),
    )
    permissions = Permissions(
        written + _user_rules(config.user_file),
        config.preset,
        ctx.use(FS),
        ctx.use(SANDBOX),
        ctx.use(APPROVAL),
        ctx.use(TOOLS),
    )

    def turn_begins(committed: Committed) -> None:
        if isinstance(committed.event.data, TurnStartEvent):
            permissions.begin(committed.session)

    ctx.on(SESSION_EVENT, turn_begins)
    ctx.use(FS).restrictions.add(ctx, "permissions", permissions.restricts)
    ctx.use(TOOLS).guards.add(ctx, "permissions", permissions.guard)
    ctx.provide(PERMISSIONS, permissions)
