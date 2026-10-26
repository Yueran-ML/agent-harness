"""What can be done to a project of editing: what the tools of the model and the command line do.

Each operation takes the project as it is, does one thing, and says in a few sentences what it
did: a receipt, for a model or a person. Whatever changes the timeline commits it, and writes the
story anew from it. The receipt of a commit says what was cut, how long the film lasts before and
after, what lies nowhere any more and which cuts may be heard.

The files of a project, beside those of :mod:`columbina.video.project`: ``transcripts/`` with a
transcript for each source that was transcribed, ``story.md``, and ``out/`` with what was made.
"""

import asyncio
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from pydantic import ValidationError

from columbina.video import render
from columbina.video.media import Silence, Tools
from columbina.video.project import Project, ProjectError, StaleError, written
from columbina.video.story import (
    label,
    line_of,
    normal,
    read,
    whole,
)
from columbina.video.story import written as story_of
from columbina.video.timeline import Layer, Timeline, Transcript
from columbina.video.transcribe import Transcriber, TranscriptionError, transcript_of

__all__ = ["STORY", "TRANSCRIPTS", "Editor", "Placing", "Shape"]

STORY = "story.md"
TRANSCRIPTS = "transcripts"


def _length(us: int) -> str:
    minutes, rest = divmod(us, 60_000_000)
    return f"{minutes}:{rest / 1_000_000:04.1f}"


@dataclass(frozen=True)
class Placing:
    """Where a layer is laid: at words of a sentence of the main line, or at a frame.

    Attributes:
        sentence: The sentence, as the story names it, such as ``talk.3``.
        words: Words of that sentence, which have to be in it once: the layer lasts while they
            are said. Without them, the layer begins with the sentence and lasts as long as it.
        at_frame: The frame where the layer begins, where it is laid at no words.
        frames: How many frames it lasts, where it is laid at a frame. Without it a picture or a
            sound lasts as long as its source, and a title to the end.
    """

    sentence: str | None = None
    words: str | None = None
    at_frame: int | None = None
    frames: int | None = None


@dataclass(frozen=True)
class Shape:
    """How a layer looks and sounds. What is not given stays as it is, or as it is by default."""

    position: (
        Literal["full", "top-left", "top-right", "bottom-left", "bottom-right", "center"] | None
    ) = None
    scale: float | None = None
    opacity: float | None = None
    volume: float | None = None
    fade_ms: int | None = None
    speed: float | None = None
    source_ms: int | None = None

    def changes(self) -> dict[str, object]:
        """What the shape changes of a layer, by the names of its fields."""
        given = {
            "position": self.position,
            "scale": self.scale,
            "opacity": self.opacity,
            "volume": self.volume,
            "fade_us": None if self.fade_ms is None else self.fade_ms * 1000,
            "speed": self.speed,
            "source_us": None if self.source_ms is None else self.source_ms * 1000,
        }
        return {key: value for key, value in given.items() if value is not None}


class Editor:
    """The operations on one project of editing.

    Args:
        directory: The directory of the project.
        tools: ffmpeg and ffprobe.
        transcriber: What writes down what was said, if anything does.
    """

    def __init__(
        self, directory: Path, tools: Tools, transcriber: Transcriber | None = None
    ) -> None:
        self.project = Project(directory)
        self.tools = tools
        self.transcriber = transcriber
        self._lock = asyncio.Lock()

    # Reading the project

    def timeline(self) -> tuple[Timeline, str]:
        """The timeline as it was committed last, and its digest.

        Raises:
            ProjectError: It cannot be read.
        """
        text, digest = self.project.timeline()
        if not text:
            return Timeline(), digest
        try:
            return Timeline.model_validate_json(text), digest
        except ValidationError as error:
            raise ProjectError(f"the timeline is not what this program writes: {error}") from None

    def _transcript_path(self, asset: str) -> Path:
        return self.project.directory / TRANSCRIPTS / f"{asset}.json"

    def transcripts(self) -> dict[str, Transcript]:
        """The transcripts of the sources that were transcribed, in the order of the sources.

        Raises:
            ProjectError: One cannot be read.
        """
        found: dict[str, Transcript] = {}
        for asset in self.project.manifest().assets:
            path = self._transcript_path(asset.id)
            if not path.exists():
                continue
            try:
                found[asset.id] = Transcript.model_validate_json(path.read_bytes())
            except (OSError, ValidationError) as error:
                raise ProjectError(f"the transcript {path} cannot be read: {error}") from None
        return found

    async def _silences(self, assets: Sequence[str]) -> dict[str, tuple[Silence, ...]]:
        found: dict[str, tuple[Silence, ...]] = {}
        for key in assets:
            asset = self.project.asset(key)
            found[key] = (
                await self.project.cached_silences(asset, self.tools) if asset.audio else ()
            )
        return found

    async def _plan(self, timeline: Timeline, transcripts: dict[str, Transcript]) -> render.Plan:
        main = list(dict.fromkeys(entry.asset for entry in timeline.main))
        lengths = {key: self.project.asset(key).duration_us for key in main}
        return render.planned(
            timeline,
            transcripts,
            await self._silences(main),
            lengths,
            self.project.manifest().canvas,
        )

    def _keep(self, timeline: Timeline, base: str, note: str) -> str:
        before, _ = self.timeline()
        digest = self.project.commit(timeline.model_dump_json(indent=1), base, note)
        self._follow(before, base, timeline, digest)
        return digest

    def _write_story(self, timeline: Timeline, digest: str) -> None:
        written(self.project.directory / STORY, story_of(timeline, self.transcripts(), digest))

    def _follow(self, before: Timeline, base: str, after: Timeline, digest: str) -> None:
        """Have the story follow a timeline that changed for another reason than the story.

        A story as it was written is written anew. One that was changed since keeps what was
        changed in it: where the main line is the same, it is now of the new timeline; where the
        main line is another one, it is left as it is, of the timeline before, and is not
        committed.
        """
        path = self.project.directory / STORY
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            self._write_story(after, digest)
            return
        if text == story_of(before, self.transcripts(), base):
            self._write_story(after, digest)
        elif before.main == after.main:
            head, _, rest = text.partition("\n")
            if base[:12] in head:
                written(path, f"{head.replace(base[:12], digest[:12])}\n{rest}")

    # Sources and transcripts

    async def imported(self, paths: Sequence[Path]) -> str:
        """Take files in as sources."""
        async with self._lock:
            done = await self.project.imported(paths, self.tools)
        lines: list[str] = []
        for kind, assets in (
            ("Taken in", done.added),
            ("Changed", done.changed),
            ("As before", done.same),
        ):
            for asset in assets:
                facts = (
                    f"{asset.kind}, {_length(asset.duration_us)}"
                    if asset.duration_us
                    else asset.kind
                )
                if asset.video is not None:
                    facts += f", {asset.video.width}x{asset.video.height}"
                lines.append(f"{kind}: {asset.id} ({facts}) from {asset.path}.")
        canvas = self.project.manifest().canvas
        if canvas is not None:
            lines.append(
                f"The film is {canvas.width}x{canvas.height} at {canvas.rate} frames a second."
            )
        lines.extend(f"Note: {warning}." for warning in done.warnings)
        if done.changed:
            lines.append("A source that changed is to be transcribed again before it is cut.")
        return "\n".join(lines)

    async def transcribed(
        self, key: str, language: str | None = None, *, again: bool = False
    ) -> str:
        """Write down what was said in a source, and put it on the main line if it is not there.

        Raises:
            ProjectError: There is no transcriber, the source has no sound, or it is gone.
            TranscriptionError: The transcriber failed.
        """
        if self.transcriber is None:
            raise ProjectError(
                "no transcriber is set up. Set up faster-whisper and a model, or give the words "
                "of the source in a file"
            )
        async with self._lock:
            asset = await self.project.checked(self.project.asset(key))
            known = self.transcripts().get(key)
            if known is not None and known.digest == asset.identity.digest and not again:
                return (
                    f"{key} was transcribed before: {len(known.sentences)} sentences. "
                    f"The story is {STORY}."
                )
            sound = await self.project.cached_audio(asset, self.tools)
            words = await self.transcriber.words(sound, language)
            if not words:
                raise TranscriptionError(f"nothing was heard in {key}")
            silences = await self.project.cached_silences(asset, self.tools)
            transcript = transcript_of(
                key, asset.identity.digest, words, silences, asset.duration_us, language
            )
            written(self._transcript_path(key), transcript.model_dump_json(indent=1))
            timeline, digest = self.timeline()
            # The sentences of the source come at the end, whole: where it was transcribed before,
            # its words are others now, and what was cut of it is not known any more.
            kept = [entry for entry in timeline.main if entry.asset != key]
            kept += whole(transcript)
            self._keep(
                timeline.model_copy(update={"main": tuple(kept)}), digest, f"transcribed {key}"
            )
        shift = (
            f", their times moved by {transcript.offset_us / 1000:.0f} ms to fit the pauses"
            if abs(transcript.offset_us) >= 1000
            else ""
        )
        return (
            f"{key} was transcribed: {len(transcript.sentences)} sentences and "
            f"{len(transcript.words)} words{shift}. Its sentences are at the end of the main line. "
            f"The story is {STORY}: read it, cut it, and apply it."
        )

    # The story

    async def applied(
        self, *, dry: bool = False, pause_most_ms: int | None = None, fresh: bool = False
    ) -> str:
        """Commit the story: the main line becomes what it says.

        Args:
            dry: Say what it would do, and do nothing.
            pause_most_ms: How long a pause of the sources lasts at most, from now on.
            fresh: Write the story anew from the timeline, and commit nothing.

        Raises:
            ProjectError: There is no story, or it was written from another timeline.
            StoryError: A line of it is not understood.
        """
        async with self._lock:
            timeline, digest = self.timeline()
            transcripts = self.transcripts()
            path = self.project.directory / STORY
            if fresh:
                self._write_story(timeline, digest)
                return f"The story was written anew from the timeline as it is: {path}."
            try:
                text = await asyncio.to_thread(path.read_text, encoding="utf-8")
            except OSError as error:
                raise ProjectError(
                    f"there is no story to apply: {error}. Transcribe a source first"
                ) from None
            story = read(text, transcripts)
            if not digest.startswith(story.base):
                raise StaleError(
                    "the story was written from another timeline than the one that is committed: "
                    "something else changed the timeline since. Apply with fresh to write the "
                    "story anew from it, which drops what was changed in the story"
                )
            update: dict[str, object] = {"main": story.main}
            if pause_most_ms is not None:
                update["pause_most_us"] = pause_most_ms * 1000
            after = timeline.model_copy(update=update)
            receipt = await self._receipt(timeline, after, transcripts)
            if dry:
                return f"Not committed. It would be so:\n{receipt}"
            # The story that was committed is written anew, as the timeline is now.
            self._write_story(
                after,
                self.project.commit(
                    after.model_dump_json(indent=1), digest, receipt.splitlines()[0]
                ),
            )
        return f"Committed.\n{receipt}"

    async def _receipt(
        self, before: Timeline, after: Timeline, transcripts: dict[str, Transcript]
    ) -> str:
        """Say what a change of the main line does."""
        old = {(entry.asset, entry.sentence): entry for entry in before.main}
        new = {(entry.asset, entry.sentence): entry for entry in after.main}
        cut_sentences = [key for key in old if key not in new]
        kept_words = sum(len(entry.kept) for entry in after.main)
        had_words = sum(len(entry.kept) for entry in before.main)
        order_before = [key for key in old if key in new]
        order_after = [key for key in new if key in old]
        moved = sum(1 for one, other in zip(order_before, order_after, strict=True) if one != other)
        lines = [
            f"Sentences cut: {len(cut_sentences)}. Words cut: {max(0, had_words - kept_words)}. "
            f"Sentences back: {len([key for key in new if key not in old])}. Moved: {moved}."
        ]
        plan_before = await self._plan(before, transcripts) if before.main else None
        plan_after = await self._plan(after, transcripts) if after.main else None
        lasting = _length(plan_after.length_us) if plan_after else "0:00.0"
        was = _length(plan_before.length_us) if plan_before else "0:00.0"
        lines.append(f"The film lasts {lasting}; it lasted {was}.")
        if plan_after is not None:
            lost = [
                layer.id
                for layer in after.layers
                if layer.anchor is not None
                and render.timed(plan_after, transcripts, layer.anchor) is None
            ]
            if lost:
                lines.append(
                    f"These layers lie nowhere now, their words were cut: {', '.join(lost)}."
                )
            tight = sorted(
                {
                    label(after.main[cut.entry].asset, after.main[cut.entry].sentence)
                    for cut in plan_after.cuts
                    if cut.tight
                }
            )
            if tight:
                lines.append(
                    f"Cuts in {', '.join(tight)} fall where no pause is, and may be heard."
                )
        return "\n".join(lines)

    async def corrected(self, sentence: str, wrong: str, right: str) -> str:
        """Show a word that was heard wrong as it was said. Its time stays as it is.

        Raises:
            ProjectError: There is no such sentence, or the word is not in it once.
        """
        async with self._lock:
            transcript, number = self._sentence(sentence)
            first, last = transcript.sentences[number]
            found = [
                index
                for index in range(first, last)
                if normal(transcript.text(index)) == normal(wrong)
            ]
            if len(found) != 1:
                raise ProjectError(
                    f'"{wrong}" is {"not" if not found else "more than once"} a word of '
                    f'{sentence}, which says "{_said(transcript, first, last)}"'
                )
            shown = {**transcript.shown, found[0]: right.strip()}
            timeline, digest = self.timeline()
            was = story_of(timeline, self.transcripts(), digest)
            written(
                self._transcript_path(transcript.asset),
                transcript.model_copy(update={"shown": shown}).model_dump_json(indent=1),
            )
            story = self.project.directory / STORY
            # A story that was changed since it was written keeps the word as it was in it.
            anew = not story.exists() or story.read_text(encoding="utf-8") == was
            if anew:
                self._write_story(timeline, digest)
        return f'In {sentence}, "{wrong}" is shown as "{right.strip()}" from now on.' + (
            " The story was written anew." if anew else " The story was changed, and is as it was."
        )

    def _sentence(self, named: str) -> tuple[Transcript, int]:
        asset, _, number = named.rpartition(".")
        transcript = self.transcripts().get(asset)
        if transcript is None or not number.isdigit() or int(number) >= len(transcript.sentences):
            raise ProjectError(f"there is no sentence {named}")
        return transcript, int(number)

    # Layers

    def _placed(self, placing: Placing, timeline: Timeline) -> dict[str, object]:
        """Where a layer is laid, as the fields of a layer."""
        if placing.sentence is None:
            return {
                "anchor": None,
                "until": None,
                "at_frame": placing.at_frame or 0,
                "frames": placing.frames,
            }
        transcript, number = self._sentence(placing.sentence)
        entry = next(
            (
                one
                for one in timeline.main
                if one.asset == transcript.asset and one.sentence == number
            ),
            None,
        )
        if entry is None or not entry.kept:
            raise ProjectError(f"{placing.sentence} is not on the main line: nothing of it is kept")
        if placing.words is None:
            return {
                "anchor": (entry.asset, entry.kept[0]),
                "until": (entry.asset, entry.kept[-1]),
                "frames": None,
            }
        wanted = normal(placing.words)
        found: list[tuple[int, int]] = []
        for start in range(len(entry.kept)):
            said = ""
            for end in range(start, len(entry.kept)):
                said += normal(transcript.text(entry.kept[end]))
                if said == wanted:
                    found.append((entry.kept[start], entry.kept[end]))
                if len(said) >= len(wanted):
                    break
        if len(found) != 1:
            raise ProjectError(
                f'"{placing.words}" is {"not" if not found else "more than once"} among the '
                f"kept words of {placing.sentence}"
            )
        first, last = found[0]
        return {"anchor": (entry.asset, first), "until": (entry.asset, last), "frames": None}

    async def layered(
        self,
        action: Literal["add", "change", "remove"],
        *,
        layer: str | None = None,
        kind: Literal["picture", "sound", "title"] | None = None,
        asset: str | None = None,
        text: str | None = None,
        placing: Placing | None = None,
        shape: Shape | None = None,
    ) -> str:
        """Add a layer, change one, or take one away.

        Raises:
            ProjectError: What is asked cannot be.
        """
        async with self._lock:
            timeline, digest = self.timeline()
            layers = list(timeline.layers)
            old = next((one for one in layers if one.id == layer), None)
            if action != "add" and old is None:
                raise ProjectError(f"there is no layer {layer}")
            if action == "remove" and old is not None:
                layers.remove(old)
                self._keep(
                    timeline.model_copy(update={"layers": tuple(layers)}),
                    digest,
                    f"removed {old.id}",
                )
                return f"The layer {old.id} was taken away."
            changes: dict[str, object] = {} if shape is None else shape.changes()
            if placing is not None:
                changes.update(self._placed(placing, timeline))
            if asset is not None:
                self.project.asset(asset)
                changes["asset"] = asset
            if text is not None:
                changes["text"] = text
            try:
                if action == "add":
                    if kind is None:
                        raise ProjectError(
                            "a layer that is added needs a kind: picture, sound or title"
                        )
                    if kind != "title" and asset is None:
                        raise ProjectError(f"a {kind} needs a source")
                    if kind == "title" and not text:
                        raise ProjectError("a title needs its words")
                    taken = {one.id for one in layers}
                    number = 1
                    while f"{kind}-{number}" in taken:
                        number += 1
                    made = Layer.model_validate({"id": f"{kind}-{number}", "kind": kind, **changes})
                    if placing is None:
                        made = made.model_copy(update={"at_frame": 0})
                    layers.append(made)
                else:
                    was = old.model_dump() if old is not None else {}
                    made = Layer.model_validate({**was, **changes})
                    layers = [made if one is old else one for one in layers]
            except ValidationError as error:
                raise ProjectError(f"the layer cannot be so: {error.errors()[0]['msg']}") from None
            after = timeline.model_copy(update={"layers": tuple(layers)})
            plan = await self._plan(after, self.transcripts()) if after.main else None
            self._keep(after, digest, f"{action} {made.id}")
        where = ""
        if plan is not None and made.anchor is not None:
            start = render.timed(plan, self.transcripts(), made.anchor)
            where = (
                " It lies nowhere: its words are cut."
                if start is None
                else f" It begins at {_length(start)}."
            )
        done = "added" if action == "add" else "changed"
        return f"The layer {made.id} ({made.kind}) was {done}.{where}"

    async def styled(
        self,
        *,
        captions: bool | None = None,
        duck: bool | None = None,
        crossfade: tuple[str, int] | None = None,
    ) -> str:
        """Switch the captions or the ducking of sounds, or cross a sentence into the next one.

        Args:
            captions: Whether the words are shown under the film.
            duck: Whether sounds under the film are quieter while somebody speaks.
            crossfade: A sentence of the main line, and how many frames it crosses into the next.

        Raises:
            ProjectError: The sentence is not on the main line.
        """
        async with self._lock:
            timeline, digest = self.timeline()
            update: dict[str, object] = {}
            said: list[str] = []
            if captions is not None:
                update["captions"] = captions
                said.append(f"captions {'on' if captions else 'off'}")
            if duck is not None:
                update["duck"] = duck
                said.append(f"ducking {'on' if duck else 'off'}")
            if crossfade is not None:
                transcript, number = self._sentence(crossfade[0])
                place = next(
                    (
                        at
                        for at, one in enumerate(timeline.main)
                        if (one.asset, one.sentence) == (transcript.asset, number)
                    ),
                    None,
                )
                if place is None:
                    raise ProjectError(f"{crossfade[0]} is not on the main line")
                fades = {key: value for key, value in timeline.crossfades.items() if key != place}
                if crossfade[1] > 0:
                    fades[place] = crossfade[1]
                update["crossfades"] = fades
                said.append(f"{crossfade[0]} crosses {crossfade[1]} frames into the next sentence")
            if not update:
                return "Nothing was changed."
            self._keep(timeline.model_copy(update=update), digest, ", ".join(said))
        return f"Changed: {', '.join(said)}."

    # Making the film, and going back

    async def rendered(self, quality: Literal["preview", "final"] = "preview") -> str:
        """Make the film, and its captions.

        Raises:
            ProjectError: There is nothing to make.
            RenderError: What was made is not the film.
        """
        async with self._lock:
            timeline, _ = self.timeline()
            transcripts = self.transcripts()
            if not timeline.main:
                raise ProjectError(
                    "the main line is empty: transcribe a source, and keep some of it"
                )
            quiet = await self._silences(list(dict.fromkeys(one.asset for one in timeline.main)))
            name = "preview" if quality == "preview" else "film"
            film, captions, plan, made = await render.rendered(
                self.project, self.tools, timeline, transcripts, quiet, quality, name
            )
        lines = [f"The {quality} is {film}: {_length(plan.length_us)}."]
        if captions is not None:
            lines.append(f"The captions are {captions}.")
        if made.layers:
            lines.append(f"Left out, they lie nowhere: {', '.join(made.layers)}.")
        return "\n".join(lines)

    async def undone(self, *, redo: bool = False) -> str:
        """Take the last commit back, or do again what was taken back.

        Raises:
            ProjectError: There is nothing to take back, or to do again.
        """
        async with self._lock:
            _, digest = self.timeline()
            before, _ = self.timeline()
            after = self.project.redo(digest, "redo") if redo else self.project.undo(digest, "undo")
            self._follow(before, digest, self.timeline()[0], after)
        done = "Done again" if redo else "Taken back"
        return f"{done}. The story was written anew from the timeline."

    async def info(self) -> str:
        """Say how the project stands: its sources, the film, its layers and what was made."""
        manifest = self.project.manifest()
        timeline, digest = self.timeline()
        transcripts = self.transcripts()
        lines = [f"The project {self.project.directory}:"]
        if manifest.canvas is not None:
            canvas = manifest.canvas
            lines.append(
                f"The film is {canvas.width}x{canvas.height} at {canvas.rate} frames a second."
            )
        for asset in manifest.assets:
            said = transcripts.get(asset.id)
            heard = f", transcribed: {len(said.sentences)} sentences" if said else ""
            stale = (
                " (changed since, to be transcribed again)"
                if said and said.digest != asset.identity.digest
                else ""
            )
            lines.append(f"- {asset.id}: {asset.kind}, {_length(asset.duration_us)}{heard}{stale}")
        if timeline.main:
            plan = await self._plan(timeline, transcripts)
            lines.append(
                f"The main line has {len(timeline.main)} sentences and lasts "
                f"{_length(plan.length_us)}. Captions {'on' if timeline.captions else 'off'}, "
                f"ducking {'on' if timeline.duck else 'off'}."
            )
        for layer in timeline.layers:
            what = layer.text if layer.kind == "title" else layer.asset
            lines.append(f"- layer {layer.id}: {layer.kind} {what}")
        out = self.project.directory / "out"
        if out.is_dir():
            lines.extend(f"- made: {one}" for one in sorted(out.iterdir()))
        story = self.project.directory / STORY
        if story.exists():
            lines.append(f"The story is {story}, written from the timeline {digest[:12]}.")
        return "\n".join(lines)


def _said(transcript: Transcript, first: int, last: int) -> str:
    """What a sentence says, as the story shows it."""
    return line_of([(transcript.text(index), False) for index in range(first, last)])
