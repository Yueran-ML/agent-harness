"""Agent editing through the engine, sharing the desktop's timelines and undo history.

Analysis and editable stories are derived files. Timeline edits are submitted once, as one
engine transaction. All films are rendered by the engine.
"""

import asyncio
import contextlib
import json
import re
import unicodedata
from collections.abc import Callable, Mapping, Sequence
from dataclasses import asdict
from pathlib import Path
from typing import Any, Literal, cast

from columbina.video import edits, grading, media, story, textcut, words
from columbina.video.edits import CueItem
from columbina.video.engine import Applied, Engine, EngineError, Progress
from columbina.video.project import ProjectError, identity_of, written
from columbina.video.span import placed, source_span
from columbina.video.transcribe import Transcriber, TranscriptionError, transcript_of
from columbina.video.transcript import Transcript

__all__ = ["Editor", "Subtitles", "chosen_subtitles", "receipt", "safe_name", "sidecars"]

Subtitles = Literal["visible", "all"] | Sequence[str]
"""The subtitle tracks that an export writes as SRT: ``visible``, the one that the timeline
shows, if any; ``all``, every subtitle track, shown or hidden; or the ids of tracks."""


def receipt(value: object) -> str:
    """A compact, structured receipt whose ids can be used by the next tool."""
    return json.dumps(value, ensure_ascii=False, indent=2)


def safe_name(name: str) -> str:
    """A display name used as one filename, without interpreting it as a path."""
    name = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", name).strip(" .")[:80]
    if name.split(".")[0].upper() in {
        "CON",
        "PRN",
        "AUX",
        "NUL",
        *(f"COM{i}" for i in range(1, 10)),
        *(f"LPT{i}" for i in range(1, 10)),
    }:
        name = "_" + name
    return name or "Timeline"


def chosen_subtitles(timeline: Mapping[str, Any], chosen: Subtitles) -> list[str]:
    """The ids of the subtitle tracks of a timeline that ``chosen`` names, for ``render.start``.

    A timeline shows at most one subtitle track, as DaVinci Resolve does; an export writes the
    SRT of any of them, shown or hidden. Ids are given as they are, for the engine to check.
    """
    subtitles = [one for one in timeline.get("tracks", []) if one.get("kind") == "subtitle"]
    if chosen == "visible":
        return [str(one["id"]) for one in subtitles if one.get("enabled", True)]
    if chosen == "all":
        return [str(one["id"]) for one in subtitles]
    if isinstance(chosen, str):
        return [chosen]
    return list(chosen)


def sidecars(timeline: Mapping[str, Any], path: Path, chosen: Sequence[str]) -> list[Path]:
    """The SRT files that an export of the subtitle tracks ``chosen`` writes beside ``path``.

    As the engine names them, in the order of the tracks of the timeline: one track is
    ``<stem>.srt``; more are ``<stem>.<language or name>.srt``, the name made safe and numbered
    where two are alike.
    """
    tracks = [
        one
        for one in timeline.get("tracks", [])
        if one.get("kind") == "subtitle" and one.get("id") in chosen
    ]
    if len(tracks) == 1:
        return [path.with_suffix(".srt")]
    taken: set[str] = set()
    found: list[Path] = []
    for track in tracks:
        label = _label(str(track.get("language") or track.get("name") or ""))
        name, suffix = label, 2
        while name.lower() in taken:
            name, suffix = f"{label}-{suffix}", suffix + 1
        taken.add(name.lower())
        found.append(path.with_name(f"{path.stem}.{name}.srt"))
    return found


def _label(text: str) -> str:
    """A language or a name made safe for a file, as ``safe_name`` of the engine's ``srt.rs``."""
    name = "".join(
        "_" if unicodedata.category(one) == "Cc" or one in '<>:"/\\|?*' else one
        for one in text[:80]
    ).strip(". ")
    return name or "subtitles"


def _missing(message: str) -> EngineError:
    return EngineError(message, code=-32602, data={"reason": "missing"})


def _timeline(project: Mapping[str, Any], id: str | None) -> dict[str, Any]:
    timelines: list[dict[str, Any]] = project["timelines"]
    if id is not None:
        for timeline in timelines:
            if timeline["id"] == id:
                return timeline
        raise _missing(f"there is no timeline {id}; video_info lists the timelines")
    if len(timelines) == 1:
        return timelines[0]
    if not timelines:
        raise _missing("there is no timeline; create one with video_edit and add_timeline")
    raise EngineError(
        "name a timeline: this project has more than one", code=-32602, data={"reason": "ambiguous"}
    )


def _media(project: Mapping[str, Any], id: str) -> dict[str, Any]:
    for item in project["media"]:
        if item["id"] == id:
            return item
    raise _missing(f"there is no media {id}; video_info lists the media")


_TIMELINE_OPERATIONS = frozenset(
    {
        "set_clip", "set_track", "move_track", "insert_clip", "remove_clip", "add_track",
        "remove_track",
        # The operations of sound (schema 4).
        "set_track_audio", "add_bus", "remove_bus", "set_bus", "add_vca", "remove_vca",
        "set_vca", "add_track_group", "remove_track_group", "set_track_group", "add_insert",
        "set_insert", "move_insert", "set_timeline_audio",
        # The operations of automation (T14).
        "set_lane", "splice_lane",
        # The keys of a clip's settings (phase 4).
        "set_clip_keys", "splice_clip_keys",
        # The speed points of a clip (phase 4).
        "set_retime",
    }
)  # fmt: skip
"""The basic operations whose ``timeline`` the tool's fills where it is left out."""

_GRADE_EDITS = frozenset(
    {
        "add_serial", "add_serial_before", "append_node", "add_parallel", "add_layer",
        "add_outside", "extract_node", "reset_node", "copy_grade", "append_node_graph",
        "paste_value", "paste_node",
    }
)  # fmt: skip
"""The edits of grades that name their grades by targets (``grade``, ``from``, ``to``) and take
no ``timeline``; the engine refuses one given them. ``collapse_group_grades`` names clips, and
takes the timeline of its clips."""

_TIMELINE_LEVELS = frozenset({"clip", "timeline"})
"""The levels of a target of a grade that name a timeline."""


def _with_timeline(target: object, timeline: str) -> object:
    """A target of a grade, or a list of them, with the timeline it leaves out filled."""
    if isinstance(target, list):
        return [_with_timeline(one, timeline) for one in cast("list[object]", target)]
    if isinstance(target, Mapping):
        given = dict(cast("Mapping[str, Any]", target))
        if given.get("level") in _TIMELINE_LEVELS:
            given.setdefault("timeline", timeline)
        return given
    return target


def _timeline_left_out(item: Mapping[str, Any]) -> bool:
    """Whether an item leaves out a timeline that the tool's is to fill."""
    if item.get("edit") in _GRADE_EDITS:
        targets = [item.get(field) for field in ("grade", "from", "to")]
        flat = [
            one
            for target in targets
            for one in (cast("list[object]", target) if isinstance(target, list) else [target])
        ]
        return any(
            isinstance(one, Mapping)
            and cast("Mapping[str, Any]", one).get("level") in _TIMELINE_LEVELS
            and "timeline" not in one
            for one in flat
        )
    return ("edit" in item or item.get("op") in _TIMELINE_OPERATIONS) and "timeline" not in item


def _applied(change: Applied) -> dict[str, Any]:
    return asdict(change)


def _cues(timeline: Mapping[str, Any], transcripts: Mapping[str, Transcript]) -> list[CueItem]:
    """The subtitles of what the dialogue track says, of every media of it that is transcribed.

    Sentence chunks on their actual clips, including repeated, sped-up and reversed takes, in
    the order in which they are shown; each ends where the next begins, if it would end later,
    since the words that a transcriber times can overlap and the engine refuses overlapping
    subtitles.
    """
    track = story.dialogue_track(timeline)
    if track is None:
        raise ProjectError("the timeline has no dialogue track to make subtitles from")
    told = story.telling(timeline, track, transcripts)
    items: list[CueItem] = []
    for part in told.parts:
        transcript = transcripts[part.clip.media]
        chunk: list[story.Spoken] = []
        for word in [*part.words, None]:
            if word is not None:
                trying = [*chunk, word]
                text = story.line_of([(transcript.text(one.index), False) for one in trying])
                if not chunk or sum(2 if ord(c) > 0x2E80 else 1 for c in text) <= 42:
                    chunk = trying
                    continue
            if chunk:
                start = min(one.first for one in chunk)
                end = max(one.end for one in chunk)
                if end > start:
                    items.append(
                        CueItem(
                            start=start,
                            length=end - start,
                            text=story.line_of(
                                [(transcript.text(one.index), False) for one in chunk]
                            ),
                        )
                    )
            chunk = [word] if word is not None else []
    return edits.untangled(items)


class Editor:
    """One connection, with serialized multi-call analysis/story operations."""

    def __init__(self, engine: Engine, transcriber: Transcriber | None = None) -> None:
        self.engine = engine
        self.directory = engine.directory
        self.transcriber = transcriber
        self.lock = asyncio.Lock()

    async def timeline(self, id: str | None = None) -> dict[str, Any]:
        """Resolve an explicit timeline, or the only timeline in the project."""
        _, project = await self.engine.project()
        return _timeline(project, id)

    async def imported(self, paths: Sequence[Path], bin: str | None = None) -> str:
        """Import the whole batch into its bin as one history step."""
        revision, added = await self.engine.imported(paths, bin=bin)
        return receipt({"revision": revision, "added": added})

    async def edited(
        self, operations: Sequence[Mapping[str, Any]], timeline: str | None = None
    ) -> str:
        """Only fill omitted timeline targets; the engine interprets every edit.

        An edit of a grade takes no timeline of its own: its grades are named by targets, and
        a target of a clip or a timeline that leaves its timeline out is given the tool's.
        """
        batch = [dict(one) for one in operations]
        if any(_timeline_left_out(one) for one in batch):
            target = (await self.timeline(timeline))["id"]
            for one in batch:
                if one.get("edit") in _GRADE_EDITS:
                    for field in ("grade", "from", "to"):
                        if field in one:
                            one[field] = _with_timeline(one[field], target)
                elif "edit" in one or one.get("op") in _TIMELINE_OPERATIONS:
                    one.setdefault("timeline", target)
        return receipt(_applied(await self.engine.apply(batch)))

    async def told(self, timeline: str | None = None, track: str | None = None) -> str:
        """Refresh the editable story from one current dialogue track."""
        revision, project = await self.engine.project()
        target = _timeline(project, timeline)
        track = track or story.dialogue_track(target)
        if track is None:
            raise ProjectError("the timeline has no video or audio track for a story")
        path = story.story_path(self.directory, target["id"])

        def write() -> None:
            # Long transcripts take a while to tell; the connection is served meanwhile.
            transcripts = words.transcripts_of(self.directory, project)
            written(path, story.of_track(target, track, transcripts, revision))

        await asyncio.to_thread(write)
        return receipt(
            {"path": str(path), "timeline": target["id"], "track": track, "revision": revision}
        )

    async def applied(
        self, timeline: str | None = None, *, dry: bool = False, pause_most_ms: int | None = None
    ) -> str:
        """Turn the edited story into ripple ranges and submit them together.

        The story is good for the timeline as it is now where its digest is that of the
        timeline, the track and the words now (:func:`columbina.video.story.marked`): the
        revision that it names is not compared. The cuts are then applied only at the revision,
        and in the session of the engine, that the timeline was read at, so that an edit made
        meanwhile, or an engine started again and counting its revisions from 0, refuses them.
        """
        revision, project = await self.engine.project()
        session = self.engine.session
        target = _timeline(project, timeline)
        path = story.story_path(self.directory, target["id"])

        def work() -> tuple[story.Marked, list[textcut.Range]]:
            try:
                text = path.read_text(encoding="utf-8")
            except FileNotFoundError:
                raise story.StoryError(
                    f"there is no story of timeline {target['id']} yet: write it with "
                    "video_story, change it, and apply it then"
                ) from None
            transcripts = words.transcripts_of(self.directory, project)
            said = story.marked(text, target, transcripts)
            return said, textcut.cut_by(said, transcripts, pause_most_ms=pause_most_ms)

        marked, ranges = await asyncio.to_thread(work)
        report: dict[str, Any] = {
            "timeline": target["id"],
            "dry": dry,
            "ranges": [asdict(one) for one in ranges],
            "removed_frames": sum(one.frames for one in ranges),
        }
        if not dry and ranges:
            report.update(
                _applied(
                    await self.engine.apply(
                        textcut.edits_of(target["id"], ranges), revision=revision, session=session
                    )
                )
            )
            try:
                await self.told(target["id"], marked.telling.track)
            except (
                EngineError,
                story.StoryError,
                words.WordsError,
                ProjectError,
                OSError,
            ) as error:
                # The cut already committed. A person may have removed the track meanwhile;
                # report the successful transaction rather than invite the caller to repeat it.
                report["story_refresh_error"] = str(error)
        return receipt(report)

    async def transcribed(
        self,
        id: str,
        tools: media.Tools | None,
        *,
        language: str | None = None,
        again: bool = False,
        timeline: str | None = None,
        subtitles: str | None = None,
        check: Callable[[Path], object] | None = None,
    ) -> str:
        """Keep aligned word analysis and optionally replace a subtitle track."""
        _, project = await self.engine.project()
        item = _media(project, id)
        source = Path(item["path"])
        if check is not None:
            check(source)
        digest: str = item["identity"]["digest"]
        known = words.kept_words(self.directory, digest)
        reused = known is not None and not again
        target = _timeline(project, timeline) if subtitles is not None else None
        if target is not None:
            found = next((t for t in target["tracks"] if t["id"] == subtitles), None)
            if found is None or found["kind"] != "subtitle":
                raise _missing(f"there is no subtitle track {subtitles} on timeline {target['id']}")
        if not reused:
            if self.transcriber is None:
                raise TranscriptionError(
                    "set COLUMBINA_WHISPER_MODEL to a faster-whisper model before transcribing"
                )
            if item["facts"].get("audio") is None:
                raise TranscriptionError(f"media {id} has no sound")
            if tools is None:
                raise media.MediaUnavailableError("ffmpeg is needed for transcription")
            # Use the existing media identity to avoid assigning words to a replaced source.
            if (await asyncio.to_thread(identity_of, source)).digest != digest:
                raise TranscriptionError(
                    f"media {id} changed since import; import the current file again"
                )
            duration = item["facts"]["duration"]
            duration_us = duration["ticks"] * 1_000_000 // duration["scale"]
            cache = words.words_path(self.directory, digest).parent
            cache.mkdir(parents=True, exist_ok=True)
            # The sound is taken out for the transcriber alone, and is as large as a film is
            # long: it is not kept once the words are, nor when they could not be had.
            sound = cache / f"audio-{digest[:16]}.wav"
            try:
                if not sound.exists():
                    await media.audio(tools, source, sound)
                quiet = words.kept_silences(self.directory, digest, -35, 300_000)
                if quiet is None:
                    quiet = await media.silences(
                        tools, source, place=cache, duration_us=duration_us
                    )
                    words.keep_silences(self.directory, digest, -35, 300_000, quiet)
                heard = await self.transcriber.words(sound, language)
            finally:
                with contextlib.suppress(OSError):
                    await asyncio.to_thread(sound.unlink, missing_ok=True)
            if not heard:
                raise TranscriptionError(f"nothing was heard in {id}")
            known = words.Words.of(transcript_of(id, digest, heard, quiet, duration_us, language))
            words.keep_words(self.directory, known)
        assert known is not None
        report: dict[str, Any] = {
            "media": id,
            "reused": reused,
            "words": len(known.words),
            "sentences": len(known.sentences),
            "path": str(words.words_path(self.directory, digest)),
        }
        if target is not None and subtitles is not None:
            # Transcription may take minutes: use the current timeline when placing the cues.
            revision, project = await self.engine.project()
            session = self.engine.session
            target = _timeline(project, target["id"])
            now = target

            def cued() -> list[CueItem]:
                return _cues(now, words.transcripts_of(self.directory, project))

            items = await asyncio.to_thread(cued)
            report.update(
                _applied(
                    await self.engine.apply(
                        [edits.cues_from(target["id"], subtitles, items)],
                        revision=revision,
                        session=session,
                    )
                )
            )
            report["cues"] = len(items)
        return receipt(report)

    async def corrected(self, media_id: str, sentence: int, wrong: str, right: str) -> str:
        """Correct displayed analysis text without changing the recorded words or audio."""
        _, project = await self.engine.project()
        item = _media(project, media_id)
        changed = words.corrected(
            self.directory, item["identity"]["digest"], sentence, wrong, right
        )
        return receipt(
            {
                "media": media_id,
                "sentence": sentence,
                "shown": changed.shown,
                "note": (
                    "Display text corrected; audio unchanged. Refresh video_story "
                    "before the next text cut."
                ),
            }
        )

    async def rendered(
        self,
        timeline: str,
        path: Path,
        *,
        codec: Literal["h264", "hevc", "prores"],
        quality: Literal["high", "medium", "small"],
        subtitle_tracks: Sequence[str],
        replace: bool,
        told: Callable[[Progress], object] | None = None,
    ) -> str:
        """Await an export, relay its progress, and cancel it if the tool is cancelled."""
        path.parent.mkdir(parents=True, exist_ok=True)
        job = await self.engine.render(
            timeline,
            path,
            codec=codec,
            quality=quality,
            subtitle_tracks=subtitle_tracks,
            replace=replace,
        )
        try:
            done = await job.finished(told)
        except asyncio.CancelledError:
            with contextlib.suppress(EngineError):
                await job.cancel()
            raise
        finally:
            await job.aclose()
        if done.state != "done":
            raise EngineError(
                done.message or f"render {done.state}", code=-32001, data={"reason": done.state}
            )
        return receipt(asdict(done))

    async def undone(self, *, redo: bool = False) -> str:
        """Undo or redo the shared project's last transaction, whoever made it."""
        done = await (self.engine.redo() if redo else self.engine.undo())
        return receipt(
            {"changed": done is not None, "redo": redo, **(_applied(done) if done else {})}
        )

    async def info(
        self,
        timeline: str | None = None,
        readable: Callable[[str], Path | None] | None = None,
        *,
        scope: Literal["audio"] | None = None,
    ) -> str:
        """List engine ids and timeline positions for subsequent edits.

        Grades are given in short (:func:`columbina.video.grading.brief`): a corrector has some
        ninety settings, and a clip that is not graded has none to tell of. ``readable`` gives
        the path of a LUT named by an absolute path where the session may read it, to tell
        whether it is there; without it such LUTs are not looked for.
        """
        revision, project = await self.engine.project()
        timelines = [_timeline(project, timeline)] if timeline else project["timelines"]
        if scope == "audio":
            from columbina.video.audio_info import mixer

            return receipt(
                {
                    "project": str(self.directory),
                    "revision": revision,
                    "session": self.engine.session,
                    "scope": "audio",
                    "values": "static; automation and VCA contributions are not evaluated",
                    "analysis_cache_source": "engine_client_calls",
                    "timelines": [
                        {
                            **mixer(target),
                            "latest_analysis": self.engine.latest_sound(
                                target["id"], revision=revision
                            ),
                        }
                        for target in timelines
                    ],
                }
            )
        groups: list[dict[str, Any]] = project.get("groups", [])
        names: set[str] = set()
        for target in timelines:
            names |= grading.luts_of(target.get("grade"))
            for track in target["tracks"]:
                for clip in track.get("clips", []):
                    names |= grading.luts_of(clip.get("grade"))
        for group in groups:
            names |= grading.luts_of(group.get("pre")) | grading.luts_of(group.get("post"))
        missing = await grading.missing_luts(self.engine, names, readable or (lambda _: None))
        summary: list[dict[str, Any]] = []
        for target in timelines:
            rate = edits.Rate.model_validate(target["rate"])
            tracks: list[dict[str, Any]] = []
            for track in target["tracks"]:
                clips: list[dict[str, Any]] = []
                for clip in track.get("clips", []):
                    item = dict(clip)
                    item.pop("grade", None)
                    if clip.get("group") is None:
                        item.pop("group", None)
                    graded = grading.brief(clip.get("grade"), missing)
                    if graded is not None:
                        item["grade"] = graded
                    position = placed(clip)
                    if position is not None:
                        item["source_out"] = {
                            "ticks": position.begins.ticks + source_span(position, rate),
                            "scale": position.begins.scale,
                        }
                    clips.append(item)
                tracks.append(
                    {**track, **({"clips": clips} if track["kind"] != "subtitle" else {})}
                )
            told = {key: value for key, value in target.items() if key != "grade"}
            graded = grading.brief(target.get("grade"), missing)
            if graded is not None:
                told["grade"] = graded
            summary.append(
                {
                    **told,
                    "tracks": tracks,
                    "frames": max(
                        (
                            one["start"] + one["length"]
                            for track in tracks
                            for one in track.get("clips", track.get("cues", []))
                        ),
                        default=0,
                    ),
                }
            )
        transcripts = words.transcripts_of(self.directory, project)
        return receipt(
            {
                "project": str(self.directory),
                "revision": revision,
                "bins": project["bins"],
                "media": [
                    {**item, "transcribed": item["id"] in transcripts} for item in project["media"]
                ],
                "timelines": summary,
                **(
                    {
                        "groups": [
                            {
                                "id": group["id"],
                                "name": group["name"],
                                "pre": grading.brief(group.get("pre"), missing),
                                "post": grading.brief(group.get("post"), missing),
                            }
                            for group in groups
                        ]
                    }
                    if groups
                    else {}
                ),
            }
        )
