"""Dialogue-guided ducking, measured on candidates before one project transaction."""

import math
from collections.abc import Callable, Sequence
from dataclasses import asdict
from pathlib import Path
from typing import Any, cast

from columbina.video import words
from columbina.video.duck_keys import UnsupportedCurveError, gain_keys
from columbina.video.editing import _timeline  # pyright: ignore[reportPrivateUsage]
from columbina.video.edits import Rate
from columbina.video.engine import Engine, EngineError
from columbina.video.span import placed, runs_showing
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
    """Reuse Studio's source mapping, including trimmed, sped-up and reversed clips.

    A clip with speed points says a word each time that its path comes by it, and nothing
    through a freeze.
    """
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
            runs = runs_showing(clip, word.start_us, word.end_us, rate, heard=True)
            if runs:
                count += 1
            spoken.extend((clip.start + first, clip.start + end) for first, end in runs)
    duration = sum(end - start for start, end in merged(coverage))
    # Short pauses within a phrase belong to one speaking interval, as in text editing.
    gap = math.floor(PAUSE_LEAST_US * rate.num / (1_000_000 * rate.den))
    return count / duration if duration else 0, merged(spoken, gap)


def quiet_ranges(
    intervals: Sequence[tuple[int, int]], end: int, rate: Rate, *, hold_ms: int = 150
) -> list[tuple[int, int]]:
    """Exclude 15 ms lookahead, 600 ms recovery and the selected mode's hold."""
    before = math.ceil(0.015 * rate.num / rate.den)
    after = math.ceil((0.600 + hold_ms / 1000) * rate.num / rate.den)
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


def paired_levels(a: dict[str, Any], b: dict[str, Any]) -> tuple[float | None, float | None, str]:
    """Compare like with like; the fallback is K-weighted loudness, never peak or RMS."""
    for metric in ("integrated", "k_weighted_mean"):
        first, second = a.get(metric), b.get(metric)
        if first is not None and second is not None:
            return first, second, metric
    return None, None, "unavailable"


class Duck:
    """Measure Ducker or source Volume envelopes, preserving unrelated settings."""

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
        mode: str = "ducker",
    ) -> dict[str, Any]:
        """Measure bounded duck proposals and commit the best-scoring one exactly once."""
        if mode not in ("ducker", "keyframes"):
            raise invalid("mode must be 'ducker' or 'keyframes'")
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
            if (
                mode == "ducker"
                and owner.get("automation_arm") != "off"
                and any(
                    path.startswith("track_fx.") and lane.get("points")
                    for path, lane in owner.get("lanes", {}).items()
                )
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
        if mode == "keyframes" and not intervals:
            return {
                "status": "needs_transcript",
                "timeline": selected["id"],
                "notice": "Run video_transcribe on the dialogue before keyframe ducking.",
            }
        end = max(
            (
                clip["start"] + clip["length"]
                for track in tracks.values()
                for clip in track.get("clips", [])
            ),
            default=0,
        )
        quiet = (
            quiet_ranges(intervals, end, rate, hold_ms=150 if mode == "ducker" else 0)
            if intervals
            else []
        )
        # Dialogue and every background use the same stereo post-pan measurement point.
        # Track targets retain their protocol meaning (pre-pan) for other clients.
        targets: list[dict[str, Any]] = [{"sum": [dialogue]}, *({"sum": [id]} for id in ids)]
        check_targets: list[dict[str, Any]] = [{"track": id} for id in ids]
        if len(ids) > 1:
            targets.append({"sum": ids})
            check_targets.append({"sum": ids})
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
            found: list[dict[str, Any]] = []
            # 255 backgrounds plus dialogue plus sum exceed one job's 256-target limit.
            # Every chunk remains bound to the same project revision and session.
            for start in range(0, len(targets), 256):
                result = await self.engine.analyze_sound(
                    "audio.analyze",
                    {**request, "targets": targets[start : start + 256]},
                    session=session,
                )
                found.extend(result["results"])
            rows = [
                next((row for row in found if row["target"] == target), None) for target in targets
            ]
            if any(row is None for row in rows):
                raise invalid("Analysis did not return every requested target.", "analysis")
            return [row for row in rows if row is not None]

        amounts = dict.fromkeys(ids, 12.0)

        def proposal() -> list[dict[str, Any]]:
            if mode == "keyframes":
                ops: list[dict[str, Any]] = []
                for id, amount in amounts.items():
                    for clip in tracks[id].get("clips", []):
                        try:
                            gain = gain_keys(
                                clip,
                                intervals,
                                selected["rate"],
                                selected.get("sample_rate", 48000),
                                amount,
                            )
                        except UnsupportedCurveError as error:
                            raise invalid(str(error), "unsupported_curve") from error
                        if gain is not None:
                            ops.append(
                                {
                                    "op": "set_clip",
                                    "timeline": selected["id"],
                                    "id": clip["id"],
                                    "gain_db": gain,
                                }
                            )
                return ops
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

        ops = proposal()
        if not ops:
            return {
                "status": "no_change",
                "timeline": selected["id"],
                "mode": mode,
                "notice": "No enabled background media clip overlaps the duck envelope.",
            }
        baseline = [await measure(frames, None) for frames in quiet]

        def overall(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
            """Rows that decide met: the real PCM sum for several backgrounds, else the track."""
            if len(ids) > 1:
                return [row for row in rows if "sum" in row]
            return [row for row in rows if "track" in row]

        def score(rows: list[dict[str, Any]]) -> tuple[int, float]:
            """Lower is better: failed overall checks, then their total shortfall in dB/LU.

            A speech check falls short by 10 - margin; a quiet check by |delta| - 1. A
            silence change has no finite size and counts only as a failure.
            """
            failed = [row for row in overall(rows) if row["met"] is False]
            shortfall = 0.0
            for row in failed:
                if row["kind"] == "speech" and row["margin_lu"] is not None:
                    shortfall += max(0.0, 10 - row["margin_lu"])
                elif row["kind"] == "quiet" and row["delta_db"] is not None:
                    shortfall += max(0.0, abs(row["delta_db"]) - 1)
            return len(failed), shortfall

        checks: list[dict[str, Any]] = []
        adjustments = 0
        tried: list[dict[str, Any]] = []
        best: dict[str, Any] | None = None
        while True:
            ops = proposal()
            checks = []
            increase: set[str] = set()
            decrease: set[str] = set()
            for frames in intervals:
                measured = await measure(frames, ops)
                for target, row in zip(check_targets, measured[1:], strict=True):
                    voice, background, metric = paired_levels(measured[0], row)
                    margin = (
                        voice - background if voice is not None and background is not None else None
                    )
                    met = margin >= 10 if margin is not None else None
                    if row.get("silence") is True and any(
                        measured[0].get(field) is not None
                        for field in ("integrated", "k_weighted_mean")
                    ):
                        # A finite voice is above a measured zero background, but no finite
                        # number represents that difference. A silent voice remains unknown.
                        met, metric = True, "silent_background"
                    checks.append(
                        {
                            "kind": "speech",
                            **target,
                            "range": list(frames),
                            "margin_lu": margin,
                            "metric": metric,
                            "met": met,
                        }
                    )
                    if met is False and (len(ids) == 1 or "sum" in target):
                        increase.update(ids if "sum" in target else [target["track"]])
            for frames, old in zip(quiet, baseline, strict=True):
                measured = await measure(frames, ops)
                for target, before, after in zip(check_targets, old[1:], measured[1:], strict=True):
                    a, b, metric = paired_levels(before, after)
                    delta = b - a if a is not None and b is not None else None
                    met = abs(delta) <= 1 if delta is not None else None
                    if before.get("silence") is True and after.get("silence") is True:
                        met, metric = True, "silence"
                    elif (
                        before.get("silence") is True
                        and any(
                            after.get(field) is not None
                            for field in ("integrated", "k_weighted_mean")
                        )
                    ) or (
                        after.get("silence") is True
                        and any(
                            before.get(field) is not None
                            for field in ("integrated", "k_weighted_mean")
                        )
                    ):
                        met, metric = False, "silence_changed"
                    checks.append(
                        {
                            "kind": "quiet",
                            **target,
                            "range": list(frames),
                            "delta_db": delta,
                            "metric": metric,
                            "met": met,
                        }
                    )
                    if (
                        met is False
                        and delta is not None
                        and delta < -1
                        and (len(ids) == 1 or "sum" in target)
                    ):
                        decrease.update(ids if "sum" in target else [target["track"]])
            if not intervals:
                # An explicitly chosen source may lack words. Validate/process the candidate,
                # but never infer speaking intervals or claim the 10 LU test passed.
                await measure(None, ops)
            candidate = {
                "amount_db": dict(amounts),
                "score": score(checks),
                "ops": ops,
                "checks": checks,
            }
            tried.append(candidate)
            # Ties keep the earlier, smaller adjustment.
            if best is None or candidate["score"] < best["score"]:
                best = candidate
            if adjustments == 2 or not (increase or decrease):
                break
            for id in ids:
                if id in increase:
                    amounts[id] = min(30, amounts[id] + 3)
                elif id in decrease:
                    amounts[id] = max(0, amounts[id] - 3)
            if any(row["amount_db"] == amounts for row in tried):
                # Up then down (or a bound) returns to a measured candidate: nothing new to learn.
                break
            adjustments += 1
        assert best is not None
        ops, checks, amounts = best["ops"], best["checks"], best["amount_db"]
        clips = {clip["id"]: clip for track in tracks.values() for clip in track.get("clips", [])}

        def unchanged(op: dict[str, Any]) -> bool:
            """Whether the merge-patch or Volume replacement leaves the project as it is."""
            if op["op"] == "set_track_audio":
                current = tracks[op["track"]].get("strip", {}).get("track_fx")
                if not isinstance(current, dict):
                    return False
                fx = cast(dict[str, Any], current)
                wanted: dict[str, Any] = op["patch"]["strip"]["track_fx"]
                return all(fx.get(key) == value for key, value in wanted.items())
            if op["op"] == "set_clip":
                return clips[op["id"]].get("gain_db", 0.0) == op["gain_db"]
            return False

        # A repeat with the same parameters must not add an empty step to undo history.
        changed = not all(unchanged(op) for op in ops)
        applied = (
            await self.engine.apply(ops, revision=revision, session=session) if changed else None
        )
        track_checks = [row for row in checks if "track" in row]
        overall_checks = overall(checks)
        per_track_complete = bool(intervals and quiet and track_checks) and all(
            row["met"] is not None for row in track_checks
        )
        complete = bool(intervals and quiet and overall_checks) and all(
            row["met"] is not None for row in overall_checks
        )
        return {
            "status": "applied" if changed else "unchanged",
            "changed": changed,
            "timeline": selected["id"],
            "dialogue": dialogue,
            "music": ids,
            "mode": mode,
            "amount_db": amounts,
            "adjustments": adjustments,
            "candidates": [
                {
                    "amount_db": row["amount_db"],
                    "failed": row["score"][0],
                    "shortfall_db": row["score"][1],
                }
                for row in tried
            ],
            "checks": checks,
            "met": False
            if any(row["met"] is False for row in overall_checks)
            else (True if complete else None),
            "verification_complete": complete,
            "per_track_met": (
                all(row["met"] is True for row in track_checks) if per_track_complete else None
            ),
            "measurement_phase": "candidate_before_apply",
            "measurement_point": "stereo_post_pan",
            "ops": ops,
            "applied": asdict(applied) if applied is not None else None,
            "notice": "Dialogue and backgrounds are measured at the same stereo post-strip, "
            "post-pan point. Multiple backgrounds are verified by their actual PCM sum; "
            "per-track results remain diagnostic. Quiet intervals exclude "
            "lookahead and recovery (plus hold in Ducker mode). Null means the overall target "
            "could not be verified. Pairs without Integrated use the same ungated K-weighted "
            "mean on both sides, explicitly reported in metric; intervals shorter than one "
            "10 ms cell remain unverified. Measured silence is reported separately. "
            "Keyframe envelopes hold across overlapping "
            "recovery/lookahead windows.",
        }
