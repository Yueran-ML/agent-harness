"""Dialogue-guided ducking, measured on candidates before one project transaction."""

import math
from collections.abc import Callable, Sequence
from dataclasses import asdict
from pathlib import Path
from typing import Any

from columbina.video import words
from columbina.video.editing import _timeline  # pyright: ignore[reportPrivateUsage]
from columbina.video.edits import Rate
from columbina.video.engine import Engine, EngineError
from columbina.video.span import frames_showing, placed
from columbina.video.transcript import PAUSE_LEAST_US, Transcript


def invalid(message: str, reason: str = "invalid") -> EngineError:
    """An actionable audio-tool refusal, using the engine's usual error envelope."""
    return EngineError(message, code=-32602, data={"reason": reason})


def merged(ranges: Sequence[tuple[int, int]], gap: int = 0) -> list[tuple[int, int]]:
    """Union ordered intervals, optionally including short pauses between them."""
    result: list[tuple[int, int]] = []
    for start, end in sorted(ranges):
        if start >= end:
            continue
        if result and start <= result[-1][1] + gap:
            result[-1] = (result[-1][0], max(end, result[-1][1]))
        else:
            result.append((start, end))
    return result


def speech(
    track: dict[str, Any], rate: Rate, transcripts: dict[str, Transcript]
) -> tuple[float, list[tuple[int, int]]]:
    """Reuse Studio's source mapping, including trimmed, sped-up and reversed clips."""
    spoken: list[tuple[int, int]] = []
    coverage: list[tuple[int, int]] = []
    count = 0
    for raw in track.get("clips", []):
        if not raw.get("enabled", True):
            continue
        clip = placed(raw)
        if clip is None:
            continue
        coverage.append((clip.start, clip.end))
        transcript = transcripts.get(clip.media)
        if transcript is None:
            continue
        for word in transcript.words:
            if not any(character.isalnum() for character in word.text):
                continue
            first, end = frames_showing(clip, word.start_us, word.end_us, rate)
            if first < end:
                count += 1
                spoken.append((clip.start + first, clip.start + end))
    duration = sum(end - start for start, end in merged(coverage))
    # Short pauses within a phrase belong to one speaking interval, as in text editing.
    gap = math.floor(PAUSE_LEAST_US * rate.num / (1_000_000 * rate.den))
    return count / duration if duration else 0, merged(spoken, gap)


def quiet_ranges(
    intervals: Sequence[tuple[int, int]], end: int, rate: Rate
) -> list[tuple[int, int]]:
    """Exclude the 15 ms lookahead and the default 150 ms hold + 600 ms recovery."""
    before = math.ceil(0.015 * rate.num / rate.den)
    after = math.ceil(0.750 * rate.num / rate.den)
    covered = merged([(max(0, a - before), min(end, b + after)) for a, b in intervals])
    result: list[tuple[int, int]] = []
    at = 0
    for start, stop in covered:
        if at < start:
            result.append((at, start))
        at = max(at, stop)
    if at < end:
        result.append((at, end))
    return result


class Duck:
    """Set Track FX Ducker; keep all unrelated strip settings and existing automation."""

    def __init__(self, engine: Engine, directory: Path, check: Callable[[str], Path]) -> None:
        self.engine = engine
        self.directory = directory
        self.check = check

    async def run(
        self,
        *,
        music: Sequence[str],
        dialogue: str | None = None,
        timeline: str | None = None,
    ) -> dict[str, Any]:
        """Measure bounded Ducker proposals and commit the last one exactly once."""
        revision, project = await self.engine.project()
        session = self.engine.session
        selected = _timeline(project, timeline)
        rate = Rate.model_validate(selected["rate"])
        tracks = {
            track["id"]: track
            for track in selected["tracks"]
            if track["kind"] == "audio" and track.get("enabled", True)
        }
        transcripts = words.transcripts_of(self.directory, project)
        scores = {id: speech(track, rate, transcripts) for id, track in tracks.items()}
        if dialogue is None:
            highest = max((score for score, _ in scores.values()), default=0)
            choices = [id for id, (score, _) in scores.items() if score == highest and score > 0]
            if len(choices) != 1:
                return {
                    "status": "needs_dialogue",
                    "timeline": selected["id"],
                    "candidates": choices,
                    "notice": "Run video_transcribe first, or specify the dialogue track.",
                }
            dialogue = choices[0]
        if dialogue not in tracks:
            raise invalid("dialogue must name an enabled audio track", "missing")
        ids = list(dict.fromkeys(music))
        if not ids or dialogue in ids or any(id not in tracks for id in ids):
            raise invalid("music must name enabled background audio tracks, excluding dialogue")
        if any(tracks[id].get("locked") for id in ids):
            raise invalid("Unlock the selected background tracks before ducking.", "locked")
        for id in ids:
            owner = tracks[id]
            if owner.get("automation_arm") != "off" and any(
                path.startswith("track_fx.") and lane.get("points")
                for path, lane in owner.get("lanes", {}).items()
            ):
                raise invalid(
                    f"Track {id} has Ducker automation; turn its automation Off or edit that "
                    "lane explicitly before replacing Track FX settings.",
                    "automation",
                )
        used = {
            clip["source"]["media"]
            for track in tracks.values()
            for clip in track.get("clips", [])
            if clip["source"].get("kind") == "media"
        }
        for item in project["media"]:
            if item["id"] in used:
                self.check(item["path"])
        intervals = scores[dialogue][1]
        end = max(
            (
                clip["start"] + clip["length"]
                for track in tracks.values()
                for clip in track.get("clips", [])
            ),
            default=0,
        )
        quiet = quiet_ranges(intervals, end, rate) if intervals else []
        targets = [{"track": dialogue}, *({"track": id} for id in ids)]
        params: dict[str, Any] = {
            "timeline": selected["id"],
            "revision": revision,
            "targets": targets,
        }

        async def measure(
            frames: tuple[int, int] | None, ops: list[dict[str, Any]] | None
        ) -> list[dict[str, Any]]:
            request = {**params}
            if frames is not None:
                request["range"] = {"from": frames[0], "to": frames[1]}
            if ops is not None:
                request["candidate"] = ops
            result = await self.engine.analyze_sound("audio.analyze", request, session=session)
            rows = [
                next((row for row in result["results"] if row["target"] == target), None)
                for target in targets
            ]
            if any(row is None for row in rows):
                raise invalid("Analysis did not return every requested track.", "analysis")
            return [row for row in rows if row is not None]

        baseline = [await measure(frames, None) for frames in quiet]
        amounts = dict.fromkeys(ids, 12.0)

        def proposal() -> list[dict[str, Any]]:
            return [
                {
                    "op": "set_track_audio",
                    "timeline": selected["id"],
                    "track": id,
                    "patch": {
                        "strip": {
                            "track_fx": {
                                "on": True,
                                "sources": [{"track": dialogue}],
                                "amount_db": amount,
                                "recovery_ms": 600,
                                "lookahead_ms": 15,
                                "rise_ms": 10,
                                "hold_ms": 150,
                            }
                        }
                    },
                }
                for id, amount in amounts.items()
            ]

        checks: list[dict[str, Any]] = []
        adjustments = 0
        while True:
            ops = proposal()
            checks = []
            increase: set[str] = set()
            decrease: set[str] = set()
            for frames in intervals:
                measured = await measure(frames, ops)
                voice = measured[0].get("integrated")
                for id, row in zip(ids, measured[1:], strict=True):
                    background = row.get("integrated")
                    margin = (
                        voice - background if voice is not None and background is not None else None
                    )
                    met = margin >= 10 if margin is not None else None
                    checks.append(
                        {
                            "kind": "speech",
                            "track": id,
                            "range": list(frames),
                            "margin_lu": margin,
                            "met": met,
                        }
                    )
                    if met is False:
                        increase.add(id)
            for frames, old in zip(quiet, baseline, strict=True):
                measured = await measure(frames, ops)
                for id, before, after in zip(ids, old[1:], measured[1:], strict=True):
                    a, b = before.get("integrated"), after.get("integrated")
                    metric = "integrated"
                    if a is None or b is None:
                        # Short intervals still have a peak, but do not prove a loudness match.
                        a, b = before.get("sample_peak"), after.get("sample_peak")
                        metric = "sample_peak"
                    delta = b - a if a is not None and b is not None else None
                    met = abs(delta) <= 1 if delta is not None and metric == "integrated" else None
                    checks.append(
                        {
                            "kind": "quiet",
                            "track": id,
                            "range": list(frames),
                            "delta_db": delta,
                            "metric": metric,
                            "met": met,
                        }
                    )
                    if met is False and delta is not None and delta < -1:
                        decrease.add(id)
            if not intervals:
                # An explicitly chosen source may lack words. Validate/process the candidate,
                # but never infer speaking intervals or claim the 10 LU test passed.
                await measure(None, ops)
            if adjustments == 2 or not (increase or decrease):
                break
            for id in ids:
                if id in increase:
                    amounts[id] = min(30, amounts[id] + 3)
                elif id in decrease:
                    amounts[id] = max(0, amounts[id] - 3)
            adjustments += 1
        applied = await self.engine.apply(ops, revision=revision, session=session)
        per_track_complete = bool(intervals and quiet and checks) and all(
            row["met"] is not None for row in checks
        )
        # Individually quiet stems need not make a quiet sum (especially correlated copies).
        # Until their actual sum is measured, multi-track ducking is not globally verified.
        complete = per_track_complete and len(ids) == 1
        return {
            "status": "applied",
            "timeline": selected["id"],
            "dialogue": dialogue,
            "music": ids,
            "mode": "ducker",
            "amount_db": amounts,
            "adjustments": adjustments,
            "checks": checks,
            "met": False
            if any(row["met"] is False for row in checks)
            else (True if complete else None),
            "verification_complete": complete,
            "per_track_met": (
                all(row["met"] is True for row in checks) if per_track_complete else None
            ),
            "measurement_phase": "candidate_before_apply",
            "ops": ops,
            "applied": asdict(applied),
            "notice": "Margins are checked for each background track. Multiple backgrounds "
            "have not been verified as a combined mix. Quiet intervals exclude lookahead, "
            "hold and recovery. Null means the overall target could not be verified.",
        }
