"""Loudness measurements and Normalize Audio Levels through the shared engine."""

from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any, Literal

from columbina.video.editing import _timeline  # pyright: ignore[reportPrivateUsage]
from columbina.video.engine import Engine, EngineError


def _invalid(message: str, reason: str = "invalid") -> EngineError:
    return EngineError(message, code=-32602, data={"reason": reason})


class Loudness:
    """Select sound to measure; let the engine apply clip gains as one undo step."""

    def __init__(self, engine: Engine, check: Callable[[str], Path]) -> None:
        self.engine = engine
        self._check = check

    async def run(
        self,
        *,
        action: Literal["measure", "apply"],
        target: Literal["clip", "track", "bus", "main", "clip_gain", "track_fader"],
        timeline: str | None = None,
        ids: Sequence[str] = (),
        frames: tuple[int, int] | None = None,
        standard: str | None = None,
        level: float | None = None,
        mode: Literal["relative", "independent", "optimize"] | None = None,
    ) -> dict[str, Any]:
        """Wait for the analysis, retaining null readings, clamping and apply refusals.

        Ranges are timeline frames, intersected with clips when measuring them. Clip normalization
        moves the existing Volume curve; it never substitutes a fixed gain for its keys.
        """
        if action == "apply" and (target == "track_fader" or target == "main"):
            if frames is not None:
                raise _invalid(
                    "fader apply measures whole targets; use measure for a selected range"
                )
            from columbina.video.mix_loudness import apply_mix

            return await apply_mix(
                self.engine,
                self._check,
                target=target,
                timeline=timeline,
                ids=ids,
                standard=standard,
                level=level,
                mode=mode,
            )
        if action == "measure":
            if target in {"clip_gain", "track_fader"} or mode is not None or level is not None:
                raise _invalid(
                    "measure takes clip, track, bus or main; mode and level are for apply"
                )
        elif target != "clip_gain" or frames is not None:
            raise _invalid("apply supports clip_gain only, over whole clips without a range")
        elif mode == "optimize":
            raise _invalid("clip_gain uses relative or independent mode; optimize is for main")
        if frames is not None and not 0 <= frames[0] < frames[1]:
            raise _invalid("range must have 0 <= from < to in timeline frames", "out_of_range")
        if target != "main" and not ids:
            raise _invalid("name at least one id; video_info lists clips, tracks and buses")
        revision, project = await self.engine.project()
        session = self.engine.session
        standards = await self.engine.audio_standards()
        if (
            standard is not None
            and not any(row["id"] == standard for row in standards)
            and (action != "apply" or standard not in {"sample_peak", "true_peak"})
        ):
            raise _invalid(
                f"unknown standard {standard}; available: "
                + ", ".join(row["id"] for row in standards),
                "standard",
            )
        selected = _timeline(project, timeline)
        targets = [{target: id} for id in ids]
        if target == "main":
            mains = [
                bus
                for bus in selected.get("buses", [])
                if not bus.get("strip", {}).get("outputs", [])
            ]
            if not ids and len(mains) == 1:
                ids = [mains[0]["id"]]
            if not ids:
                choices = ", ".join(f"{bus['id']} ({bus['name']})" for bus in mains)
                raise _invalid(
                    f"select one main output in ids: {choices or 'no main outputs'}",
                    "ambiguous" if mains else "missing",
                )
            if len(ids) != 1 or ids[0] not in {bus["id"] for bus in mains}:
                raise _invalid("ids must select one main output (a bus with no outputs)", "missing")
            targets = [{"bus": ids[0]}]
        used = {
            clip["source"]["media"]
            for track in selected["tracks"]
            if track["kind"] == "audio"
            for clip in track.get("clips", [])
            if clip["source"].get("kind") == "media"
            and (target not in {"clip", "clip_gain"} or clip["id"] in ids)
        }
        for item in project["media"]:
            if item["id"] in used:
                self._check(item["path"])
        params: dict[str, Any] = {"timeline": selected["id"], "revision": revision}
        if action == "measure":
            params["targets"] = targets
            if frames is not None:
                params["range"] = {"from": frames[0], "to": frames[1]}
            if standard is not None:
                params["standard"] = standard
            done = await self.engine.analyze_sound("audio.analyze", params, session=session)
            chosen = done.get("standard")
            status = "measured"
        else:
            params.update(clips=list(ids), mode=mode or "relative", apply=True)
            if standard is not None:
                params["level"] = standard
            if level is not None:
                params["target"] = level
            done = await self.engine.analyze_sound("audio.normalize", params, session=session)
            chosen = done.get("level", standard)
            status = (
                "refused"
                if done.get("refused") is not None
                else ("applied" if done.get("applied") is not None else "unchanged")
            )
        info = next((row for row in standards if row["id"] == chosen), None)
        said = {
            **done,
            "status": status,
            "timeline": selected["id"],
            "measured_revision": revision,
            "session": session,
            "standard_info": info,
        }
        if info is not None:
            if info["target"] is None:
                said["notice"] = "This standard specifies no target; the timeline target is used."
            else:
                said["notice"] = (
                    "Official published target."
                    if info["official"]
                    else "Unofficial preset: no official target numbers for this preset."
                )
            if level is not None:
                said["notice"] += " The requested level overrides the preset target."
        return said
