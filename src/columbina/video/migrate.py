"""Read legacy films into Studio timelines, leaving their source directories intact.

Media are inspected first; their add operations and the entire timeline are committed in one
transaction. The only file written in a legacy directory is its migrated.json receipt.
"""

import asyncio
import json
import math
import os
import re
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from datetime import UTC, datetime
from fractions import Fraction
from pathlib import Path
from typing import Any, Literal, cast

from columbina.session import lock
from columbina.video import edits, media, render, story, words
from columbina.video.engine import Engine, EngineError
from columbina.video.project import Asset, Canvas, Manifest, ProjectError, written
from columbina.video.timeline import Layer, Timeline, Transcript

__all__ = ["discover", "migrate"]

_EXCLUDED = {".git", "node_modules", ".columbina"}
Check = Callable[[Path, Literal["read", "write"]], object]


def discover(workspace: Path) -> list[Path]:
    """Find unconverted schema-1 project directories, at most three levels down."""
    found: list[Path] = []
    pending = [(workspace.resolve(), 0)]
    while pending:
        directory, depth = pending.pop()
        manifest = directory / "project.json"
        if (
            manifest.is_file()
            and (directory / "timeline.json").is_file()
            and not (directory / "migrated.json").exists()
        ):
            try:
                value = json.loads(manifest.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                value = None
            if (
                isinstance(value, dict)
                and cast("dict[str, Any]", value).get("version") == 1
                and {"canvas", "assets"} <= value.keys()
            ):
                found.append(directory)
        if depth == 3:
            continue
        for child in directory.iterdir():
            if (
                child.name.casefold() in _EXCLUDED
                or not child.is_dir()
                or child.resolve() != child.absolute()
            ):
                continue
            pending.append((child, depth + 1))
    return sorted(found)


@dataclass
class _Legacy:
    directory: Path
    manifest: Manifest
    timeline: Timeline
    transcripts: dict[str, Transcript]
    silences: dict[str, tuple[media.Silence, ...]]
    cached: dict[tuple[str, float, int], tuple[media.Silence, ...]]
    plan: render.Plan


async def _read(
    directory: Path, destination: Path, tools: media.Tools | None, check: Check, workspace: Path
) -> _Legacy:
    for name in ("project.json", "timeline.json"):
        check(directory / name, "read")
    manifest = Manifest.model_validate_json((directory / "project.json").read_bytes())
    timeline = Timeline.model_validate_json((directory / "timeline.json").read_bytes())
    transcripts: dict[str, Transcript] = {}
    quiet: dict[str, tuple[media.Silence, ...]] = {}
    cached: dict[tuple[str, float, int], tuple[media.Silence, ...]] = {}
    main = {entry.asset for entry in timeline.main}
    for asset in manifest.assets:
        check(Path(asset.path), "read")
        path = directory / "transcripts" / f"{asset.id}.json"
        if path.parent != directory / "transcripts":
            raise ProjectError(f"invalid legacy media id: {asset.id}")
        if path.exists():
            check(path, "read")
            transcripts[asset.id] = Transcript.model_validate_json(path.read_bytes())
            if transcripts[asset.id].digest != asset.identity.digest:
                raise ProjectError(f"{asset.id} has a transcript of another file")
        if asset.audio is None:
            continue
        prefix = f"{asset.id}-{asset.identity.digest[:12]}-silences-"
        for cache in (directory / "cache").glob("*.json"):
            if not cache.name.startswith(prefix):
                continue
            match = re.fullmatch(r"(-?\d+(?:\.\d+)?)-(\d+)\.json", cache.name[len(prefix) :])
            if match is None:
                continue
            noise, shortest = float(match[1]), int(match[2])
            check(cache, "read")
            pairs = json.loads(cache.read_text(encoding="utf-8"))
            silences = tuple(media.Silence(int(first), int(last)) for first, last in pairs)
            cached[asset.identity.digest, noise, shortest] = silences
            if noise == -35 and shortest == 300_000:
                quiet[asset.id] = silences
        if asset.id in main and asset.id not in quiet:
            if tools is None:
                tools = await media.located(
                    os.environ.get("PATH", ""), [workspace], cwd=destination
                )
            quiet[asset.id] = await media.silences(
                tools,
                Path(asset.path),
                place=destination / "analysis",
                duration_us=asset.duration_us,
            )
            cached[asset.identity.digest, -35, 300_000] = quiet[asset.id]
    missing = main - transcripts.keys()
    if missing:
        raise ProjectError(f"legacy film has no transcript for {', '.join(sorted(missing))}")
    canvas = manifest.canvas or Canvas(width=1920, height=1080, rate="25/1")
    if manifest.canvas is None:
        # Audio-only legacy projects count frames in milliseconds (1000 fps).
        # Studio still needs a picture rate; preserve times when converting to its 25 fps.
        timeline = timeline.model_copy(
            update={
                "crossfades": {
                    key: round(Fraction(value, 1000) * 25)
                    for key, value in timeline.crossfades.items()
                },
            }
        )
    plan = render.planned(
        timeline, transcripts, quiet, {a.id: a.duration_us for a in manifest.assets}, canvas
    )
    return _Legacy(directory, manifest, timeline, transcripts, quiet, cached, plan)


def _clip(
    id: str, start: int, length: int, source: dict[str, Any], *, link: str | None = None
) -> dict[str, Any]:
    return {
        "id": id,
        "start": start,
        "length": length,
        "source": source,
        "enabled": True,
        "speed": {"num": 1, "den": 1},
        "reverse": False,
        "link": link,
        "transform": edits.Transform().model_dump(),
        "gain_db": 0.0,
        "pan": 0.0,
        "fade_in": 0,
        "fade_out": 0,
    }


def _source(item: Mapping[str, Any], ticks: int) -> dict[str, Any]:
    return {
        "kind": "media",
        "media": item["id"],
        "in": {"ticks": ticks, "scale": item["facts"]["duration"]["scale"]},
    }


def _span(old: _Legacy, layer: Layer) -> tuple[int, int] | None:
    plan = old.plan

    def frame_us(frame: int) -> int:
        return plan.us(frame) if old.manifest.canvas is not None else frame * 1000

    start = (
        render.timed(plan, old.transcripts, layer.anchor)
        if layer.anchor is not None
        else frame_us(layer.at_frame or 0)
    )
    if start is None or start >= plan.length_us:
        return None
    if layer.until is not None:
        end = render.timed(plan, old.transcripts, layer.until, end=True)
        if end is None:
            return None
    elif layer.frames is not None:
        end = start + frame_us(layer.frames)
    else:
        asset = next((a for a in old.manifest.assets if a.id == layer.asset), None)
        end = (
            start + round(max(0, asset.duration_us - layer.source_us) / layer.speed)
            if asset is not None and asset.kind != "image"
            else plan.length_us
        )
    first, last = max(0, plan.frame(start)), min(plan.frames, plan.frame(end))
    return (first, last) if last > first else None


def _transform(layer: Layer, asset: Asset, canvas: Canvas) -> dict[str, float]:
    picture = asset.video
    if picture is None:
        raise ProjectError(f"{asset.id} has no picture")
    width, height = picture.width, picture.height
    if abs(picture.rotation) % 180 == 90:
        width, height = height, width
    fit = min(canvas.width / width, canvas.height / height)
    if layer.position == "full":
        scale = max(canvas.width / width, canvas.height / height) / fit
        return edits.Transform(scale=scale, opacity=layer.opacity).model_dump()
    shown_width = max(2, round(canvas.width * layer.scale / 2) * 2)
    shown_height = shown_width * height / width
    scale = shown_width / (width * fit)
    margin = round(min(canvas.width, canvas.height) * 0.04)
    x = (canvas.width - shown_width) / 2
    y = (canvas.height - shown_height) / 2
    if "left" in layer.position:
        x = margin
    elif "right" in layer.position:
        x = canvas.width - shown_width - margin
    if "top" in layer.position:
        y = margin
    elif "bottom" in layer.position:
        y = canvas.height - shown_height - margin
    return edits.Transform(
        x=(x + shown_width / 2) / canvas.width - 0.5,
        y=(y + shown_height / 2) / canvas.height - 0.5,
        scale=scale,
        opacity=layer.opacity,
    ).model_dump()


def _bounded(
    clip: dict[str, Any], item: Mapping[str, Any], rate: Fraction, dropped: list[str]
) -> None:
    facts = item["facts"]
    picture: dict[str, Any] = facts.get("video") or {}
    if picture.get("still", False):
        return
    duration = facts["duration"]
    speed = clip["speed"]
    remaining = duration["ticks"] - clip["source"]["in"]["ticks"]
    maximum = ((remaining + 1) * rate.numerator * speed["den"] - 1) // (
        duration["scale"] * rate.denominator * speed["num"]
    )
    # The span may end at EOF, but the last displayed frame must start before it.
    maximum = min(
        maximum,
        (remaining * rate.numerator * speed["den"] - 1)
        // (duration["scale"] * rate.denominator * speed["num"])
        + 1,
    )
    if clip["length"] > maximum:
        dropped.append(
            f"{clip['id']}: length {clip['length']} shortened to {maximum} frames at source end"
        )
        clip["length"] = max(0, maximum)


def _timeline(
    old: _Legacy, media_by_asset: Mapping[str, dict[str, Any]], dropped: list[str]
) -> list[dict[str, Any]]:
    plan = old.plan
    canvas = old.manifest.canvas or Canvas(width=1920, height=1080, rate="25/1")
    video = edits.track("video", id="$main_video", name="V1")
    audio = edits.track("audio", id="$main_audio", name="A1")
    video_tracks, audio_tracks = [video], [audio]
    transitions: list[dict[str, Any]] = []
    links: list[dict[str, Any]] = []
    slots: list[dict[str, str]] = []
    for i, cut in enumerate(plan.cuts):
        item = media_by_asset[cut.asset]
        before = plan.fades[i - 1] // 2 if i else 0
        after = plan.fades[i] - plan.fades[i] // 2
        length = plan.lengths[i] - before - after
        start = plan.starts[i] + before
        scale = item["facts"]["duration"]["scale"]
        ticks = (
            (plan.frame(cut.start_us) + before)
            * plan.rate.denominator
            * scale
            // plan.rate.numerator
        )
        slot: dict[str, str] = {}
        for kind, track in (("video", video), ("audio", audio)):
            if item["facts"].get(kind) is None:
                continue
            id = f"$main_{kind}_{i}"
            clip = _clip(id, start, length, _source(item, ticks))
            _bounded(clip, item, plan.rate, dropped)
            if clip["length"] > 0:
                track["clips"].append(clip)
                slot[kind] = id
        slots.append(slot)
        if len(slot) == 2:
            links.extend(edits.set_clip("$film", id, link=f"$group{i}") for id in slot.values())
    for i, fade in enumerate(plan.fades[:-1]):
        if fade <= 0:
            continue
        transitions.extend(
            edits.add_transition(
                "$film",
                slots[i][kind],
                slots[i + 1][kind],
                fade,
                kind="dissolve" if kind == "video" else "crossfade",
            )
            for kind in ("video", "audio")
            if kind in slots[i] and kind in slots[i + 1]
        )
    for kind in ("picture", "sound", "title"):
        for i, layer in enumerate(old.timeline.layers):
            if layer.kind != kind:
                continue
            span = _span(old, layer)
            if span is None:
                dropped.append(f"{layer.id}: its anchor was cut or its range is outside the film")
                continue
            first, last = span
            asset = None
            item = None
            if kind == "title":
                margin = round(canvas.height * 0.06)
                align: Literal["left", "center", "right"] = (
                    "left"
                    if "left" in layer.position
                    else "right"
                    if "right" in layer.position
                    else "center"
                )
                x = (
                    margin / canvas.width
                    if align == "left"
                    else 1 - margin / canvas.width
                    if align == "right"
                    else 0.5
                )
                y = (
                    margin / canvas.height
                    if "top" in layer.position
                    else 1 - margin / canvas.height
                    if "bottom" in layer.position
                    else 0.5
                )
                style = edits.TitleStyle(
                    size=max(1, round(canvas.height * 0.08)), x=x, y=y, align=align
                )
                source = {"kind": "title", "text": layer.text or "", "style": style.model_dump()}
            else:
                asset = next((a for a in old.manifest.assets if a.id == layer.asset), None)
                if asset is None or (
                    asset.video is None if kind == "picture" else asset.audio is None
                ):
                    dropped.append(f"{layer.id}: its {kind} source is unavailable")
                    continue
                item = media_by_asset[asset.id]
                scale = item["facts"]["duration"]["scale"]
                source = _source(item, layer.source_us * scale // 1_000_000)
            clip = _clip(f"$layer{i}", first, last - first, source)
            speed = Fraction(str(layer.speed)).limit_denominator(1000)
            clip["speed"] = {"num": speed.numerator, "den": speed.denominator}
            fade = min(clip["length"] // 2, max(0, plan.frame(layer.fade_us)))
            clip.update(fade_in=fade, fade_out=fade)
            if kind == "picture" and asset is not None:
                clip["transform"] = _transform(layer, asset, canvas)
            elif kind == "sound":
                clip["gain_db"] = (
                    max(-96, 20 * math.log10(layer.volume)) if layer.volume > 0 else -96
                )
                clip["enabled"] = layer.volume > 0
            if item is not None:
                _bounded(clip, item, plan.rate, dropped)
            if clip["length"] < 1:
                continue
            clip["fade_in"] = clip["fade_out"] = min(fade, clip["length"] // 2)
            tracks = audio_tracks if kind == "sound" else video_tracks
            track_kind = "audio" if kind == "sound" else "video"
            track = edits.track(
                track_kind,
                id=f"$layer_track{i}",
                name=("A" if kind == "sound" else "V") + str(len(tracks) + 1),
            )
            track["clips"].append(clip)
            tracks.append(track)
    tracks = [*video_tracks, *audio_tracks]
    if old.timeline.captions:
        subtitles = edits.track("subtitle", id="$subtitles", name="Subtitles")
        for cue in render.captions(plan, old.timeline, old.transcripts):
            first, last = max(0, plan.frame(cue.start_us)), min(plan.frames, plan.frame(cue.end_us))
            if last > first:
                subtitles["cues"].append(
                    {"id": "", "start": first, "length": last - first, "text": cue.text}
                )
        tracks.append(subtitles)
    if old.timeline.duck and any(layer.kind == "sound" for layer in old.timeline.layers):
        dropped.append(
            "duck: automatic music ducking is deferred to the Fairlight phase; "
            "static music gain is retained"
        )
    add = edits.add_timeline(
        old.directory.name,
        edits.Rate(num=plan.rate.numerator, den=plan.rate.denominator),
        id="$film",
        width=canvas.width,
        height=canvas.height,
        tracks=tracks,
    )
    return [add, *links, *transitions]


def _asset_paths(old: _Legacy) -> dict[str, Path]:
    return {asset.id: Path(asset.path).resolve() for asset in old.manifest.assets}


def _existing(project: Mapping[str, Any]) -> dict[Path, dict[str, Any]]:
    return {Path(item["path"]).resolve(): item for item in project["media"]}


async def _one(
    directory: Path, engine: Engine, tools: media.Tools | None, check: Check, workspace: Path
) -> dict[str, Any]:
    check(directory / "migrated.json", "write")
    recovered = await asyncio.to_thread(_recovered, directory, engine.directory)
    if recovered is not None:
        return recovered
    old = await _read(directory, engine.directory, tools, check, workspace)
    asset_paths = await asyncio.to_thread(_asset_paths, old)
    paths = list(dict.fromkeys(asset_paths.values()))
    inspected = await engine.inspected(paths) if paths else []
    revision, project = await engine.project()
    existing = await asyncio.to_thread(_existing, project)
    prepared: dict[Path, dict[str, Any]] = {}
    ops: list[dict[str, Any]] = []
    for i, (path, item) in enumerate(zip(paths, inspected, strict=True)):
        known = existing.get(path)
        if known is not None:
            if known["identity"]["digest"] != item["identity"]["digest"]:
                raise ProjectError(
                    f"media pool source changed at {path}; relink or import it before migration"
                )
            picture: dict[str, Any] = item["facts"].get("video") or {}
            known_picture: dict[str, Any] = known["facts"].get("video") or {}
            if picture.get("still") and not known_picture.get("still"):
                ops.append(edits.set_media(known["id"], still=True))
                # The batch updates these facts before adding the migrated clips.
                known_picture["still"] = True
            prepared[path] = known
        else:
            item["id"] = f"$media{i}"
            prepared[path] = item
            ops.append({"op": "add_media", "media": item})
    media_by_asset = {id: prepared[path] for id, path in asset_paths.items()}
    for asset in old.manifest.assets:
        if media_by_asset[asset.id]["identity"]["digest"] != asset.identity.digest:
            raise ProjectError(
                f"legacy source {asset.id} changed since import; "
                "its cuts cannot be migrated against another file"
            )
    dropped: list[str] = []
    ops += _timeline(old, media_by_asset, dropped)
    _analysis(old, engine.directory, dropped)
    while True:
        try:
            applied = await engine.apply(ops, revision=revision)
            break
        except EngineError as error:
            data = error.data or {}
            index, longest = data.get("index"), data.get("longest")
            if (
                data.get("reason") != "no_handles"
                or type(index) is not int
                or type(longest) is not int
                or not 0 <= index < len(ops)
            ):
                raise
            operation = ops[index]
            if operation.get("edit") != "add_transition" or not 0 <= longest < operation["length"]:
                raise
            dropped.append(
                f"{operation['kind']} at {operation['left']}/{operation['right']}: "
                f"{operation['length']} frames shortened to {longest} for available handles"
            )
            if longest:
                operation["length"] = longest
            else:
                ops.pop(index)
    mapping = {
        key: applied.names.get(item["id"], item["id"]) for key, item in media_by_asset.items()
    }
    report: dict[str, Any] = {
        "timeline": applied.names["$film"],
        "at": datetime.now(UTC).isoformat(),
        "media": mapping,
        "dropped": dropped,
    }
    result = {"source": str(directory), **report}
    # A committed project must not be reported as a failed edit: retrying would duplicate it.
    # Keep a second receipt in Studio so a failed legacy marker can be repaired on the next run.
    warnings: list[str] = []
    try:
        written(_receipts(engine.directory) / f"{report['timeline']}.json", _json(result))
    except ProjectError as error:
        warnings.append(str(error))
    try:
        written(directory / "migrated.json", _json(report))
        (_receipts(engine.directory) / f"{report['timeline']}.json").unlink(missing_ok=True)
    except (ProjectError, OSError) as error:
        warnings.append(f"migration committed as {report['timeline']}; marker: {error}")
    try:
        revision, project = await engine.project()
        timeline = next((t for t in project["timelines"] if t["id"] == report["timeline"]), None)
        if timeline is not None:
            track = story.dialogue_track(timeline)
            if track is not None:
                written(
                    story.story_path(engine.directory, timeline["id"]),
                    story.of_track(
                        timeline, track, words.transcripts_of(engine.directory, project), revision
                    ),
                )
    except (ProjectError, words.WordsError, story.StoryError, EngineError, OSError) as error:
        warnings.append(f"migration committed; analysis/story: {error}")
    if warnings:
        result["warnings"] = warnings
    return result


def _json(value: Mapping[str, Any]) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def _receipts(studio: Path) -> Path:
    return studio / "analysis" / "migrations"


def _recovered(directory: Path, studio: Path) -> dict[str, Any] | None:
    for receipt in _receipts(studio).glob("t[0-9]*.json"):
        value: dict[str, Any] = json.loads(receipt.read_text(encoding="utf-8"))
        if value.get("source") == str(directory):
            report = {key: value[key] for key in ("timeline", "at", "media", "dropped")}
            written(directory / "migrated.json", _json(report))
            receipt.unlink()
            return value
    return None


def _analysis(old: _Legacy, studio: Path, dropped: list[str]) -> None:
    for transcript in old.transcripts.values():
        previous = words.kept_words(studio, transcript.digest)
        converted = words.Words.of(transcript)
        if previous is None:
            words.keep_words(studio, converted)
        elif previous != converted:
            dropped.append(f"{transcript.asset}: existing Studio transcription kept")
    for (digest, noise, shortest), silences in old.cached.items():
        words.keep_silences(studio, digest, noise, shortest, silences)


async def migrate(
    workspace: Path, engine: Engine, *, tools: media.Tools | None = None, check: Check | None = None
) -> list[dict[str, Any]]:
    """Migrate each discovered old project as one transaction, and return its receipt."""
    check = check or (lambda path, kind: None)
    if not await asyncio.to_thread(discover, workspace):
        return []
    # Desktop startup and the first agent call may arrive together. Only migration takes this
    # lock; ordinary edits still use the engine's revision check and remain available.
    deadline = asyncio.get_running_loop().time() + 30
    while (held := lock.acquire(engine.directory / "migration.lock")) is None:
        if asyncio.get_running_loop().time() >= deadline:
            raise ProjectError("another client is migrating this Studio project; try again later")
        await asyncio.sleep(0.1)
    reports: list[dict[str, Any]] = []
    try:
        for directory in await asyncio.to_thread(discover, workspace):
            try:
                reports.append(await _one(directory, engine, tools, check, workspace))
            except (OSError, ValueError) as error:
                raise ProjectError(f"cannot migrate {directory}: {error}") from None
    finally:
        held.release()
    return reports
