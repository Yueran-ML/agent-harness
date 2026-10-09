"""Walking a tree for a tool that searches: every entry is asked about, and links stay closed."""

import asyncio
import shutil
import threading
from pathlib import Path
from typing import Any

import pytest

import columbina.fs
from columbina.fs import FS, Access, Fs, Refuse, bound, locate
from columbina.fs.walk import Tally, walk
from columbina.kernel import Context
from columbina.session import CORE_EVENTS, Session
from support.links import link_directory


@pytest.fixture
async def fs(ctx: Context) -> Fs:
    await ctx.plugin(columbina.fs)
    return ctx.use(FS)


@pytest.fixture
def session(ctx: Context) -> Session:
    return Session(ctx, "s-1", {kind.event_type: kind for kind in CORE_EVENTS})


@pytest.fixture
def work(tmp_path: Path) -> Path:
    """A small tree: two files at the top, and two directories with files of their own."""
    directory = tmp_path / "work"
    for name in ("z.txt", "a.txt", "sub/b.txt", "sub/deep/c.txt", "lib/d.txt"):
        (directory / name).parent.mkdir(parents=True, exist_ok=True)
        (directory / name).write_text(name)
    return directory


async def found_in(
    fs: Fs,
    session: Session,
    work: Path,
    root: str = ".",
    *,
    limit: int = 1000,
    tool: str = "probe",
) -> tuple[list[str], Tally]:
    tally = Tally()
    where = locate(work, root)
    found = [
        entry.where.relative
        async for entry in walk(fs, session, work, where, tool, limit=limit, tally=tally)
    ]
    return found, tally


async def test_the_files_of_a_tree_are_handed_on_level_by_level(
    fs: Fs, session: Session, work: Path
) -> None:
    found, tally = await found_in(fs, session, work)

    assert found == ["a.txt", "z.txt", "lib/d.txt", "sub/b.txt", "sub/deep/c.txt"]
    assert tally == Tally(entries=8, refused=0, unreadable=0, cut_short=False)


async def test_a_file_is_handed_on_with_its_place_and_the_names_that_lead_to_it(
    fs: Fs, session: Session, work: Path
) -> None:
    tally = Tally()
    root = locate(work, "sub")

    found = [
        entry async for entry in walk(fs, session, work, root, "probe", limit=100, tally=tally)
    ]

    deep = found[1]
    status = (work / "sub" / "deep" / "c.txt").stat()
    assert [entry.below for entry in found] == [("b.txt",), ("deep", "c.txt")]
    assert deep.where == locate(work, "sub/deep/c.txt")
    assert (deep.size, deep.modified_ns) == (status.st_size, status.st_mtime_ns)


async def test_the_history_of_a_repository_is_passed_over(
    fs: Fs, session: Session, work: Path
) -> None:
    (work / ".git" / "objects").mkdir(parents=True)
    (work / ".git" / "config").write_text("[core]")
    (work / "sub" / ".git").mkdir()
    (work / "sub" / ".git" / "HEAD").write_text("ref")
    (work / ".gitignore").write_text("*.pyc")

    found, tally = await found_in(fs, session, work)

    assert found == [".gitignore", "a.txt", "z.txt", "lib/d.txt", "sub/b.txt", "sub/deep/c.txt"]
    assert tally.entries == 9


async def test_a_link_is_not_followed_wherever_it_leads(
    fs: Fs, session: Session, work: Path, tmp_path: Path
) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.txt").write_text("s")
    link_directory(work / "out", outside)
    link_directory(work / "again", work / "sub")

    found, tally = await found_in(fs, session, work)

    assert found == ["a.txt", "z.txt", "lib/d.txt", "sub/b.txt", "sub/deep/c.txt"]
    assert tally == Tally(entries=8)


async def test_the_seam_is_asked_about_every_entry_that_the_walk_comes_upon(
    fs: Fs, session: Session, work: Path, ctx: Context
) -> None:
    asked: list[tuple[str, str, str, bool]] = []

    def watching(access: Access) -> Refuse | None:
        asked.append((access.where.relative, access.kind, access.tool, access.named))
        return None

    fs.restrictions.add(ctx, "watching", watching)

    await found_in(fs, session, work, "sub", tool="grep")

    assert asked == [
        ("sub/b.txt", "read", "grep", False),
        ("sub/deep", "read", "grep", False),
        ("sub/deep/c.txt", "read", "grep", False),
    ]


async def test_what_the_seam_refuses_is_neither_entered_nor_handed_on(
    fs: Fs, session: Session, work: Path, ctx: Context
) -> None:
    def private(access: Access) -> Refuse | None:
        if access.where.relative in ("sub", "z.txt"):
            return Refuse("That is private.", "DENIED")
        return None

    fs.restrictions.add(ctx, "private", private)

    found, tally = await found_in(fs, session, work)

    assert found == ["a.txt", "lib/d.txt"]
    # What lies in a directory that was refused is not even looked at.
    assert tally == Tally(entries=5, refused=2)


async def test_a_walk_stops_when_it_has_looked_at_as_much_as_it_may(
    fs: Fs, session: Session, work: Path
) -> None:
    found, tally = await found_in(fs, session, work, limit=3)

    assert found == ["a.txt"]
    assert tally == Tally(entries=3, cut_short=True)


async def test_a_walk_that_looked_at_everything_was_not_cut_short(
    fs: Fs, session: Session, work: Path
) -> None:
    _, tally = await found_in(fs, session, work, limit=8)

    assert tally == Tally(entries=8, cut_short=False)


async def test_a_directory_that_cannot_be_listed_is_counted_and_passed(
    fs: Fs, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    really_list: Any = bound.listing

    def listing(path: Path) -> Any:
        if path.name == "sub":
            raise PermissionError(13, "Permission denied")
        return really_list(path)

    monkeypatch.setattr(bound, "listing", listing)

    found, tally = await found_in(fs, session, work)

    assert found == ["a.txt", "z.txt", "lib/d.txt"]
    assert tally == Tally(entries=5, unreadable=1)


async def test_a_directory_that_became_a_link_while_the_walk_was_under_way_is_not_listed(
    fs: Fs, session: Session, work: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (outside / "secret.txt").write_text("s")
    really_list: Any = bound.listing

    def listing(path: Path) -> Any:
        listed = really_list(path)
        if path.name == "work":
            # The walk has seen that sub is a directory. Now sub becomes a link.
            shutil.rmtree(work / "sub")
            link_directory(work / "sub", outside)
        return listed

    monkeypatch.setattr(bound, "listing", listing)

    found, tally = await found_in(fs, session, work)

    assert found == ["a.txt", "z.txt", "lib/d.txt"]
    assert tally.unreadable == 1


async def test_a_walk_that_is_cancelled_lists_no_further_directory(
    fs: Fs, session: Session, work: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    listed: list[str] = []
    began = threading.Event()
    go_on = threading.Event()
    really_list: Any = bound.listing

    def listing(path: Path) -> Any:
        listed.append(path.name)
        began.set()
        go_on.wait(timeout=10)
        return really_list(path)

    monkeypatch.setattr(bound, "listing", listing)
    walking = asyncio.ensure_future(found_in(fs, session, work))
    assert await asyncio.to_thread(began.wait, 10)

    walking.cancel()
    await asyncio.gather(walking, return_exceptions=True)
    go_on.set()
    await asyncio.sleep(0.1)

    assert walking.cancelled()
    assert listed == ["work"]


async def test_a_walk_outside_the_working_directory_finds_what_a_grant_lets_it_see(
    fs: Fs, session: Session, work: Path, tmp_path: Path, ctx: Context
) -> None:
    def anywhere(access: Access) -> bool:
        return True

    elsewhere = tmp_path / "elsewhere"
    (elsewhere / "docs").mkdir(parents=True)
    (elsewhere / "docs" / "n.txt").write_text("n")
    tally = Tally()
    assert [
        entry
        async for entry in walk(
            fs, session, work, locate(work, "../elsewhere"), "probe", limit=100, tally=Tally()
        )
    ] == []
    fs.grants.add(ctx, "anywhere", anywhere)
    root = locate(work, "../elsewhere")

    found = [
        entry async for entry in walk(fs, session, work, root, "probe", limit=100, tally=tally)
    ]

    assert [entry.below for entry in found] == [("docs", "n.txt")]
    assert not found[0].where.inside
    assert found[0].where.path == locate(work, "../elsewhere/docs/n.txt").path
