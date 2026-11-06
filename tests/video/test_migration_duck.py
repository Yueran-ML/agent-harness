"""Legacy duck consent is durable, timeline-scoped, deterministic and retryable."""

import json
from pathlib import Path
from typing import Any
from unittest.mock import AsyncMock

import pytest

from columbina.video.engine import Applied, Engine
from columbina.video.migration_duck import answer, pending
from columbina.video.project import ProjectError


def fixture(workspace: Path) -> tuple[AsyncMock, Path, Path]:
    studio = workspace / ".columbina/studio"
    receipts = studio / "analysis/migrations"
    receipts.mkdir(parents=True)
    old = workspace / "Old Film"
    old.mkdir()
    (old / "project.json").write_text('{"version":1,"canvas":{},"assets":[]}', encoding="utf-8")
    (old / "timeline.json").write_text("{}", encoding="utf-8")
    value: dict[str, Any] = {
        "timeline": "t1",
        "at": "old",
        "media": {},
        "dropped": ["duck: omitted"],
    }
    marker = old / "migrated.json"
    marker.write_text(json.dumps(value), encoding="utf-8")
    receipt = receipts / "t1.json"
    receipt.write_text(json.dumps({**value, "source": str(old)}), encoding="utf-8")
    engine = AsyncMock(spec=Engine)
    engine.directory = studio
    engine.session = 1
    engine.project.return_value = (
        7,
        {
            "media": [],
            "timelines": [
                {
                    "id": "t1",
                    "rate": {"num": 25, "den": 1},
                    "tracks": [
                        {
                            "id": f"r{i}",
                            "name": f"A{i}",
                            "kind": "audio",
                            "clips": [],
                            "enabled": True,
                            "locked": False,
                        }
                        for i in range(1, 4)
                    ],
                }
            ],
        },
    )
    engine.apply.return_value = Applied(8, [], 1)
    engine.analyze_sound.return_value = {
        "results": [
            {"target": {"sum": tracks}, "integrated": -18, "sample_peak": -10}
            for tracks in (["r1"], ["r2"], ["r3"], ["r2", "r3"])
        ]
    }
    return engine, marker, receipt


async def test_decline_marks_both_receipts_and_survives_migration_repair(tmp_path: Path) -> None:
    engine, marker, receipt = fixture(tmp_path)
    assert len(await pending(tmp_path, engine)) == 1
    assert (await answer(tmp_path, engine, "t1", False))["status"] == "declined"
    assert json.loads(marker.read_text())["duck_prompted"] is True
    assert json.loads(receipt.read_text())["duck_prompted"] is True
    engine.apply.assert_not_awaited()
    engine.analyze_sound.assert_not_awaited()
    # Existing migration recovery copies only its original fields and deletes its backup.
    value = json.loads(marker.read_text())
    value.pop("duck_prompted")
    marker.write_text(json.dumps(value))
    receipt.unlink()
    assert await pending(tmp_path, engine) == []
    assert (await answer(tmp_path, engine, "t1", True))["status"] == "already_answered"
    engine.apply.assert_not_awaited()


async def test_accept_reuses_duck_with_a1_and_all_backgrounds_once(tmp_path: Path) -> None:
    engine, marker, _ = fixture(tmp_path)
    result = await answer(tmp_path, engine, "t1", True)
    assert result["mode"] == "ducker"
    assert result["dialogue"] == "r1"
    assert result["music"] == ["r2", "r3"]
    engine.apply.assert_awaited_once()
    operations = engine.apply.call_args.args[0]
    assert [op["track"] for op in operations] == ["r2", "r3"]
    assert all(
        op["patch"]["strip"]["track_fx"]["sources"] == [{"track": "r1"}] for op in operations
    )
    assert json.loads(marker.read_text())["duck_prompted"] is True
    assert await pending(tmp_path, engine) == []


async def test_failure_is_retryable_and_neither_receipt_is_marked(tmp_path: Path) -> None:
    engine, marker, receipt = fixture(tmp_path)
    engine.apply.side_effect = ProjectError("stale project")
    with pytest.raises(ProjectError, match="stale project"):
        await answer(tmp_path, engine, "t1", True)
    assert "duck_prompted" not in json.loads(marker.read_text())
    assert "duck_prompted" not in json.loads(receipt.read_text())
    assert len(await pending(tmp_path, engine)) == 1
    engine.apply.side_effect = None
    assert (await answer(tmp_path, engine, "t1", True))["duck_prompted"] is True


async def test_filters_no_duck_deleted_timeline_and_invalid_answer(tmp_path: Path) -> None:
    engine, marker, receipt = fixture(tmp_path)
    for path in (marker, receipt):
        value = json.loads(path.read_text())
        value["dropped"] = ["other: dropped", "not duck: omitted"]
        path.write_text(json.dumps(value))
    assert await pending(tmp_path, engine) == []
    with pytest.raises(ProjectError, match="no unanswered"):
        await answer(tmp_path, engine, "t1", True)
    with pytest.raises(ProjectError, match="timeline ID"):
        await answer(tmp_path, engine, "../t1", False)
    value["dropped"] = ["duck: omitted"]
    receipt.write_text(json.dumps(value))
    engine.project.return_value = (7, {"timelines": []})
    assert await pending(tmp_path, engine) == []


async def test_studio_backup_works_without_legacy_source_and_missing_a1_is_not_guessed(
    tmp_path: Path,
) -> None:
    engine, marker, _ = fixture(tmp_path)
    marker.unlink()
    engine.project.return_value[1]["timelines"][0]["tracks"][0]["name"] = "Renamed voice"
    assert len(await pending(tmp_path, engine)) == 1
    with pytest.raises(ProjectError, match="could not be identified"):
        await answer(tmp_path, engine, "t1", True)
    assert len(await pending(tmp_path, engine)) == 1
    assert (await answer(tmp_path, engine, "t1", False))["duck_prompted"] is True
