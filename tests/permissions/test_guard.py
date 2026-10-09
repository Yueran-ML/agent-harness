"""The permissions at work: a guard before every call, and a restriction on every file."""

import json
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest
from pydantic import BaseModel

import columbina.approval
import columbina.fs
import columbina.permissions
import columbina.sandbox
import columbina.session
import columbina.tools
from columbina.approval import APPROVAL, APPROVAL_ASK, Approval, ApprovalRequest, Outcome
from columbina.builtin_tools import edit, glob, grep, read, write
from columbina.fs import FS, Access, Fs, locate
from columbina.kernel import Context
from columbina.llm import ToolCallBlock
from columbina.permissions import PERMISSIONS, PRESETS, PROJECT_FILE, Permissions
from columbina.permissions.commands import split_bash
from columbina.sandbox import SANDBOX, Sandbox
from columbina.session import SESSIONS, Session, Sessions, TurnStartEvent
from columbina.tools import TOOLS, Tool, ToolFailure, ToolOutcome, ToolRun, Tools
from support.links import link_directory


class CommandArgs(BaseModel):
    command: str


class NoteArgs(BaseModel):
    text: str = ""


@dataclass
class Bench:
    """A session with the tools that work on files, a tool that runs commands, and the permissions.

    Attributes:
        ran: The commands that the tool ``shell`` was let to run.
        asked: The questions that were put to the user.
        answer: What the user answers, or ``None`` if nobody is there to ask.
    """

    ctx: Context
    tools: Tools
    fs: Fs
    sandbox: Sandbox
    approval: Approval
    permissions: Permissions
    sessions: Sessions
    session: Session
    work: Path
    ran: list[str] = field(default_factory=list[str])
    asked: list[ApprovalRequest] = field(default_factory=list[ApprovalRequest])
    answer: Outcome | None = None
    _calls: int = 0

    async def call(self, name: str, **arguments: Any) -> ToolOutcome:
        """Call a tool the way a model does, and return how the call ended."""
        self._calls += 1
        call_id = f"call-{self._calls}"
        asked = ToolCallBlock(call_id=call_id, name=name, arguments=json.dumps(arguments))
        return await self.tools.run(self.ctx, asked, ToolRun(call_id, self.work, self.session))

    async def code(self, name: str, **arguments: Any) -> str | None:
        """Call a tool, and return the code of the error that came back, if one did."""
        outcome = await self.call(name, **arguments)
        return None if outcome.error is None else outcome.error.code

    async def shell(self, command: str) -> str | None:
        return await self.code("shell", command=command)

    def put(self, path: str, content: str = "needle\n") -> None:
        file = self.work / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content, encoding="utf-8")


async def bench(ctx: Context, tmp_path: Path, **config: Any) -> Bench:
    """Load everything, with ``config`` as the config of the permissions."""
    work = tmp_path / "work"
    work.mkdir(exist_ok=True)
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    await ctx.plugin(columbina.permissions, config)
    for tool in (read, write, edit, glob, grep):
        await ctx.plugin(tool)
    made = Bench(
        ctx=ctx,
        tools=ctx.use(TOOLS),
        fs=ctx.use(FS),
        sandbox=ctx.use(SANDBOX),
        approval=ctx.use(APPROVAL),
        permissions=ctx.use(PERMISSIONS),
        sessions=ctx.use(SESSIONS),
        session=ctx.use(SESSIONS).create(ctx, work),
        work=work,
    )

    async def run_command(args: CommandArgs, run: ToolRun) -> str:
        made.ran.append(args.command)
        return "ran"

    async def note(args: NoteArgs, run: ToolRun) -> str:
        return "noted"

    def command_of(args: CommandArgs, run: ToolRun) -> str:
        return args.command

    def responder(request: ApprovalRequest) -> Outcome | None:
        made.asked.append(request)
        return made.answer

    made.tools.add(
        ctx,
        Tool(
            name="shell",
            description="Run a command.",
            parameters=CommandArgs,
            execute=run_command,
            effect="run",
            target=command_of,
        ),
        Tool(name="note", description="Take a note.", parameters=NoteArgs, execute=note),
    )
    made.permissions.splitters.add(ctx, "shell", split_bash)
    ctx.on(APPROVAL_ASK, responder)
    return made


# A call that a rule refuses or allows


async def test_a_call_that_a_rule_refuses_does_not_run_and_says_which_rule(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, deny=["shell(rm *)"])

    outcome = await at.call("shell", command="rm -rf .")

    assert outcome.error is not None
    assert outcome.error.code == "DENIED"
    assert outcome.text == "The call was not allowed: The rule shell(rm *) is against it."
    assert at.ran == []
    assert at.asked == []


async def test_a_command_that_rules_allow_runs_without_a_question(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, allow=["shell(git status*)", "shell(git diff*)"])

    assert await at.shell("git status && git diff --stat") is None
    assert at.ran == ["git status && git diff --stat"]
    assert at.asked == []


async def test_a_call_that_does_nothing_to_the_world_needs_no_rule(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)

    assert await at.code("note", text="remember") is None
    assert at.asked == []


async def test_a_tool_that_registered_nothing_to_take_commands_apart_is_not_allowed_by_pattern(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, allow=["other(git status*)", "shell(git status*)"])

    async def run_command(args: CommandArgs, run: ToolRun) -> str:
        at.ran.append(args.command)
        return "ran"

    at.tools.add(
        ctx,
        Tool(
            name="other",
            description="Run a command.",
            parameters=CommandArgs,
            execute=run_command,
            effect="run",
            target=lambda args, run: args.command,
        ),
    )

    assert await at.code("other", command="git status") == "APPROVAL_UNAVAILABLE"
    assert await at.shell("git status") is None


# A call that somebody is asked about


async def test_a_command_that_no_rule_allows_is_asked_about_and_runs_on_a_yes(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.answer = "allowed-once"

    assert await at.shell("make  test\n") is None

    assert at.ran == ["make  test\n"]
    assert at.asked == [
        ApprovalRequest(
            session=at.session,
            call_id="call-1",
            tool="shell",
            summary="shell: make test",
            reason="No rule allows this command, and nothing holds it here.",
            target="make  test\n",
            standing="this command, word for word",
        )
    ]


async def test_a_yes_holds_for_the_one_call_that_was_asked_about(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.answer = "allowed-once"

    await at.shell("make")
    await at.shell("make")

    assert [request.call_id for request in at.asked] == ["call-1", "call-2"]


@pytest.mark.parametrize(
    ("answer", "code", "said"),
    [
        ("rejected", "APPROVAL_REJECTED", "It was asked about, and the answer was no."),
        ("cancelled", "APPROVAL_CANCELLED", "It was asked about, and the question was withdrawn."),
        ("unavailable", "APPROVAL_UNAVAILABLE", "Nobody is here who could be asked"),
        (None, "APPROVAL_UNAVAILABLE", "Nobody is here who could be asked, so it was not done."),
    ],
)
async def test_a_call_that_was_asked_about_and_not_allowed_does_not_run(
    ctx: Context, tmp_path: Path, answer: Outcome | None, code: str, said: str
) -> None:
    at = await bench(ctx, tmp_path)
    at.answer = answer

    outcome = await at.call("shell", command="make")

    assert outcome.error is not None
    assert outcome.error.code == code
    assert outcome.text.startswith(
        "The call was not allowed: No rule allows this command, and nothing holds it here. "
    )
    assert said in outcome.text
    assert at.ran == []


async def test_in_a_session_where_nobody_is_asked_the_answer_is_no_and_says_so(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.answer = "allowed-once"
    at.approval.set_policy(at.session, "never")

    outcome = await at.call("shell", command="make")

    assert outcome.error is not None
    assert outcome.error.code == "APPROVAL_REJECTED"
    assert outcome.text.endswith("Nobody is asked in this session, so it was not done.")
    assert at.asked == []
    assert at.ran == []


async def test_a_long_command_is_asked_about_by_its_beginning_and_handed_over_whole(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    long = "echo " + "x" * 500

    await at.shell(long)

    assert at.asked[0].summary == "shell: echo " + "x" * 195 + " ... [305 more]"
    assert at.asked[0].target == long


async def test_a_file_that_an_ask_rule_speaks_about_is_asked_about_when_a_call_names_it(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["read(config)"])
    at.put("config/app.yml", "port: 80\n")
    at.answer = "allowed-once"

    outcome = await at.call("read", path="config/app.yml")

    assert outcome.error is None
    assert "port: 80" in outcome.text
    assert [(request.tool, request.summary, request.reason) for request in at.asked] == [
        ("read", "read: config/app.yml", "The rule read(config) wants somebody to be asked first.")
    ]


async def test_a_call_that_names_nothing_is_asked_about_by_the_name_of_its_tool(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["note"])
    at.answer = "allowed-once"

    assert await at.code("note", text="remember") is None
    assert [(request.summary, request.target) for request in at.asked] == [("note", None)]


async def test_a_session_with_full_access_runs_commands_without_a_question(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.sandbox.set_mode(at.session, "danger-full-access")

    assert await at.shell("make $(nproc)") is None
    assert at.asked == []


# What was allowed for the rest of a session


async def test_a_question_says_what_a_yes_for_the_session_would_cover(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["write(new.txt)", "note"])

    await at.shell("git status")
    await at.shell("rm $TARGET")
    await at.code("write", path="./new.txt", content="x")
    await at.code("note", text="x")

    assert [request.standing for request in at.asked] == [
        "this command, word for word",
        None,
        "write on new.txt",
        "any call of note",
    ]


async def test_a_call_that_was_allowed_for_the_session_is_not_asked_about_again(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.answer = "allowed-once"
    assert await at.shell("git status") is None

    at.permissions.grant(at.session, "shell", "git status")
    at.answer = None

    assert await at.shell("git status") is None
    assert at.ran == ["git status", "git status"]
    assert len(at.asked) == 1
    assert at.permissions.granted(at.session) == (("shell", "git status"),)


async def test_the_yes_is_for_that_call_and_for_no_other(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path)
    at.permissions.grant(at.session, "shell", "git status")

    assert await at.shell("git status --short") == "APPROVAL_UNAVAILABLE"
    assert await at.shell("git status && rm -rf .") == "APPROVAL_UNAVAILABLE"
    assert await at.shell("git  status") == "APPROVAL_UNAVAILABLE"
    assert at.ran == []


async def test_what_was_allowed_in_one_session_is_asked_about_in_another(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    other = at.sessions.create(ctx, at.work)
    at.permissions.grant(other, "shell", "git status")

    assert await at.shell("git status") == "APPROVAL_UNAVAILABLE"
    assert at.permissions.granted(at.session) == ()
    assert at.permissions.granted(other) == (("shell", "git status"),)


async def test_a_rule_that_refuses_is_stronger_than_a_yes_for_the_session(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, deny=["shell(git push*)"])
    at.permissions.grant(at.session, "shell", "git push")

    assert await at.shell("git push") == "DENIED"
    assert at.ran == []


async def test_a_rule_that_asks_gives_way_to_a_yes_for_the_session(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["write(new.txt)"])
    assert await at.code("write", path="new.txt", content="one") == "APPROVAL_UNAVAILABLE"

    at.permissions.grant(at.session, "write", "new.txt")

    assert await at.code("write", path="./new.txt", content="one") is None
    assert await at.code("write", path="new.txt", content="two") is None
    assert (at.work / "new.txt").read_text(encoding="utf-8") == "two"
    assert len(at.asked) == 1


async def test_a_yes_for_a_file_is_for_the_tool_that_was_asked_about(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["write"])
    at.put("notes.txt")
    await at.code("read", path="notes.txt")
    at.fs.delivered(at.session)
    at.permissions.grant(at.session, "write", "notes.txt")

    assert await at.code("edit", path="notes.txt", old="needle", new="thread") == (
        "APPROVAL_UNAVAILABLE"
    )
    assert await at.code("write", path="other.txt", content="x") == "APPROVAL_UNAVAILABLE"
    assert await at.code("write", path="notes.txt", content="x") is None


async def test_a_tool_without_anything_that_it_acts_on_is_allowed_as_a_whole(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["note"])
    at.permissions.grant(at.session, "note", None)

    assert await at.code("note", text="one") is None
    assert await at.code("note", text="another") is None
    assert at.asked == []
    assert at.permissions.granted(at.session) == (("note", None),)


@pytest.mark.parametrize(
    ("tool", "target"),
    [("shell", "rm $TARGET"), ("shell", None), ("nowhere", "x"), ("bare", "git status")],
)
async def test_a_call_that_has_to_be_asked_about_every_time_cannot_be_allowed_for_a_session(
    ctx: Context, tmp_path: Path, tool: str, target: str | None
) -> None:
    at = await bench(ctx, tmp_path)

    async def run_command(args: CommandArgs, run: ToolRun) -> str:
        return "ran"

    # A tool that runs commands and registered nothing that takes them apart.
    at.tools.add(
        ctx,
        Tool(
            name="bare",
            description="Run.",
            parameters=CommandArgs,
            execute=run_command,
            effect="run",
            target=lambda args, run: args.command,
        ),
    )

    assert at.permissions.standing(at.session, tool, target) is None
    with pytest.raises(ValueError, match="has to be asked about every time"):
        at.permissions.grant(at.session, tool, target)
    assert at.permissions.granted(at.session) == ()


async def test_what_was_allowed_is_told_in_one_order(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path)
    at.permissions.grant(at.session, "write", "b.txt")
    at.permissions.grant(at.session, "shell", "git status")
    at.permissions.grant(at.session, "note", None)
    at.permissions.grant(at.session, "write", "a.txt")
    at.permissions.grant(at.session, "write", "a.txt")

    assert at.permissions.granted(at.session) == (
        ("note", None),
        ("shell", "git status"),
        ("write", "a.txt"),
        ("write", "b.txt"),
    )


# Rules about files, at every file


async def test_a_file_that_a_rule_keeps_from_reading_is_kept_from_every_tool(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, deny=["read(.env)"])
    at.put(".env", "needle=secret\n")
    at.put("app/.env", "needle=secret\n")
    at.put("notes.txt", "needle\n")

    assert await at.code("read", path=".env") == "DENIED"
    assert await at.code("read", path="app/../.env") == "DENIED"
    assert await at.code("read", path=str(at.work / "app" / ".env")) == "DENIED"
    assert await at.code("grep", pattern="needle", path="app/.env") == "DENIED"
    assert await at.code("edit", path=".env", old="secret", new="x") == "DENIED"
    searched = await at.call("grep", pattern="needle")
    listed = await at.call("glob", pattern="**/*")

    assert searched.text.splitlines() == [
        "notes.txt:1:needle",
        "[2 files and directories were not searched, because this tool may not see them.]",
    ]
    assert listed.text.splitlines()[0] == "notes.txt"
    assert ".env" not in listed.text.splitlines()


async def test_a_refusal_at_a_file_says_which_rule_it_was(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path, deny=["write(docs)"])
    at.put("docs/a.md", "old\n")
    await at.call("read", path="docs/a.md")
    at.fs.delivered(at.session)

    outcome = await at.call("edit", path="docs/a.md", old="old", new="new")

    assert outcome.error is not None
    assert outcome.error.code == "DENIED"
    assert outcome.text == "The call was not allowed: The rule write(docs) is against it."
    refusal = at.fs.judge(Access(at.session, at.work, locate(at.work, "docs/a.md"), "write", "x"))
    assert refusal is not None
    assert refusal.reason == "The rule write(docs) keeps docs/a.md from it."


async def test_a_file_that_an_ask_rule_speaks_about_is_passed_over_by_a_search(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, ask=["read(config)"])
    at.put("config/app.yml", "needle\n")
    at.put("notes.txt", "needle\n")
    at.answer = "allowed-once"

    searched = await at.call("grep", pattern="needle")

    assert searched.text.splitlines() == [
        "notes.txt:1:needle",
        "[1 file or directory was not searched, because this tool may not see them.]",
    ]
    assert at.asked == []
    refusal = at.fs.judge(
        Access(at.session, at.work, locate(at.work, "config/app.yml"), "read", "grep", named=False)
    )
    assert refusal is not None
    assert refusal.code == "DENIED"
    assert refusal.reason == (
        "The rule read(config) wants somebody to be asked before config/app.yml is touched, "
        "and a search asks nobody."
    )


async def test_a_rule_about_a_path_outside_speaks_about_it_whatever_the_working_directory(
    ctx: Context, tmp_path: Path
) -> None:
    secret = tmp_path / "elsewhere" / "id_rsa"
    secret.parent.mkdir()
    secret.write_text("key\n")
    at = await bench(ctx, tmp_path, deny=[f"read({secret.parent.as_posix()})", "read(../beside)"])
    (tmp_path / "beside").mkdir()
    (tmp_path / "beside" / "a.txt").write_text("a\n")
    at.sandbox.set_mode(at.session, "danger-full-access")

    assert await at.code("read", path=str(secret)) == "DENIED"
    assert await at.code("read", path="../beside/a.txt") == "DENIED"
    assert await at.code("read", path="../elsewhere/../beside/a.txt") == "DENIED"


async def test_a_rule_about_files_does_not_speak_about_what_is_no_path(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, deny=["read(*)"])

    async def look(args: NoteArgs, run: ToolRun) -> str:
        return "looked"

    at.tools.add(
        ctx,
        Tool(
            name="lookup",
            description="Look something up.",
            parameters=NoteArgs,
            execute=look,
            effect="read",
            target=lambda args, run: chr(0),
        ),
    )

    assert await at.code("lookup") is None


# The rules of a project


async def test_a_project_can_refuse_and_ask_and_cannot_allow(
    ctx: Context, tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    work = tmp_path / "work"
    (work / ".columbina").mkdir(parents=True)
    (work / PROJECT_FILE).write_text(
        "deny:\n  - shell(rm *)\nask:\n  - read(config)\nallow:\n  - shell\n  - shell(make*)\n"
    )
    with caplog.at_level(logging.WARNING, logger="columbina.permissions"):
        at = await bench(ctx, tmp_path)
        at.put("config/app.yml")

        assert await at.shell("rm x") == "DENIED"
        assert await at.shell("make") == "APPROVAL_UNAVAILABLE"
        assert await at.code("read", path="config/app.yml") == "APPROVAL_UNAVAILABLE"

    assert at.ran == []
    assert "hold rules under allow, which a project cannot give" in caplog.text
    assert caplog.text.count("which a project cannot give") == 1


async def test_the_rules_of_a_project_count_beside_those_of_the_config(
    ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    (work / ".columbina").mkdir(parents=True)
    (work / PROJECT_FILE).write_text("deny:\n  - shell(git push*)\n")
    at = await bench(ctx, tmp_path, allow=["shell(git *)"])

    assert await at.shell("git status") is None
    assert await at.shell("git push") == "DENIED"


async def test_a_project_without_rules_of_its_own_has_those_of_the_config(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, deny=["shell(rm *)"])

    assert at.permissions.rules(at.session) is None
    assert await at.shell("rm x") == "DENIED"
    held = at.permissions.rules(at.session)
    assert held is not None
    assert [str(rule) for rule in held.deny] == ["shell(rm *)"]


@pytest.mark.parametrize(
    ("content", "wrong"),
    [
        (b"deny: [a\n", "it is not YAML that can be read at line"),
        (b"deny:\n  - 'read('\n", "does not close it"),
        (b"forbid: []\n", "only deny, ask and allow are known"),
        (b"\xff\xfe\x00", "are not text in UTF-8"),
        (b"# " + b"x" * (256 * 1024), "are larger than 256 KiB"),
    ],
    ids=["no yaml", "no rule", "another kind", "no text", "too large"],
)
async def test_a_project_whose_rules_cannot_be_read_has_nothing_done_in_it(
    ctx: Context, tmp_path: Path, content: bytes, wrong: str, caplog: pytest.LogCaptureFixture
) -> None:
    work = tmp_path / "work"
    (work / ".columbina").mkdir(parents=True)
    (work / PROJECT_FILE).write_bytes(content)
    with caplog.at_level(logging.ERROR, logger="columbina.permissions"):
        at = await bench(ctx, tmp_path, allow=["shell"])
        at.put("notes.txt")
        at.sandbox.set_mode(at.session, "danger-full-access")

        outcome = await at.call("shell", command="ls")
        again = await at.call("read", path="notes.txt")
        harmless = await at.call("note", text="x")

    for refused in (outcome, again, harmless):
        assert refused.error is not None
        assert refused.error.code == "RULES_UNREADABLE"
        assert wrong in refused.text
        assert refused.text.endswith("Nothing is done until that is put right. Tell the user.")
    assert outcome.text.startswith(
        "The call was not allowed: The rules of the project in .columbina/permissions.yml"
    )
    assert at.ran == []
    assert at.permissions.rules(at.session) is None
    refusal = at.fs.judge(Access(at.session, at.work, locate(at.work, "notes.txt"), "read", "x"))
    assert refusal is not None
    assert refusal.code == "RULES_UNREADABLE"
    assert caplog.text.count(wrong) == 1


async def test_rules_of_a_project_that_lead_out_of_it_are_not_read(
    ctx: Context, tmp_path: Path
) -> None:
    # A repository could point its rules at a file that is nobody's business.
    work = tmp_path / "work"
    work.mkdir()
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "permissions.yml").write_text("password: hunter2\n- broken")
    link_directory(work / ".columbina", elsewhere)
    at = await bench(ctx, tmp_path)

    outcome = await at.call("note", text="x")

    assert outcome.error is not None
    assert outcome.error.code == "RULES_UNREADABLE"
    assert "lead out of the working directory, so they were not read" in outcome.text
    assert "hunter2" not in outcome.text


async def test_rules_of_a_project_that_no_path_leads_to_are_not_read(
    ctx: Context, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def nowhere(cwd: Path, written: str) -> Any:
        raise ToolFailure("a link on the way leads to another machine.", code="INVALID_PATH")

    at = await bench(ctx, tmp_path)
    monkeypatch.setattr(columbina.permissions, "locate", nowhere)

    outcome = await at.call("note", text="x")

    assert outcome.error is not None
    assert outcome.error.code == "RULES_UNREADABLE"
    assert "cannot be found: a link on the way leads to another machine." in outcome.text


async def test_rules_of_a_project_that_are_no_file_cannot_be_opened(
    ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    (work / PROJECT_FILE).mkdir(parents=True)
    at = await bench(ctx, tmp_path)

    outcome = await at.call("note", text="x")

    assert outcome.error is not None
    assert outcome.error.code == "RULES_UNREADABLE"
    assert "cannot be opened" in outcome.text


async def test_before_the_first_call_of_a_session_was_decided_no_file_is_touched(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)
    at.put("notes.txt")

    refusal = at.fs.judge(Access(at.session, at.work, locate(at.work, "notes.txt"), "read", "x"))

    assert refusal is not None
    assert refusal.code == "RULES_NOT_READ"
    assert refusal.reason == (
        "The rules of this session have not been read yet, so nothing is touched."
    )


async def test_the_rules_of_a_project_are_read_once_for_a_session(
    ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    (work / ".columbina").mkdir(parents=True)
    (work / PROJECT_FILE).write_text("deny:\n  - shell(rm *)\n")
    at = await bench(ctx, tmp_path)
    at.sandbox.set_mode(at.session, "danger-full-access")

    assert await at.shell("rm x") == "DENIED"
    (work / PROJECT_FILE).write_text("deny: []\n")

    assert await at.shell("rm x") == "DENIED"
    later = at.sessions.create(ctx, work)
    at.sandbox.set_mode(later, "danger-full-access")
    assert await at.tools.run(
        ctx,
        ToolCallBlock(call_id="c", name="shell", arguments='{"command": "rm x"}'),
        ToolRun("c", work, later),
    ) == ToolOutcome(
        call=ToolCallBlock(call_id="c", name="shell", arguments='{"command": "rm x"}'),
        text="ran",
        value="ran",
    )


# The rules of the user and of the config


async def test_the_file_of_the_user_may_hold_rules_of_all_three_kinds(
    ctx: Context, tmp_path: Path
) -> None:
    mine = tmp_path / "home" / "permissions.yml"
    mine.parent.mkdir()
    mine.write_text(
        "deny:\n  - shell(rm *)\nask:\n  - shell(git push*)\nallow:\n  - shell(git *)\n"
    )
    at = await bench(ctx, tmp_path, user_file=mine)

    assert await at.shell("git status") is None
    assert await at.shell("git push") == "APPROVAL_UNAVAILABLE"
    assert await at.shell("rm x") == "DENIED"


async def test_a_file_of_the_user_that_is_not_there_holds_no_rules(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, user_file=tmp_path / "nowhere" / "permissions.yml")

    assert await at.code("note") is None


@pytest.mark.parametrize(
    ("content", "wrong"),
    [
        (b"deny:\n  - 'read('\n", "does not close it"),
        (b"\xff\xfe", "are not text in UTF-8"),
        (b"# " + b"x" * (256 * 1024), "are larger than 256 KiB"),
    ],
    ids=["no rule", "no text", "too large"],
)
async def test_a_file_of_the_user_that_cannot_be_read_keeps_the_permissions_from_loading(
    ctx: Context, tmp_path: Path, content: bytes, wrong: str
) -> None:
    mine = tmp_path / "permissions.yml"
    mine.write_bytes(content)

    with pytest.raises(ValueError, match=wrong):
        await bench(ctx, tmp_path, user_file=mine)

    assert ctx.get(PERMISSIONS) is None


async def test_a_file_of_the_user_that_is_a_directory_keeps_the_permissions_from_loading(
    ctx: Context, tmp_path: Path
) -> None:
    mine = tmp_path / "permissions.yml"
    mine.mkdir()

    with pytest.raises(ValueError, match="cannot be read"):
        await bench(ctx, tmp_path, user_file=mine)


async def test_a_rule_of_the_config_that_is_no_rule_keeps_the_permissions_from_loading(
    ctx: Context, tmp_path: Path
) -> None:
    with pytest.raises(ValueError, match="the config of the permissions: the rule 'shell\\('"):
        await bench(ctx, tmp_path, allow=["shell("])


# The preset


@pytest.mark.parametrize(
    ("preset", "mode", "policy"),
    [
        ("read-only", "read-only", "ask"),
        ("workspace-write", "workspace-write", "ask"),
        ("full-access", "danger-full-access", "never"),
    ],
)
async def test_the_preset_is_written_into_the_log_when_the_first_turn_begins(
    ctx: Context, tmp_path: Path, preset: str, mode: str, policy: str
) -> None:
    at = await bench(ctx, tmp_path, preset=preset)

    assert (at.sandbox.recorded(at.session), at.approval.recorded(at.session)) == (None, None)

    at.session.append(TurnStartEvent(turn=1))

    assert (at.sandbox.recorded(at.session), at.approval.recorded(at.session)) == (mode, policy)
    assert [event.type for event in at.session.events] == [
        "turn/start",
        "sandbox/mode",
        "approval/policy",
    ]


async def test_the_preset_is_workspace_write_unless_the_config_says_otherwise(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path)

    at.session.append(TurnStartEvent(turn=1))

    assert at.sandbox.recorded(at.session) == "workspace-write"
    assert at.approval.recorded(at.session) == "ask"


async def test_what_the_log_names_already_is_not_written_again(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, preset="full-access")
    at.sandbox.set_mode(at.session, "read-only")

    at.session.append(TurnStartEvent(turn=1))
    at.session.append(TurnStartEvent(turn=2))

    assert at.sandbox.recorded(at.session) == "read-only"
    assert at.approval.recorded(at.session) == "never"
    assert [event.type for event in at.session.events] == [
        "sandbox/mode",
        "turn/start",
        "approval/policy",
        "turn/start",
    ]


async def test_a_policy_that_the_log_names_already_is_kept_whatever_the_preset(
    ctx: Context, tmp_path: Path
) -> None:
    at = await bench(ctx, tmp_path, preset="full-access")
    at.approval.set_policy(at.session, "ask")

    at.session.append(TurnStartEvent(turn=1))

    assert at.approval.recorded(at.session) == "ask"
    assert at.sandbox.recorded(at.session) == "danger-full-access"


async def test_other_events_of_the_log_write_no_preset(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path)

    at.approval.set_policy(at.session, "ask")

    assert at.sandbox.recorded(at.session) is None


@pytest.mark.parametrize(
    ("before", "preset", "mode", "policy"),
    [
        ("full-access", "read-only", "read-only", "ask"),
        ("full-access", "workspace-write", "workspace-write", "ask"),
        ("read-only", "full-access", "danger-full-access", "never"),
    ],
)
async def test_a_session_can_be_given_another_preset_on_its_way(
    ctx: Context, tmp_path: Path, before: str, preset: str, mode: str, policy: str
) -> None:
    at = await bench(ctx, tmp_path, preset=before)
    at.session.append(TurnStartEvent(turn=1))

    at.permissions.set_preset(at.session, preset)

    assert (at.sandbox.recorded(at.session), at.approval.recorded(at.session)) == (mode, policy)
    # What the log names is kept when the next turn begins.
    at.session.append(TurnStartEvent(turn=2))
    assert (at.sandbox.recorded(at.session), at.approval.recorded(at.session)) == (mode, policy)


async def test_a_preset_that_does_not_exist_changes_nothing(ctx: Context, tmp_path: Path) -> None:
    at = await bench(ctx, tmp_path)
    at.session.append(TurnStartEvent(turn=1))
    written = len(at.session.events)

    with pytest.raises(ValueError, match="there is no preset 'everything'"):
        at.permissions.set_preset(at.session, "everything")

    assert len(at.session.events) == written
    assert PRESETS == ("read-only", "workspace-write", "full-access")


# What leaves with the plugin


async def test_what_the_permissions_registered_leaves_with_them(
    ctx: Context, tmp_path: Path
) -> None:
    work = tmp_path / "work"
    work.mkdir()
    await ctx.plugin(columbina.session, {"root": tmp_path / "sessions"})
    await ctx.plugin(columbina.tools)
    await ctx.plugin(columbina.fs)
    await ctx.plugin(columbina.sandbox)
    await ctx.plugin(columbina.approval)
    loaded = await ctx.plugin(columbina.permissions)
    fs, tools = ctx.use(FS), ctx.use(TOOLS)
    assert "permissions" in fs.restrictions
    assert "permissions" in tools.guards

    await loaded.dispose()

    assert "permissions" not in fs.restrictions
    assert "permissions" not in tools.guards
    assert ctx.get(PERMISSIONS) is None
