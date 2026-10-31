"""The wrappers of the Gallery's methods: what each sends, and what it reads of the answer."""

import base64
import copy
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from columbina.video.engine import UNEXPECTED, Engine, EngineError
from support import fake_engine
from support.fake_engine import Fake, RefusedError, served

FAKE = Path(fake_engine.__file__)


async def attached(directory: Path) -> Engine:
    return await Engine.attach(directory, program=str(FAKE))


def answering(fake: Fake, asked: list[tuple[str, dict[str, Any]]]) -> None:
    """Have the fake engine answer the Gallery's methods as the engine does, keeping what each
    was asked."""

    def method(name: str, result: Callable[[dict[str, Any]], dict[str, Any]]) -> None:
        def handler(_asking: object, params: dict[str, Any]) -> dict[str, Any]:
            asked.append((name, copy.deepcopy(params)))
            return result(params)

        setattr(fake, name.replace(".", "_"), handler)

    still = {
        "id": "s1",
        "album": "a1",
        "name": "1.2.1",
        "label": "",
        "image": {"file": "s1.png", "format": "png", "width": 64, "height": 36},
    }

    def grab(params: dict[str, Any]) -> dict[str, Any]:
        if fake.headless is not False:
            raise RefusedError(-32601, "there is no method still.grab in a headless engine")
        return still

    def applied(_params: dict[str, Any]) -> dict[str, Any]:
        done = [{"op": "set_grade", "grade": {"level": "clip", "timeline": "t1", "clip": "c2"}}]
        return {"revision": 4, "done": done, "next_id": 9, "names": {}, "clamped": []}

    method("still.grab", grab)
    method("still.get", lambda params: {**still, "grades": {"clip": None, "timeline": None}})
    method(
        "gallery.list",
        lambda params: {
            "albums": [{"id": "a1", "name": "Stills 1", "kind": "stills", "stills": [still]}]
        },
    )
    png = base64.b64encode(b"\x89PNG").decode()
    method("gallery.thumbnail", lambda params: {"width": 2, "height": 1, "png": png})
    method("still.apply", applied)
    method("grade.append_node", applied)
    method("still.label", lambda params: {"id": params["still"], "label": params["label"]})
    method("still.delete", lambda params: {"deleted": params["stills"]})
    method("still.move", lambda params: {"moved": {"s1": "ps3"}})
    method(
        "gallery.album.create",
        lambda params: {"id": "pa2", "name": "PowerGrade 2", "kind": params["kind"]},
    )
    method("gallery.album.rename", lambda params: {"id": params["album"], "name": params["name"]})
    method("gallery.album.delete", lambda params: {"deleted": ["s4"]})
    method("still.export", lambda params: {"written": [{"image": "a.png", "json": "a.json"}]})
    method("still.import", lambda params: {"added": [still]})


async def test_a_still_is_grabbed_listed_and_read(tmp_path: Path) -> None:
    asked: list[tuple[str, dict[str, Any]]] = []
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        answering(fake, asked)
        with pytest.raises(EngineError) as headless:
            await connected.grab_still()
        assert headless.value.code == -32601
        fake.headless = False
        grabbed = await connected.grab_still()
        assert asked[-1] == ("still.grab", {"view": "timeline"})
        assert grabbed["name"] == "1.2.1"
        await connected.grab_still(album="pa1", clip="c2", level="pre_clip", format="exr")
        assert asked[-1] == (
            "still.grab",
            {
                "view": "timeline",
                "album": "pa1",
                "clip": "c2",
                "level": "pre_clip",
                "format": "exr",
            },
        )
        albums = await connected.gallery()
        assert asked[-1] == ("gallery.list", {})
        assert albums[0]["stills"][0]["id"] == "s1"
        await connected.gallery("a1")
        assert asked[-1] == ("gallery.list", {"album": "a1"})
        held = await connected.still("s1")
        assert held["grades"] == {"clip": None, "timeline": None}
        assert await connected.still_thumbnail("s1", 30) == (2, 1, b"\x89PNG")
        assert asked[-1] == ("gallery.thumbnail", {"still": "s1", "height": 30})
        fake.results["gallery.list"] = {"albums": [None]}
        with pytest.raises(EngineError) as odd:
            await connected.gallery()
        assert odd.value.code == UNEXPECTED
        fake.results["gallery.thumbnail"] = {"width": 2, "height": 1, "png": "not base64!"}
        with pytest.raises(EngineError) as odd:
            await connected.still_thumbnail("s1")
        assert odd.value.code == UNEXPECTED


async def test_a_stills_grade_is_applied_and_a_node_appended_as_one_change(tmp_path: Path) -> None:
    asked: list[tuple[str, dict[str, Any]]] = []
    target = {"level": "clip", "timeline": "t1", "clip": "c2"}
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        answering(fake, asked)
        applied = await connected.apply_still("s1", [target])
        assert asked[-1] == (
            "still.apply",
            {"still": "s1", "to": [target], "mode": "apply", "keyframes": "none"},
        )
        assert applied.revision == 4
        assert applied.done[0]["op"] == "set_grade"
        await connected.apply_still(
            "s1",
            [target],
            mode="append",
            keyframes="start_frame",
            preserve_nodes=2,
            revision=3,
        )
        assert asked[-1] == (
            "still.apply",
            {
                "still": "s1",
                "to": [target],
                "mode": "append",
                "keyframes": "start_frame",
                "preserve_nodes": 2,
                "revision": 3,
            },
        )
        source = {"level": "clip", "timeline": "t1", "clip": "c1"}
        appended = await connected.append_node(source, "n4", [target])
        assert asked[-1] == (
            "grade.append_node",
            {"from": source, "node": "n4", "to": [target], "keyframes": "source_timecode"},
        )
        assert appended.next_id == 9
        fake.refusals["still.apply"] = {
            "code": -32001,
            "message": "the still s1 has no timeline grade",
            "data": {"reason": "no_grade", "still": "s1", "grade": "timeline"},
        }
        with pytest.raises(EngineError) as refused:
            await connected.apply_still("s1", [{"level": "timeline", "timeline": "t1"}])
        assert refused.value.code == -32001
        assert refused.value.data == {"reason": "no_grade", "still": "s1", "grade": "timeline"}


async def test_albums_and_stills_are_kept_by_the_wrappers(tmp_path: Path) -> None:
    asked: list[tuple[str, dict[str, Any]]] = []
    async with served(tmp_path / "film") as fake, await attached(tmp_path / "film") as connected:
        answering(fake, asked)
        await connected.label_still("s1", "Day exterior")
        assert asked[-1] == ("still.label", {"still": "s1", "label": "Day exterior"})
        assert await connected.delete_stills(["s1", "s2"]) == ["s1", "s2"]
        assert await connected.move_stills(["s1"], "pa1") == {"s1": "ps3"}
        assert asked[-1] == ("still.move", {"stills": ["s1"], "album": "pa1"})
        made = await connected.create_album("powergrade")
        assert made == {"id": "pa2", "name": "PowerGrade 2", "kind": "powergrade"}
        assert asked[-1] == ("gallery.album.create", {"kind": "powergrade"})
        await connected.create_album(name="Looks")
        assert asked[-1] == ("gallery.album.create", {"kind": "stills", "name": "Looks"})
        await connected.rename_album("a1", "Day")
        assert asked[-1] == ("gallery.album.rename", {"album": "a1", "name": "Day"})
        assert await connected.delete_album("a2") == ["s4"]
        written = await connected.export_stills(["s1"], tmp_path, prefix="Show", replace=True)
        assert written == [{"image": "a.png", "json": "a.json"}]
        assert asked[-1] == (
            "still.export",
            {"stills": ["s1"], "dir": str(tmp_path), "prefix": "Show", "replace": True},
        )
        added = await connected.import_stills([tmp_path / "a.png"], album="a1")
        assert added[0]["id"] == "s1"
        assert asked[-1] == ("still.import", {"paths": [str(tmp_path / "a.png")], "album": "a1"})
        fake.results["still.move"] = {"moved": {"s1": 3}}
        with pytest.raises(EngineError) as odd:
            await connected.move_stills(["s1"], "pa1")
        assert odd.value.code == UNEXPECTED
        fake.results["still.delete"] = {"deleted": "s1"}
        with pytest.raises(EngineError) as odd:
            await connected.delete_stills(["s1"])
        assert odd.value.code == UNEXPECTED
