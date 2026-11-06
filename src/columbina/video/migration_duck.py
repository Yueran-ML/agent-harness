"""Offer the ducking omitted by legacy migration, without changing conversion itself."""

import asyncio
import json
import re
from pathlib import Path
from typing import Any

from columbina.session import lock
from columbina.video.duck import Duck
from columbina.video.engine import Engine
from columbina.video.migrate import discover
from columbina.video.project import ProjectError, written


def _read(path: Path) -> dict[str, Any] | None:
    # Receipts must not redirect writes outside the selected workspace through links.
    if path.resolve() != path.absolute():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def _receipts(workspace: Path, studio: Path) -> dict[str, list[tuple[Path, dict[str, Any]]]]:
    found: dict[str, list[tuple[Path, dict[str, Any]]]] = {}
    workspace = workspace.resolve()
    paths: list[tuple[Path, str | None]] = [
        (directory / "migrated.json", str(directory))
        for directory in discover(workspace, marked=True)
    ]
    paths.extend(
        (path, None)
        for path in (studio / "analysis/migrations").glob("*.json")
        if not path.name.startswith("pending-")
    )
    for path, source in paths:
        value = _read(path)
        if value is None or not re.fullmatch(r"t[0-9]+", str(value.get("timeline", ""))):
            continue
        if source is not None:
            value = {**value, "source": source}
        found.setdefault(value["timeline"], []).append((path, value))
    return found


def _has_duck(value: dict[str, Any]) -> bool:
    lines = value.get("dropped")
    return isinstance(lines, list) and any(
        isinstance(line, str) and line.startswith("duck:") for line in lines
    )


async def pending(workspace: Path, engine: Engine) -> list[dict[str, Any]]:
    """Return old, still-existing timelines whose duck question has no durable answer."""
    _, project = await engine.project()
    existing = {timeline["id"] for timeline in project["timelines"]}
    receipts = await asyncio.to_thread(_receipts, workspace, engine.directory)
    reports = []
    for timeline, copies in receipts.items():
        if timeline not in existing or any(
            value.get("duck_prompted") is True for _, value in copies
        ):
            continue
        value = next((value for _, value in copies if _has_duck(value)), None)
        if value is not None:
            reports.append(
                {**value, "source": value.get("source", ""), "existing": True, "duck_pending": True}
            )
    return reports


def _mark(studio: Path, timeline: str, copies: list[tuple[Path, dict[str, Any]]]) -> list[str]:
    value = {**copies[0][1], "duck_prompted": True}
    # Migration recovery rewrites its own receipts. This companion retains the answer across
    # that repair, including when a legacy directory is read-only or has been moved away.
    companion = studio / "analysis/migrations" / f"duck-{timeline}.json"
    if companion.resolve() != companion.absolute():
        raise ProjectError("The duck receipt must remain inside the Studio project.")
    written(companion, json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    warnings = []
    for path, old in copies:
        try:
            written(
                path,
                json.dumps({**old, "duck_prompted": True}, ensure_ascii=False, indent=2) + "\n",
            )
        except ProjectError as error:
            warnings.append(str(error))
    return warnings


async def answer(workspace: Path, engine: Engine, timeline: str, accept: bool) -> dict[str, Any]:
    """Explicit consent runs the existing deterministic Duck workflow; refusal edits no sound."""
    if not re.fullmatch(r"t[0-9]+", timeline) or type(accept) is not bool:
        raise ProjectError("A migration answer needs a timeline ID and a boolean choice.")
    held = lock.acquire(engine.directory / "migration.lock")
    if held is None:
        raise ProjectError("Another client is handling migration; try again later.")
    try:
        reports = await pending(workspace, engine)
        receipts = await asyncio.to_thread(_receipts, workspace, engine.directory)
        copies = receipts.get(timeline, [])
        if any(value.get("duck_prompted") is True for _, value in copies):
            return {"timeline": timeline, "duck_prompted": True, "status": "already_answered"}
        if not any(report["timeline"] == timeline for report in reports):
            raise ProjectError("This timeline has no unanswered legacy duck receipt.")
        result: dict[str, Any] = {"status": "declined"}
        if accept:
            _, project = await engine.project()
            selected = next((item for item in project["timelines"] if item["id"] == timeline), None)
            if selected is None:
                raise ProjectError("The migrated timeline no longer exists.")
            tracks = [track for track in selected["tracks"] if track["kind"] == "audio"]
            dialogue = [track for track in tracks if track.get("name") == "A1"]
            music = [
                track
                for track in tracks
                if re.fullmatch(r"A(?:[2-9]|[1-9][0-9]+)", track.get("name", ""))
            ]
            if len(dialogue) != 1 or not music:
                raise ProjectError(
                    "The migrated A1 dialogue and A2+ background tracks could not be identified."
                )
            result = await Duck(engine, engine.directory, Path).run(
                timeline=timeline,
                dialogue=dialogue[0]["id"],
                music=[track["id"] for track in music],
            )
            if result.get("status") != "applied":
                raise ProjectError(str(result.get("notice", "Ducking was not applied; try again.")))
        warnings = await asyncio.to_thread(_mark, engine.directory, timeline, copies)
        return {**result, "timeline": timeline, "duck_prompted": True, "warnings": warnings}
    finally:
        held.release()
