"""A project of editing: a directory of its own, with what it knows of its sources and its timeline.

**The directory.** ``project.json`` lists the sources and the frame of the film, ``timeline.json``
is the timeline as it was committed last, ``history/`` holds the timelines that an undo or a
redo takes back, and ``cache/`` holds what was worked out of the sources and can be worked out
again: their sound for a transcriber, and where they are silent. A source is listed by its path
and is never copied. Every file is written beside itself first and then put in its place.

**A source** gets an id from its name, which never changes: the story and what is laid over
the film point to it by that. Of each source the project keeps an identity: its size, when it
was changed, and a digest of its size, its first and its last 64 KiB. Before a source is used it
is looked at again. A file that was only touched is used, and one that was replaced is said to
have changed: what is known of the other file is not used for it.

**The frame** of the film is that of the first source with a picture: as it is shown, turned,
and at its rate of frames.

**The timeline** is text to this module. A commit names the digest of the timeline that it was
made from, and is refused if the timeline is another one by now: something else changed it.
Each commit puts the timeline before it on a stack to undo, and empties the stack to redo; an
undo and a redo move one timeline between the two. The stack to undo keeps the last 200.
"""

import asyncio
import hashlib
import json
import os
import re
import secrets
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from pydantic import ValidationError

from columbina.data import Data
from columbina.video import media
from columbina.video.media import AudioFacts, Silence, Tools, VideoFacts

__all__ = [
    "CACHE",
    "HISTORY",
    "HISTORY_MOST",
    "MANIFEST",
    "TIMELINE",
    "Asset",
    "Canvas",
    "ChangedSourceError",
    "Identity",
    "Imported",
    "Manifest",
    "Project",
    "ProjectError",
    "StaleError",
    "digest_of",
    "identity_of",
    "named",
    "written",
]

MANIFEST = "project.json"
TIMELINE = "timeline.json"
HISTORY = "history"
CACHE = "cache"
HISTORY_MOST = 200
"""How many timelines the stack to undo keeps."""

_EDGE = 64 * 1024
"""How much of the beginning and of the end of a source goes into its digest."""

_LONGEST_ID = 40


class ProjectError(Exception):
    """Something of a project cannot be done. The words say why, for a person."""


class StaleError(ProjectError):
    """The timeline is not the one that a change was made from."""


class ChangedSourceError(ProjectError):
    """A source is not the file that it was when it was taken in."""


class Identity(Data):
    """What tells a source from another file of the same name.

    Attributes:
        size: Its size in bytes.
        modified_ns: When it was changed last.
        digest: SHA-256 of its size, its first and its last 64 KiB.
    """

    size: int
    modified_ns: int
    digest: str


class Asset(Data):
    """A source of the project.

    Attributes:
        id: Its id, from its name.
        path: Where it is, whole, as it was named.
        kind: ``video``, ``audio`` or ``image``.
        format: Its format, as ffprobe names it.
        duration_us: How long it lasts; 0 for a picture.
        video: Its picture.
        audio: Its sound.
        identity: What tells it from another file.
    """

    id: str
    path: str
    kind: Literal["video", "audio", "image"]
    format: str
    duration_us: int
    video: VideoFacts | None = None
    audio: AudioFacts | None = None
    identity: Identity


class Canvas(Data):
    """The frame of the film.

    Attributes:
        width: Its width.
        height: Its height.
        rate: Its rate of frames, as a fraction.
    """

    width: int
    height: int
    rate: str


class Manifest(Data):
    """What a project knows of its sources.

    Attributes:
        version: The version of this file.
        canvas: The frame of the film, once a source with a picture was taken in.
        assets: The sources, in the order in which they were taken in.
    """

    version: Literal[1] = 1
    canvas: Canvas | None = None
    assets: tuple[Asset, ...] = ()


@dataclass(frozen=True)
class Imported:
    """What taking files in did.

    Attributes:
        added: The sources that are new.
        changed: The sources that were taken in before, and are other files now.
        same: The sources that were taken in before, as they are.
        warnings: What is not done for some of them, for a person.
    """

    added: tuple[Asset, ...] = ()
    changed: tuple[Asset, ...] = ()
    same: tuple[Asset, ...] = ()
    warnings: tuple[str, ...] = ()


def digest_of(text: str) -> str:
    """Return the digest of a text, as a change names the timeline that it was made from."""
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def identity_of(path: Path) -> Identity:
    """Work out the identity of a file. This blocks.

    Raises:
        OSError: It cannot be read.
    """
    with path.open("rb") as file:
        size = os.fstat(file.fileno()).st_size
        stamp = os.fstat(file.fileno()).st_mtime_ns
        digest = hashlib.sha256(str(size).encode())
        digest.update(file.read(_EDGE))
        if size > _EDGE:
            file.seek(max(_EDGE, size - _EDGE))
            digest.update(file.read(_EDGE))
    return Identity(size=size, modified_ns=stamp, digest=digest.hexdigest())


def named(name: str, taken: Sequence[str]) -> str:
    """Return the id of a source for its file name, unlike every id that is taken."""
    stem = Path(name).stem.lower()
    base = re.sub(r"[^0-9a-z一-鿿]+", "-", stem).strip("-")[:_LONGEST_ID].strip("-") or "source"
    found, number = base, 1
    while found in taken:
        number += 1
        found = f"{base}-{number}"
    return found


def written(path: Path, text: str) -> None:
    """Write a file beside itself first, and put it in its place. This blocks.

    Raises:
        ProjectError: It could not be written, or put in its place.
    """
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(4)}")
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary.write_text(text, encoding="utf-8", newline="\n")
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise ProjectError(f"{path} could not be written: {error}") from None


def _stack(place: Path) -> list[Path]:
    """The timelines of a stack, the oldest first."""
    return sorted(place.glob("[0-9]*.json")) if place.is_dir() else []


class Project:
    """A project of editing in a directory."""

    def __init__(self, directory: Path) -> None:
        self.directory = directory

    @property
    def cache(self) -> Path:
        """Where what can be worked out again of the sources is kept."""
        return self.directory / CACHE

    def manifest(self) -> Manifest:
        """What the project knows of its sources. A project that has none knows nothing.

        Raises:
            ProjectError: Its file cannot be read, or is of a version that is not known.
        """
        path = self.directory / MANIFEST
        if not path.exists():
            return Manifest()
        try:
            return Manifest.model_validate_json(path.read_bytes())
        except (OSError, ValidationError) as error:
            raise ProjectError(f"{path} is not what this program writes: {error}") from None

    def asset(self, key: str) -> Asset:
        """Return the source of an id.

        Raises:
            ProjectError: There is none.
        """
        for one in self.manifest().assets:
            if one.id == key:
                return one
        raise ProjectError(f"the project has no source {key}")

    def _keep(self, manifest: Manifest) -> None:
        written(self.directory / MANIFEST, manifest.model_dump_json(indent=2) + "\n")

    async def imported(self, paths: Sequence[Path], tools: Tools) -> Imported:
        """Take files in as sources. A file that was taken in before is looked at again.

        Raises:
            MediaUnavailableError: ffprobe cannot be run.
            ProjectError: A file cannot be read, or is no source; nothing is taken in then.
        """
        manifest = self.manifest()
        assets = list(manifest.assets)
        added: list[Asset] = []
        changed: list[Asset] = []
        same: list[Asset] = []
        warnings: list[str] = []
        # The names that the user gave, whole: a link on them is not followed.
        wholes = await asyncio.to_thread(lambda: [Path(given).absolute() for given in paths])
        for path in wholes:
            try:
                identity = await asyncio.to_thread(identity_of, path)
            except OSError as error:
                raise ProjectError(f"{path} cannot be read: {error}") from None
            known = next((at for at, one in enumerate(assets) if one.path == str(path)), None)
            if known is not None and assets[known].identity.digest == identity.digest:
                assets[known] = assets[known].model_copy(update={"identity": identity})
                same.append(assets[known])
                continue
            try:
                facts = await media.probed(tools, path, place=self.cache)
            except media.UnreadableMediaError as error:
                raise ProjectError(f"{path} is no source that can be edited: {error}") from None
            warnings.extend(f"{path.name}: {warning}" for warning in facts.warnings)
            # A source that was taken in before keeps its id.
            key = (
                assets[known].id
                if known is not None
                else named(path.name, [one.id for one in assets])
            )
            asset = Asset(
                id=key,
                path=str(path),
                kind=facts.kind,
                format=facts.format,
                duration_us=facts.duration_us,
                video=facts.video,
                audio=facts.audio,
                identity=identity,
            )
            if known is not None:
                assets[known] = asset
                changed.append(asset)
            else:
                assets.append(asset)
                added.append(asset)
        canvas = manifest.canvas
        if canvas is None:
            first = next((one for one in assets if one.kind == "video" and one.video), None)
            if first is not None and first.video is not None:
                canvas = Canvas(
                    width=first.video.width, height=first.video.height, rate=first.video.rate
                )
        await asyncio.to_thread(self._keep, Manifest(canvas=canvas, assets=tuple(assets)))
        return Imported(tuple(added), tuple(changed), tuple(same), tuple(warnings))

    async def checked(self, asset: Asset) -> Asset:
        """See that a source is still the file that it was, and return it as it is now.

        A file that was only touched is the same, and the time is written down.

        Raises:
            ChangedSourceError: It is gone, or another file by now.
        """
        path = Path(asset.path)
        try:
            stamp = await asyncio.to_thread(path.stat)
        except OSError as error:
            raise ChangedSourceError(
                f"the source {asset.id} at {path} is not there: {error}"
            ) from None
        if stamp.st_size == asset.identity.size and stamp.st_mtime_ns == asset.identity.modified_ns:
            return asset
        try:
            identity = await asyncio.to_thread(identity_of, path)
        except OSError as error:
            raise ChangedSourceError(
                f"the source {asset.id} at {path} cannot be read: {error}"
            ) from None
        if identity.digest != asset.identity.digest:
            raise ChangedSourceError(
                f"the source {asset.id} at {path} is another file than the one that was taken in. "
                "Take it in again to use it as it is now"
            )
        touched = asset.model_copy(update={"identity": identity})
        manifest = self.manifest()
        assets = tuple(touched if one.id == asset.id else one for one in manifest.assets)
        await asyncio.to_thread(self._keep, manifest.model_copy(update={"assets": assets}))
        return touched

    def _cached(self, asset: Asset, what: str) -> Path:
        return self.cache / f"{asset.id}-{asset.identity.digest[:12]}-{what}"

    async def cached_audio(self, asset: Asset, tools: Tools) -> Path:
        """Return the sound of a source as a transcriber reads it, taken out once.

        Raises:
            ChangedSourceError: The source is another file by now.
            ProjectError: It has no sound.
            MediaUnavailableError: ffmpeg cannot be run.
            UnreadableMediaError: The sound cannot be taken out.
        """
        asset = await self.checked(asset)
        if asset.audio is None:
            raise ProjectError(f"the source {asset.id} has no sound")
        target = self._cached(asset, "16k.wav")
        if not target.exists():
            self.cache.mkdir(parents=True, exist_ok=True)
            await media.audio(tools, Path(asset.path), target)
        return target

    async def cached_silences(
        self, asset: Asset, tools: Tools, *, noise_db: float = -35.0, shortest_us: int = 300_000
    ) -> tuple[Silence, ...]:
        """Return where a source is silent, found once for each threshold.

        Raises:
            ChangedSourceError: The source is another file by now.
            ProjectError: It has no sound.
            MediaUnavailableError: ffmpeg cannot be run.
            UnreadableMediaError: The sound cannot be read.
        """
        asset = await self.checked(asset)
        if asset.audio is None:
            raise ProjectError(f"the source {asset.id} has no sound")
        target = self._cached(asset, f"silences-{noise_db:g}-{shortest_us}.json")
        if target.exists():
            kept = json.loads(target.read_text(encoding="utf-8"))
            return tuple(Silence(start, end) for start, end in kept)
        found = await media.silences(
            tools,
            Path(asset.path),
            place=self.cache,
            duration_us=asset.duration_us,
            noise_db=noise_db,
            shortest_us=shortest_us,
        )
        written(target, json.dumps([[one.start_us, one.end_us] for one in found]))
        return found

    # The timeline

    def timeline(self) -> tuple[str, str]:
        """Return the timeline as it was committed last, and its digest. Empty before the first.

        Raises:
            ProjectError: It cannot be read.
        """
        path = self.directory / TIMELINE
        try:
            text = path.read_text(encoding="utf-8") if path.exists() else ""
        except (OSError, UnicodeDecodeError) as error:
            raise ProjectError(f"{path} cannot be read: {error}") from None
        return text, digest_of(text)

    def _fresh(self, base: str) -> str:
        text, digest = self.timeline()
        if digest != base:
            raise StaleError(
                "the timeline was changed since what was changed was read. Read it again, and "
                "make the change on what it is now"
            )
        return text

    def _push(self, stack: str, text: str, note: str) -> None:
        place = self.directory / HISTORY / stack
        kept = _stack(place)
        number = int(kept[-1].stem) + 1 if kept else 1
        record = {"timeline": text, "note": note, "at": round(time.time() * 1000)}
        written(place / f"{number:06d}.json", json.dumps(record, ensure_ascii=False))
        if stack == "undo":
            for old in _stack(place)[:-HISTORY_MOST]:
                old.unlink()

    def _pop(self, stack: str) -> str:
        kept = _stack(self.directory / HISTORY / stack)
        if not kept:
            said = "undo" if stack == "undo" else "redo"
            raise ProjectError(f"there is nothing to {said}")
        text = str(json.loads(kept[-1].read_text(encoding="utf-8"))["timeline"])
        kept[-1].unlink()
        return text

    def commit(self, text: str, base: str, note: str) -> str:
        """Make a text the timeline, and return its digest. This blocks.

        Args:
            text: The timeline.
            base: The digest of the timeline that it was made from.
            note: What it was, for the history.

        Raises:
            StaleError: The timeline is another one by now.
            ProjectError: It could not be written.
        """
        before = self._fresh(base)
        self._push("undo", before, note)
        for old in _stack(self.directory / HISTORY / "redo"):
            old.unlink()
        written(self.directory / TIMELINE, text)
        return digest_of(text)

    def undo(self, base: str, note: str) -> str:
        """Take the timeline back to the one before the last commit, and return its digest.

        Raises:
            StaleError: The timeline is another one by now.
            ProjectError: There is nothing to undo.
        """
        now = self._fresh(base)
        before = self._pop("undo")
        self._push("redo", now, note)
        written(self.directory / TIMELINE, before)
        return digest_of(before)

    def redo(self, base: str, note: str) -> str:
        """Take the timeline forward again, after an undo, and return its digest.

        Raises:
            StaleError: The timeline is another one by now.
            ProjectError: There is nothing to redo.
        """
        now = self._fresh(base)
        after = self._pop("redo")
        self._push("undo", now, note)
        written(self.directory / TIMELINE, after)
        return digest_of(after)
