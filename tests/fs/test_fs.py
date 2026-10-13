"""The seam of the file system: what may be touched, and how it is touched."""

import gc
import logging
import shutil
import sys
import weakref
from pathlib import Path
from typing import Any, cast

import pytest

import columbina.fs
from columbina.fs import FS, Access, Fingerprint, Fs, Kind, Permit, Refuse, locate
from columbina.kernel import Context, Plugin, RegistryConflictError
from columbina.session import CORE_EVENTS, Session, StepEndEvent, StepStartEvent
from columbina.tools import ToolFailure
from support.links import link_directory, unlink_directory


@pytest.fixture
async def fs(ctx: Context) -> Fs:
    await ctx.plugin(columbina.fs)
    return ctx.use(FS)


@pytest.fixture
def session(ctx: Context) -> Session:
    return Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})


@pytest.fixture
def work(tmp_path: Path) -> Path:
    directory = tmp_path / "work"
    directory.mkdir()
    (directory / "a.txt").write_text("inside")
    (tmp_path / "outside.txt").write_text("outside")
    return directory


class Asking:
    """Makes the accesses of one session to the files of one working directory."""

    def __init__(self, session: Session, work: Path) -> None:
        self.session = session
        self.work = work

    def __call__(
        self, kind: Kind, written: str, *, tool: str = "probe", named: bool = True
    ) -> Access:
        return Access(self.session, self.work, locate(self.work, written), kind, tool, named)


@pytest.fixture
def access(session: Session, work: Path) -> Asking:
    return Asking(session, work)


def allowing(seen: list[str] | None = None, name: str = "grant") -> Any:
    def grant(asked: Access) -> bool:
        if seen is not None:
            seen.append(name)
        return True

    return grant


def refusing(reason: str, seen: list[str] | None = None) -> Any:
    def restriction(asked: Access) -> Refuse | None:
        if seen is not None:
            seen.append(reason)
        return Refuse(reason, "NOT_THIS")

    return restriction


def silent(seen: list[str], name: str) -> Any:
    def restriction(asked: Access) -> Refuse | None:
        seen.append(name)
        return None

    return restriction


# What the seam allows by itself


def test_a_file_in_the_working_directory_may_be_read(fs: Fs, access: Asking) -> None:
    assert fs.judge(access("read", "a.txt")) is None
    assert fs.judge(access("read", "not-there-yet.txt")) is None


def test_a_file_outside_the_working_directory_may_not_be_read(fs: Fs, access: Asking) -> None:
    refusal = fs.judge(access("read", "../outside.txt"))

    assert refusal == Refuse(
        "The path ../outside.txt is outside the working directory. "
        "Only files inside it can be read.",
        "OUTSIDE_WORKING_DIRECTORY",
    )


def test_nothing_may_be_written_unless_something_allows_it(fs: Fs, access: Asking) -> None:
    assert fs.judge(access("write", "a.txt")) == Refuse(
        "The path a.txt cannot be written: nothing here allows writing files.", "NOT_WRITABLE"
    )
    assert fs.judge(access("write", "../outside.txt")) == Refuse(
        "The path ../outside.txt is outside the working directory. "
        "Only files inside it can be written.",
        "OUTSIDE_WORKING_DIRECTORY",
    )


# Grants


def test_a_grant_allows_what_the_seam_does_not(fs: Fs, access: Asking, ctx: Context) -> None:
    def only_writes_inside(asked: Access) -> bool:
        return asked.kind == "write" and asked.where.inside

    fs.grants.add(ctx, "writes", only_writes_inside)

    assert fs.judge(access("write", "a.txt")) is None
    assert fs.judge(access("write", "../outside.txt")) is not None
    assert fs.judge(access("read", "../outside.txt")) is not None


def test_one_grant_is_enough(fs: Fs, access: Asking, ctx: Context) -> None:
    asked: list[str] = []

    def never(access: Access) -> bool:
        asked.append("never")
        return False

    fs.grants.add(ctx, "never", never)
    fs.grants.add(ctx, "always", allowing(asked, "always"))
    fs.grants.add(ctx, "later", allowing(asked, "later"))

    assert fs.judge(access("read", "../outside.txt")) is None
    assert asked == ["never", "always"]


def test_a_grant_is_not_asked_about_what_the_seam_allows_anyway(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    asked: list[str] = []
    fs.grants.add(ctx, "always", allowing(asked))

    assert fs.judge(access("read", "a.txt")) is None
    assert asked == []


@pytest.mark.parametrize("answer", ["yes", 1, None, False])
def test_a_grant_allows_only_by_saying_so(
    fs: Fs, access: Asking, ctx: Context, answer: object
) -> None:
    def vague(asked: Access) -> Any:
        return answer

    fs.grants.add(ctx, "vague", vague)

    refusal = fs.judge(access("write", "a.txt"))

    assert refusal is not None
    assert refusal.code == "NOT_WRITABLE"


def test_a_grant_that_fails_allows_nothing_and_does_not_keep_others_from_allowing(
    fs: Fs, access: Asking, ctx: Context, caplog: pytest.LogCaptureFixture
) -> None:
    def broken(asked: Access) -> bool:
        raise RuntimeError("grant bug")

    fs.grants.add(ctx, "broken", broken)

    with caplog.at_level(logging.ERROR, logger="columbina.fs"):
        alone = fs.judge(access("write", "a.txt"))
    fs.grants.add(ctx, "always", allowing())

    assert alone is not None
    assert alone.code == "NOT_WRITABLE"
    assert "the grant 'broken' of file access failed" in caplog.text
    assert fs.judge(access("write", "a.txt")) is None


# Restrictions


def test_a_restriction_refuses_what_the_seam_would_allow(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    fs.restrictions.add(ctx, "closed", refusing("The archive is closed."))

    assert fs.judge(access("read", "a.txt")) == Refuse("The archive is closed.", "NOT_THIS")


def test_no_grant_undoes_the_refusal_of_a_restriction(fs: Fs, access: Asking, ctx: Context) -> None:
    asked: list[str] = []
    fs.grants.add(ctx, "always", allowing(asked))
    fs.restrictions.add(ctx, "closed", refusing("The archive is closed."))

    refusal = fs.judge(access("write", "a.txt"))

    assert refusal == Refuse("The archive is closed.", "NOT_THIS")
    assert asked == []


def test_restrictions_are_asked_in_the_order_they_were_added_until_one_refuses(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    asked: list[str] = []
    fs.restrictions.add(ctx, "first", silent(asked, "first"))
    fs.restrictions.add(ctx, "second", refusing("second says no", asked))
    fs.restrictions.add(ctx, "third", silent(asked, "third"))

    refusal = fs.judge(access("read", "a.txt"))

    assert asked == ["first", "second says no"]
    assert refusal == Refuse("second says no", "NOT_THIS")


def test_every_restriction_is_asked_about_an_access_that_goes_through(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    asked: list[str] = []
    fs.grants.add(ctx, "always", allowing(asked, "the grant"))
    fs.restrictions.add(ctx, "first", silent(asked, "first"))
    fs.restrictions.add(ctx, "second", silent(asked, "second"))

    assert fs.judge(access("write", "a.txt")) is None
    assert asked == ["first", "second", "the grant"]


@pytest.mark.parametrize("fault", ["raises", "allows", "answers something"])
def test_a_restriction_that_fails_refuses(
    fs: Fs, access: Asking, ctx: Context, caplog: pytest.LogCaptureFixture, fault: str
) -> None:
    def raising(asked: Access) -> Refuse | None:
        raise RuntimeError("restriction bug")

    def allowing_instead(asked: Access) -> Any:
        return True

    def chatting(asked: Access) -> Any:
        return "fine by me"

    faults = {"raises": raising, "allows": allowing_instead, "answers something": chatting}
    fs.restrictions.add(ctx, "broken", faults[fault])

    with caplog.at_level(logging.ERROR, logger="columbina.fs"):
        refusal = fs.judge(access("read", "a.txt"))

    assert refusal is not None
    assert refusal.code == "CHECK_FAILED"
    assert refusal.reason.startswith("The access could not be checked and did not happen")
    assert "the restriction 'broken' of file access" in caplog.text


def test_a_restriction_is_told_which_tool_asks_and_whether_the_file_was_named(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    seen: list[tuple[str, bool, str, str]] = []

    def watching(asked: Access) -> Refuse | None:
        seen.append((asked.tool, asked.named, asked.kind, asked.where.relative))
        return None

    fs.restrictions.add(ctx, "watching", watching)

    fs.judge(access("read", "a.txt", tool="grep", named=False))
    fs.judge(access("write", "./sub/../a.txt", tool="edit"))

    assert seen == [("grep", False, "read", "a.txt"), ("edit", True, "write", "a.txt")]


async def test_grants_and_restrictions_leave_with_the_plugin_that_added_them(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    def apply(inner: Context, config: object) -> None:
        seam = inner.use(FS)
        seam.grants.add(inner, "always", allowing())
        seam.restrictions.add(inner, "closed", refusing("closed"))

    fiber = await ctx.plugin(Plugin("policy", apply, inject=(FS,)))
    assert fs.judge(access("read", "a.txt")) is not None

    await fiber.dispose()

    assert list(fs.grants) == []
    assert list(fs.restrictions) == []
    assert fs.judge(access("read", "a.txt")) is None
    assert fs.judge(access("write", "a.txt")) is not None


def test_two_grants_cannot_have_one_name(fs: Fs, ctx: Context) -> None:
    fs.grants.add(ctx, "always", allowing())

    with pytest.raises(RegistryConflictError, match="the grant of file access 'always'"):
        fs.grants.add(ctx, "always", allowing())


# Permits


def test_an_access_that_may_happen_gets_a_permit(fs: Fs, access: Asking) -> None:
    asked = access("read", "a.txt")

    permit = fs.permit(asked)

    assert permit.access is asked


def test_an_access_that_is_refused_gets_none_and_says_why(
    fs: Fs, access: Asking, ctx: Context
) -> None:
    fs.restrictions.add(ctx, "closed", refusing("The archive is closed."))

    with pytest.raises(ToolFailure, match="The archive is closed") as refused:
        fs.permit(access("read", "a.txt"))

    assert refused.value.code == "NOT_THIS"


def test_a_file_is_opened_through_its_permit(fs: Fs, access: Asking, work: Path) -> None:
    permit = fs.permit(access("read", "a.txt"))

    with permit.open() as opened:
        content = opened.file.read()
        fingerprint = opened.fingerprint

    status = (work / "a.txt").stat()
    assert content == b"inside"
    assert fingerprint == Fingerprint(status.st_size, status.st_mtime_ns)
    assert opened.file.closed


def test_a_permit_to_write_does_not_open_for_reading(fs: Fs, access: Asking, ctx: Context) -> None:
    fs.grants.add(ctx, "always", allowing())
    permit = fs.permit(access("write", "a.txt"))

    with pytest.raises(ValueError, match="not for reading"):
        permit.open()


def test_a_file_that_is_gone_cannot_be_opened(fs: Fs, access: Asking) -> None:
    permit = fs.permit(access("read", "nowhere.txt"))

    with pytest.raises(FileNotFoundError):
        permit.open()


def test_a_directory_is_not_opened(fs: Fs, access: Asking, work: Path) -> None:
    (work / "sub").mkdir()
    permit = fs.permit(access("read", "sub"))

    with pytest.raises(ToolFailure, match="sub is not a file") as refused:
        permit.open()

    assert refused.value.code == "NOT_A_FILE"


def test_what_a_permit_opens_is_the_file_that_was_judged(
    fs: Fs, access: Asking, work: Path, tmp_path: Path
) -> None:
    (work / "d").mkdir()
    (work / "d" / "notes.txt").write_text("inside")
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    (outside / "notes.txt").write_text("a secret")
    permit = fs.permit(access("read", "d/notes.txt"))

    # Between the decision and the opening, the directory becomes a link that leads out.
    shutil.rmtree(work / "d")
    link_directory(work / "d", outside)

    with pytest.raises(ToolFailure, match="was not where it had been") as refused:
        permit.open()

    assert refused.value.code == "MOVED"


# What the model was shown


def test_a_file_that_the_model_was_shown_is_remembered_in_that_state(
    fs: Fs, access: Asking, session: Session, work: Path
) -> None:
    permit = fs.permit(access("read", "a.txt"))
    assert fs.seen(session, permit.access.where.path) is None

    fs.shown(permit, Fingerprint(6, 1000))

    assert fs.seen(session, permit.access.where.path) == Fingerprint(6, 1000)
    assert fs.seen(session, locate(work, "not-shown.txt").path) is None


def test_showing_a_file_again_replaces_what_was_remembered(
    fs: Fs, access: Asking, session: Session
) -> None:
    permit = fs.permit(access("read", "a.txt"))
    fs.shown(permit, Fingerprint(6, 1000))

    fs.shown(permit, Fingerprint(7, 2000))

    assert fs.seen(session, permit.access.where.path) == Fingerprint(7, 2000)


def test_what_one_session_was_shown_another_was_not(
    fs: Fs, access: Asking, session: Session, ctx: Context
) -> None:
    other = Session(ctx, "s-2", {kind.event_type: kind for kind in CORE_EVENTS})
    permit = fs.permit(access("read", "a.txt"))

    fs.shown(permit, Fingerprint(6, 1000))

    assert fs.seen(other, permit.access.where.path) is None


def test_the_seam_does_not_keep_a_session_alive(fs: Fs, ctx: Context, work: Path) -> None:
    passing = Session(ctx, "s-3", {kind.event_type: kind for kind in CORE_EVENTS})
    where = locate(work, "a.txt")
    fs.shown(Permit(Access(passing, work, where, "read", "probe"), fs), Fingerprint(6, 1000))
    watch = weakref.ref(passing)

    del passing
    gc.collect()

    assert watch() is None


if sys.platform == "win32":

    def test_the_case_of_a_path_does_not_make_it_another_file(
        fs: Fs, access: Asking, session: Session
    ) -> None:
        permit = fs.permit(access("read", "a.txt"))
        fs.shown(permit, Fingerprint(6, 1000))

        # However the path is written, it is found under the name that is on the disk.
        assert fs.seen(session, access("read", "A.TXT").where.path) == Fingerprint(6, 1000)
        assert fs.seen(session, access("read", "./sub/../a.TXT").where.path) == Fingerprint(6, 1000)


# What is on its way to the model


def test_what_a_call_has_read_counts_as_shown_when_the_next_step_begins(
    fs: Fs, access: Asking, session: Session
) -> None:
    permit = fs.permit(access("read", "a.txt"))
    where = permit.access.where.path

    fs.showing(permit, Fingerprint(6, 1000))

    assert fs.seen(session, where) is None

    session.append(StepEndEvent(turn=1, step=1))

    assert fs.seen(session, where) is None

    session.append(StepStartEvent(turn=1, step=2))

    assert fs.seen(session, where) == Fingerprint(6, 1000)


def test_a_step_of_one_session_delivers_nothing_to_another(
    fs: Fs, ctx: Context, access: Asking, session: Session
) -> None:
    other = Session(ctx, "s-2", {kind.event_type: kind for kind in CORE_EVENTS})
    permit = fs.permit(access("read", "a.txt"))
    fs.showing(permit, Fingerprint(6, 1000))

    other.append(StepStartEvent(turn=1, step=1))

    assert fs.seen(session, permit.access.where.path) is None


def test_whoever_calls_tools_without_a_loop_says_when_the_results_were_delivered(
    fs: Fs, access: Asking, session: Session
) -> None:
    permit = fs.permit(access("read", "a.txt"))
    fs.showing(permit, Fingerprint(6, 1000))

    fs.delivered(session)
    fs.delivered(session)

    assert fs.seen(session, permit.access.where.path) == Fingerprint(6, 1000)


def test_what_was_read_last_is_what_is_delivered(fs: Fs, access: Asking, session: Session) -> None:
    permit = fs.permit(access("read", "a.txt"))
    fs.shown(permit, Fingerprint(6, 1000))
    fs.showing(permit, Fingerprint(7, 2000))
    fs.showing(permit, Fingerprint(8, 3000))

    assert fs.seen(session, permit.access.where.path) == Fingerprint(6, 1000)

    fs.delivered(session)

    assert fs.seen(session, permit.access.where.path) == Fingerprint(8, 3000)


def test_what_the_agent_wrote_is_not_undone_by_an_older_state_that_was_on_its_way(
    fs: Fs, access: Asking, session: Session
) -> None:
    # A read and a write of one reply: the read saw the file as it was before the write.
    permit = fs.permit(access("read", "a.txt"))
    fs.showing(permit, Fingerprint(6, 1000))
    fs.shown(permit, Fingerprint(9, 5000))

    fs.delivered(session)

    assert fs.seen(session, permit.access.where.path) == Fingerprint(9, 5000)


def test_delivering_to_a_session_that_has_read_nothing_does_nothing(
    fs: Fs, session: Session, work: Path
) -> None:
    fs.delivered(session)

    assert fs.seen(session, locate(work, "a.txt").path) is None


# The working directory of a session


@pytest.fixture
def door(tmp_path: Path) -> Path:
    """A working directory that is a link to ``first``, and can be turned to ``second``."""
    for name in ("first", "second"):
        (tmp_path / name).mkdir()
        (tmp_path / name / "a.txt").write_text(name)
    link_directory(tmp_path / "door", tmp_path / "first")
    return tmp_path / "door"


def turn(door: Path) -> None:
    unlink_directory(door)
    link_directory(door, door.parent / "second")


def test_a_session_is_held_to_where_its_working_directory_was_at_first(
    fs: Fs, session: Session, door: Path
) -> None:
    where = fs.locate(session, door, "a.txt")
    assert where == locate(door, "a.txt")
    assert fs.judge(Access(session, door, where, "read", "probe")) is None

    turn(door)

    with pytest.raises(ToolFailure) as refused:
        fs.locate(session, door, "a.txt")
    assert refused.value.code == "WORKING_DIRECTORY_MOVED"
    for name in ("first", "second"):
        assert str(locate(door.parent / name, ".").path) in str(refused.value)


def test_a_place_that_was_found_without_the_seam_is_held_to_it_all_the_same(
    fs: Fs, session: Session, door: Path
) -> None:
    assert fs.judge(Access(session, door, locate(door, "a.txt"), "read", "probe")) is None

    turn(door)
    refusal = fs.judge(Access(session, door, locate(door, "a.txt"), "read", "probe"))

    assert refusal is not None
    assert refusal.code == "WORKING_DIRECTORY_MOVED"
    with pytest.raises(ToolFailure) as refused:
        fs.permit(Access(session, door, locate(door, "a.txt"), "read", "probe"))
    assert refused.value.code == "WORKING_DIRECTORY_MOVED"


def test_a_working_directory_that_is_turned_back_is_used_again(
    fs: Fs, session: Session, door: Path
) -> None:
    fs.locate(session, door, "a.txt")
    turn(door)
    unlink_directory(door)
    link_directory(door, door.parent / "first")

    assert fs.locate(session, door, "a.txt").inside


def test_another_session_starts_where_the_working_directory_is_by_then(
    fs: Fs, ctx: Context, session: Session, door: Path
) -> None:
    fs.locate(session, door, "a.txt")
    turn(door)
    later = Session(ctx, "s-2", {kind.event_type: kind for kind in CORE_EVENTS})

    where = fs.locate(later, door, "a.txt")

    assert where.path == locate(door.parent / "second", "a.txt").path
    assert fs.judge(Access(later, door, where, "read", "probe")) is None


def test_a_path_that_is_not_used_here_is_refused_before_anything_is_remembered(
    fs: Fs, session: Session, door: Path
) -> None:
    with pytest.raises(ToolFailure) as refused:
        fs.locate(session, door, "")
    assert refused.value.code == "INVALID_PATH"

    turn(door)

    assert fs.locate(session, door, "a.txt").inside


# The name that the working directory has where commands run


def named(fs: Fs, ctx: Context, name: str | None = "/workspace") -> None:
    """Have the working directory go by a name of its own in the place where commands run."""

    def there(session: Session) -> str | None:
        return name

    fs.names.add(ctx, "container", there)


def test_a_path_under_the_name_that_the_working_directory_has_elsewhere_is_the_file_here(
    fs: Fs, ctx: Context, session: Session, work: Path
) -> None:
    (work / "src").mkdir()
    (work / "src" / "a.py").write_text("")
    named(fs, ctx)

    where = fs.locate(session, work, "/workspace/src/a.py")

    here = locate(work, "src/a.py")
    assert (where.path, where.inside, where.relative, where.root) == (
        here.path,
        True,
        "src/a.py",
        here.root,
    )
    # What the model wrote is what it is told back.
    assert where.written == "/workspace/src/a.py"
    assert fs.here(session, "/workspace/src/a.py") == "src/a.py"


@pytest.mark.parametrize("written", ["/workspace", "/workspace/", "/workspace//"])
def test_the_name_alone_is_the_working_directory(
    fs: Fs, ctx: Context, session: Session, work: Path, written: str
) -> None:
    named(fs, ctx)

    where = fs.locate(session, work, written)

    assert (where.path, where.relative, where.written) == (locate(work, ".").path, ".", written)


def test_a_path_that_leaves_the_working_directory_under_that_name_is_outside_all_the_same(
    fs: Fs, ctx: Context, session: Session, work: Path
) -> None:
    named(fs, ctx)

    where = fs.locate(session, work, "/workspace/../outside.txt")

    assert not where.inside
    assert where.path == locate(work, "../outside.txt").path


@pytest.mark.parametrize("written", ["/workspaces/a.txt", "/workspace.txt", "workspace/a.txt"])
def test_a_path_that_only_looks_like_the_name_is_taken_as_it_is_written(
    fs: Fs, ctx: Context, session: Session, work: Path, written: str
) -> None:
    named(fs, ctx)

    assert fs.locate(session, work, written) == locate(work, written)
    assert fs.here(session, written) == written


def test_where_the_working_directory_has_no_other_name_a_path_is_what_it_always_was(
    fs: Fs, ctx: Context, session: Session, work: Path
) -> None:
    assert fs.locate(session, work, "/workspace/a.txt") == locate(work, "/workspace/a.txt")

    named(fs, ctx, None)

    assert fs.locate(session, work, "/workspace/a.txt") == locate(work, "/workspace/a.txt")


def test_a_path_under_that_name_is_held_to_where_the_working_directory_was_at_first(
    fs: Fs, ctx: Context, session: Session, door: Path
) -> None:
    named(fs, ctx)
    fs.locate(session, door, "/workspace/a.txt")

    turn(door)

    with pytest.raises(ToolFailure) as refused:
        fs.locate(session, door, "/workspace/a.txt")
    assert refused.value.code == "WORKING_DIRECTORY_MOVED"


def test_the_service_is_what_the_plugin_provides(fs: Fs) -> None:
    assert isinstance(fs, Fs)
    assert cast("object", fs.grants) is not cast("object", fs.restrictions)
