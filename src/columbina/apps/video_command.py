"""CLI editing through the shared Studio engine, with the same receipts as agent tools.

Import does not create clips. Create a timeline and place media with ``edit --file FILE``;
``story`` writes its editable dialogue, ``apply`` cuts it, and ``render`` exports the timeline.
Legacy projects are read only by ``migrate``. No Python path renders films independently.
"""

import argparse
import asyncio
import json
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any, cast

from columbina.video.editing import Editor, safe_name
from columbina.video.engine import Engine, EngineError, not_found, program_of
from columbina.video.media import MediaUnavailableError, UnreadableMediaError, located
from columbina.video.project import ProjectError
from columbina.video.story import StoryError
from columbina.video.transcribe import Scripted, Transcriber, TranscriptionError, Whisper
from columbina.video.words import WordsError, kept_words

__all__ = ["ENGINE", "VIDEO", "arguments", "run"]
VIDEO = "video"
ENGINE = "engine"
_FAILURES = (
    ProjectError,
    StoryError,
    MediaUnavailableError,
    UnreadableMediaError,
    TranscriptionError,
    WordsError,
    EngineError,
    OSError,
    ValueError,
)


def _non_negative(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be zero or greater")
    return parsed


def arguments(parser: argparse.ArgumentParser) -> None:
    """The operations of Studio, using engine ids returned by import, edit and info."""
    whats = parser.add_subparsers(dest="what", metavar="WHAT", required=True)

    def what(name: str, said: str, *, timeline: bool = False) -> argparse.ArgumentParser:
        one = whats.add_parser(name, help=said)
        one.add_argument("project", metavar="DIR", help="the Studio project directory")
        if timeline:
            one.add_argument("--timeline", help="timeline id; optional when there is only one")
        return one

    imported = what("import", "import files into the media pool, without placing clips")
    imported.add_argument("paths", nargs="+", metavar="FILE")
    imported.add_argument("--bin", help="destination media bin id")
    editing = what("edit", "apply a JSON array of edits as one undo step", timeline=True)
    operations = editing.add_mutually_exclusive_group(required=True)
    operations.add_argument("--edits", help="the JSON array of engine operations and edits")
    operations.add_argument("--file", help="a UTF-8 file containing that array")
    telling = what("story", "write the editable dialogue of a timeline", timeline=True)
    telling.add_argument("--track", help="dialogue track id; defaults to A1")
    transcribing = what("transcribe", "analyse a media source's spoken words", timeline=True)
    transcribing.add_argument("media", help="media id returned by import")
    transcribing.add_argument("--subtitles", help="subtitle track id to populate")
    transcribing.add_argument("--language")
    transcribing.add_argument("--again", action="store_true")
    transcribing.add_argument("--model", help="local faster-whisper model directory")
    transcribing.add_argument("--python", help="Python interpreter with faster-whisper")
    transcribing.add_argument("--words", help="previously timed words in a JSON file")
    applying = what("apply", "ripple-cut the edited story", timeline=True)
    applying.add_argument("--dry", action="store_true")
    applying.add_argument("--pause-most-ms", type=_non_negative)
    applying.add_argument("--fresh", action="store_true", help="write the story anew instead")
    correcting = what("correct", "correct displayed text, keeping the original recording")
    correcting.add_argument("media")
    correcting.add_argument("sentence", type=int, help="sentence index, from zero")
    correcting.add_argument("wrong")
    correcting.add_argument("right")
    rendering = what("render", "export a timeline through the engine", timeline=True)
    rendering.add_argument("--output", help="output path; defaults to out/<timeline name>")
    rendering.add_argument("--codec", choices=["h264", "hevc", "prores"], default="h264")
    rendering.add_argument("--quality", choices=["high", "medium", "small"], default="high")
    rendering.add_argument("--no-subtitles", action="store_true", help="omit the SRT sidecars")
    rendering.add_argument("--replace", action="store_true", help="replace existing output files")
    undoing = what("undo", "undo the shared project's last transaction")
    undoing.add_argument("--redo", action="store_true")
    what("info", "list media, timelines, tracks and clips", timeline=True)
    engine = whats.add_parser(ENGINE, help="check the shared engine connection")
    engine.add_argument("--project", metavar="DIR", required=True)
    engine.add_argument("--json", action="store_true")
    migrating = whats.add_parser("migrate", help="convert old films to Studio timelines")
    migrating.add_argument("--workspace", metavar="DIR", required=True)
    migrating.add_argument("--json", action="store_true")


def _transcriber(args: argparse.Namespace) -> Transcriber | None:
    if args.words:
        return Scripted(Path(args.words).resolve())
    model = args.model or os.environ.get("COLUMBINA_WHISPER_MODEL")
    return Whisper(Path(model).resolve(), args.python or sys.executable) if model else None


async def _done(args: argparse.Namespace, cwd: Path, directory: Path) -> str:
    program = program_of(None, os.environ.get("PATH", ""), [cwd])
    if program is None:
        raise ProjectError(not_found(None))
    async with await Engine.attach(directory, program=program, client="cli") as engine:
        editor = Editor(engine)
        if args.what == "import":
            paths = await asyncio.to_thread(lambda: [Path(p).resolve() for p in args.paths])
            return await editor.imported(paths, args.bin)
        if args.what == "edit":
            raw = (
                await asyncio.to_thread(Path(args.file).read_text, encoding="utf-8")
                if args.file
                else args.edits
            )
            batch: Any = json.loads(raw)
            if (
                not isinstance(batch, list)
                or not batch
                or any(not isinstance(op, dict) for op in cast("list[object]", batch))
            ):
                raise ProjectError("edits must be a non-empty JSON array of objects")
            return await editor.edited(cast("list[dict[str, Any]]", batch), args.timeline)
        if args.what == "story" or (args.what == "apply" and args.fresh):
            return await editor.told(args.timeline, getattr(args, "track", None))
        if args.what == "transcribe":
            editor.transcriber = _transcriber(args)
            _, project = await engine.project()
            item = next((m for m in project["media"] if m["id"] == args.media), None)
            cached = (
                item is not None
                and not args.again
                and kept_words(directory, item["identity"]["digest"]) is not None
            )
            tools = (
                await located(os.environ.get("PATH", ""), [], cwd=cwd)
                if not cached and item is not None and editor.transcriber is not None
                else None
            )
            return await editor.transcribed(
                args.media,
                tools,
                language=args.language,
                again=args.again,
                timeline=args.timeline,
                subtitles=args.subtitles,
            )
        if args.what == "apply":
            return await editor.applied(
                args.timeline, dry=args.dry, pause_most_ms=args.pause_most_ms
            )
        if args.what == "correct":
            return await editor.corrected(args.media, args.sentence, args.wrong, args.right)
        if args.what == "render":
            target = await editor.timeline(args.timeline)
            extension = ".mov" if args.codec == "prores" else ".mp4"
            path = (
                Path(args.output)
                if args.output
                else cwd / "out" / (safe_name(target["name"]) + extension)
            )
            return await editor.rendered(
                target["id"],
                path.resolve(),
                codec=args.codec,
                quality=args.quality,
                subtitles=not args.no_subtitles,
                replace=args.replace,
            )
        if args.what == "undo":
            return await editor.undone(redo=args.redo)
        return await editor.info(args.timeline)


async def _answered(directory: Path, program: str) -> dict[str, object]:
    """Ask the engine of a project what it is, and how the project stands."""
    async with await Engine.attach(directory, program=program, client="cli") as engine:
        revision, project = await engine.project()
    media, timelines = (
        len(cast("list[object]", things)) if isinstance(things, list) else 0
        for things in (project.get("media"), project.get("timelines"))
    )
    return {
        "engine": engine.hello.get("engine"),
        "protocol": engine.hello.get("protocol"),
        "pid": engine.hello.get("pid"),
        "project": engine.hello.get("project"),
        "revision": revision,
        "media": media,
        "timelines": timelines,
    }


def _engine(args: argparse.Namespace, say: Callable[[str], None]) -> int:
    program = program_of(None, os.environ.get("PATH", ""), [Path.cwd()])
    if program is None:
        say(f"columbina: {not_found(None)}")
        return 2
    try:
        told = asyncio.run(_answered(Path(args.project).resolve(), program))
    except EngineError as error:
        say(f"columbina: {error}")
        return 2
    if args.json:
        say(json.dumps(told, ensure_ascii=False))
        return 0
    timelines = told["timelines"]
    say(
        f"The engine {told['engine']} answers in protocol {told['protocol']}, as process "
        f"{told['pid']}, for {told['project']}: revision {told['revision']}, "
        f"{told['media']} media, {timelines} timeline{'' if timelines == 1 else 's'}."
    )
    return 0


async def _migrated(workspace: Path, program: str) -> list[dict[str, object]]:
    from columbina.video.migrate import migrate

    async with await Engine.attach(
        workspace / ".columbina/studio", program=program, client="cli"
    ) as engine:
        return await migrate(workspace, engine)


def _migrate(args: argparse.Namespace, say: Callable[[str], None]) -> int:
    workspace = Path(args.workspace).resolve()
    program = program_of(None, os.environ.get("PATH", ""), [Path.cwd(), workspace])
    try:
        if not workspace.is_dir():
            raise ProjectError(f"workspace does not exist: {workspace}")
        if program is None:
            raise ProjectError(not_found(None))
        reports = asyncio.run(_migrated(workspace, program))
    except _FAILURES as error:
        if args.json:
            say(json.dumps({"error": str(error)}, ensure_ascii=False))
        else:
            say(f"columbina: {error}")
        return 2
    if args.json:
        say(json.dumps({"migrations": reports}, ensure_ascii=False))
    elif not reports:
        say("No legacy projects to migrate.")
    else:
        for report in reports:
            say(f"{report['source']} -> {report['timeline']}; original files retained.")
            for issue in [
                *cast("list[str]", report["dropped"]),
                *cast("list[str]", report.get("warnings", [])),
            ]:
                say(f"  {issue}")
    return 0


def run(args: argparse.Namespace, say: Callable[[str], None]) -> int:
    """Carry out ``columbina video``, returning a structured receipt or an explained error."""
    if args.what == ENGINE:
        return _engine(args, say)
    if args.what == "migrate":
        return _migrate(args, say)
    try:
        said = asyncio.run(_done(args, Path.cwd(), Path(args.project).resolve()))
    except _FAILURES as error:
        say(f"columbina: {error}")
        return 2
    say(said)
    return 0
