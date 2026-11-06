"""Measure a proposed mix on a disposable engine snapshot, then commit it once."""

import copy
import math
from collections.abc import Callable, Sequence
from dataclasses import asdict
from pathlib import Path
from typing import Any, Literal

from columbina.video.editing import _timeline  # pyright: ignore[reportPrivateUsage]
from columbina.video.engine import Engine, EngineError

LOUDNESS_TOLERANCE = 0.3
PEAK_TOLERANCE = 0.2
SILENT_DB = -100.0


def _invalid(message: str, reason: str = "invalid") -> EngineError:
    return EngineError(message, code=-32602, data={"reason": reason})


def _values(owner: dict[str, Any]) -> list[float]:
    lane = owner.get("lanes", {}).get("fader_db")
    if owner.get("automation_arm") != "off" and lane and lane.get("points"):
        return [point["value"] for point in lane["points"]]
    return [owner.get("strip", owner).get("fader_db", 0.0)]


def _shift(
    timeline: str, target: dict[str, str], owner: dict[str, Any], delta: float
) -> tuple[list[dict[str, Any]], list[float], bool]:
    """Retain a lane's times, interpolation and keys, including when they are outside the view."""
    before = _values(owner)
    if delta == 0:
        return [], before, False
    # -100 dB and below is -inf (silence): a gain change leaves a silent point silent.
    values = [
        value if value <= SILENT_DB else min(10.0, max(SILENT_DB, value + delta))
        for value in before
    ]
    clamped = any(
        value != old + delta for value, old in zip(values, before, strict=True) if old > SILENT_DB
    )
    if values == before:
        return [], values, clamped
    lane = owner.get("lanes", {}).get("fader_db")
    if owner.get("automation_arm") != "off" and lane and lane.get("points"):
        moved = copy.deepcopy(lane)
        for point, value in zip(moved["points"], values, strict=True):
            point["value"] = value
        return (
            [
                {
                    "op": "set_lane",
                    "timeline": timeline,
                    "of": target,
                    "path": "fader_db",
                    "lane": moved,
                }
            ],
            values,
            clamped,
        )
    identity = (
        {"op": "set_track_audio", "track": target["track"]}
        if "track" in target
        else {"op": "set_bus", "id": target["bus"]}
    )
    return (
        [{**identity, "timeline": timeline, "patch": {"strip": {"fader_db": values[0]}}}],
        values,
        clamped,
    )


def _vca_maximum(timeline: dict[str, Any], owner: dict[str, Any]) -> float:
    """A conservative post-insert bound; shared ancestors count once, as the audio graph does."""
    remaining = list(owner.get("strip", {}).get("vcas", []))
    seen: set[str] = set()
    total = 0.0
    by_id = {vca["id"]: vca for vca in timeline.get("vcas", [])}
    while remaining:
        id = remaining.pop()
        if id in seen:
            continue
        seen.add(id)
        vca = by_id[id]
        total += max(_values(vca))
        remaining.extend(vca.get("vcas", []))
    return total


async def apply_mix(
    engine: Engine,
    check: Callable[[str], Path],
    *,
    target: Literal["track_fader", "main"],
    timeline: str | None,
    ids: Sequence[str],
    standard: str | None,
    level: float | None,
    mode: Literal["relative", "independent", "optimize"] | None,
) -> dict[str, Any]:
    """Apply a verified track/main fader proposal without transient edits to the open project."""
    if target == "main" and mode not in (None, "optimize"):
        raise _invalid("main apply uses mode optimize; omit mode for the default")
    if target == "track_fader" and (mode == "optimize" or not ids):
        raise _invalid("track_fader needs track ids and relative or independent mode")
    revision, project = await engine.project()
    session = engine.session
    selected = _timeline(project, timeline)
    standards = await engine.audio_standards()
    if standard is not None and not any(row["id"] == standard for row in standards):
        raise _invalid(
            "select a loudness standard from audio.standards; peak modes apply to clip_gain",
            "standard",
        )
    if target == "main":
        choices = [
            bus for bus in selected.get("buses", []) if not bus.get("strip", {}).get("outputs", [])
        ]
        if not ids and len(choices) == 1:
            ids = [choices[0]["id"]]
        if len(ids) != 1 or ids[0] not in {bus["id"] for bus in choices}:
            names = ", ".join(f"{bus['id']} ({bus['name']})" for bus in choices)
            raise _invalid(
                f"select one main output in ids: {names or 'no main outputs'}",
                "ambiguous" if choices else "missing",
            )
        owners = [next(bus for bus in choices if bus["id"] == ids[0])]
        targets = [{"bus": ids[0]}]
    else:
        by_id = {track["id"]: track for track in selected["tracks"] if track["kind"] == "audio"}
        if any(id not in by_id for id in ids):
            raise _invalid("track_fader ids must name existing audio tracks", "missing")
        owners = [by_id[id] for id in dict.fromkeys(ids)]
        if any(owner.get("locked") for owner in owners):
            raise _invalid(
                "unlock the selected audio tracks before changing their faders", "locked"
            )
        targets = [{"track": owner["id"]} for owner in owners]
    used = {
        clip["source"]["media"]
        for track in selected["tracks"]
        if track["kind"] == "audio"
        for clip in track.get("clips", [])
        if clip["source"].get("kind") == "media"
    }
    for item in project["media"]:
        if item["id"] in used:
            check(item["path"])
    params: dict[str, Any] = {"timeline": selected["id"], "revision": revision, "targets": targets}
    if standard is not None:
        params["standard"] = standard

    async def measure(ops: list[dict[str, Any]] | None = None) -> dict[str, Any]:
        return await engine.analyze_sound(
            "audio.analyze",
            {**params, **({"candidate": ops} if ops is not None else {})},
            session=session,
        )

    before = await measure()
    info = next((row for row in standards if row["id"] == before.get("standard")), None)
    wanted = level if level is not None else before.get("level")
    if wanted is None or not math.isfinite(wanted):
        raise _invalid(
            "analysis returned no finite loudness target; set level explicitly", "standard"
        )
    peak_limit: float | None = info.get("true_peak") if info is not None else None
    readings = [
        next((row for row in before.get("results", []) if row.get("target") == of), None)
        for of in targets
    ]
    if any(row is None for row in readings):
        raise _invalid("analysis did not return every requested target", "analysis")
    usable = [
        row["integrated"]
        for row in readings
        if row is not None and row.get("integrated") is not None
    ]
    deltas = [
        0.0
        if row is None or row.get("integrated") is None
        else wanted - (max(usable) if mode == "relative" else row["integrated"])
        for row in readings
    ]
    # The same tolerances define both success and whether another pass is needed.
    # Otherwise an already accepted result would still add history on every call;
    # this holds for every target (a fader track or the main output) alike.
    deltas = [0.0 if abs(delta) <= LOUDNESS_TOLERANCE else delta for delta in deltas]
    limiter: dict[str, Any] | None = None
    limiter_slot: int | None = None
    clamped: list[bool] = []
    adjustments = 0

    def proposal() -> list[dict[str, Any]]:
        ops: list[dict[str, Any]] = []
        clamped.clear()
        for owner, of, delta in zip(owners, targets, deltas, strict=True):
            changed, values, limited = _shift(selected["id"], of, owner, delta)
            ops.extend(changed)
            clamped.append(limited)
            if limiter is not None and peak_limit is not None:
                # Inserts precede the fader and VCAs. Compensate their largest gain; the
                # subsequent DSP measurement also sees EQ/Dynamics order, pan and automation.
                ceiling = peak_limit - max(values) - _vca_maximum(selected, owner)
                limiter["effect"]["ceiling_db"] = min(0.0, max(-24.0, ceiling))
                clamped[-1] |= limiter["effect"]["ceiling_db"] != ceiling
                if limiter_slot is None or limiter != owner["strip"]["inserts"][limiter_slot]:
                    ops.append(
                        {
                            "op": "add_insert" if limiter_slot is None else "set_insert",
                            "timeline": selected["id"],
                            "of": of,
                            **({"slot": limiter_slot} if limiter_slot is not None else {}),
                            "insert": copy.deepcopy(limiter),
                        }
                    )
        return ops

    ops = proposal()
    after = await measure(ops) if ops else before
    if target == "main" and usable:
        peak = after["results"][0].get("true_peak")
        if peak_limit is not None and peak is not None and peak > peak_limit + PEAK_TOLERANCE:
            strip = owners[0].get("strip", {})
            inserts = strip.get("inserts", [])
            if strip.get("inserts_on") is False:
                raise _invalid(
                    "enable Effects In on the main bus before adding its limiter; "
                    "existing bypassed effects are left unchanged"
                )
            active = [(slot, item) for slot, item in enumerate(inserts) if item.get("on", True)]
            if active and active[-1][1].get("effect", {}).get("kind") == "limiter":
                limiter_slot, existing_limiter = active[-1]
                if owners[0].get("automation_arm") != "off" and any(
                    path.startswith(f"insert.{existing_limiter['id']}.") and lane.get("points")
                    for path, lane in owners[0].get("lanes", {}).items()
                ):
                    raise _invalid(
                        "the main output limiter has automation; adjust its lane or set "
                        "automation Off before optimizing its static ceiling",
                        "automated_limiter",
                    )
                limiter = copy.deepcopy(existing_limiter)
            else:
                if len(inserts) >= 6:
                    raise _invalid(
                        "main already has six inserts; free a slot before loudness optimization",
                        "too_many_inserts",
                    )
                n = 1
                existing = {insert["id"] for insert in inserts}
                while f"fx{n}" in existing:
                    n += 1
                limiter = {
                    "id": f"fx{n}",
                    "on": True,
                    "effect": {"kind": "limiter", "ceiling_db": peak_limit},
                }
            ops = proposal()
            after = await measure(ops) if ops else before
        current = after["results"][0].get("integrated")
        if usable and current is not None and abs(current - wanted) > LOUDNESS_TOLERANCE:
            correction = wanted - current
            current_peak = after["results"][0].get("true_peak")
            if limiter is None and current_peak is not None and peak_limit is not None:
                correction = min(correction, peak_limit - current_peak)
            if abs(correction) > 1e-9:
                deltas[0] += correction
                adjustments = 1
                ops = proposal()
                after = await measure(ops) if ops else before
    results: list[dict[str, Any]] = []
    for of, old, delta, limited in zip(targets, readings, deltas, clamped, strict=True):
        row = next((row for row in after.get("results", []) if row.get("target") == of), None)
        if row is None:
            raise _invalid("candidate analysis did not return every requested target", "analysis")
        integrated, peak = row.get("integrated"), row.get("true_peak")
        expected = wanted
        if mode == "relative" and old is not None and old.get("integrated") is not None:
            expected -= max(usable) - old["integrated"]
        met = (
            integrated is not None
            and abs(integrated - expected) <= LOUDNESS_TOLERANCE
            and (peak_limit is None or (peak is not None and peak <= peak_limit + PEAK_TOLERANCE))
        )
        results.append(
            {
                "target": of,
                "before": old,
                "after": row,
                "gain_db": delta,
                "level": expected,
                "clamped": limited,
                "met": met,
            }
        )
    # No real change was made while analyzing, so cancellation/refusal never needs rollback.
    applied = await engine.apply(ops, revision=revision, session=session) if ops else None
    notice = (
        "Official published target."
        if info and info.get("official") and info.get("target") is not None
        else "Unofficial or project target; no official numeric target is claimed."
    )
    if level is not None:
        notice += " The requested level overrides the preset target."
    return {
        "status": "applied" if applied else "unchanged",
        "timeline": selected["id"],
        "session": session,
        "measured_revision": revision,
        "standard": before.get("standard"),
        "standard_info": info,
        "level": wanted,
        "true_peak_limit": peak_limit,
        "notice": notice,
        "measurement_phase": "candidate_before_apply",
        "results": results,
        "met": all(row["met"] for row in results),
        "gain_corrections": adjustments,
        "ops": ops,
        "applied": asdict(applied) if applied else None,
    }
