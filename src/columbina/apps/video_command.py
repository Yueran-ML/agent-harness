"""What ``columbina video`` does: a project of editing, worked on without a model.

The same operations as the tools of the model, :mod:`columbina.video.editing`, by hand::

    columbina video import talk raw/interview.mp4 raw/screen.mp4 music.mp3
    columbina video transcribe talk interview --model ~/models/faster-whisper-small
    (edit talk/story.md in any editor)
    columbina video apply talk
    columbina video render talk

A source is transcribed with faster-whisper and a model that ``--model`` or
``COLUMBINA_WHISPER_MODEL`` names, or with words that were written down beforehand, ``--words``.

``columbina video engine`` is of another kind: it says whether ``columbina-engine`` answers for
a project, and starts one without a window if none does, which ends by itself a while after.
"""

import argparse
import asyncio
import json
import os
import sys
from collections.abc import Callable
from pathlib import Path
from typing import cast

from columbina.video.editing import Editor, Placing, Shape
from columbina.video.engine import Engine, EngineError, not_found, program_of
from columbina.video.media import MediaUnavailableError, UnreadableMediaError, located
from columbina.video.project import ProjectError
from columbina.video.render import RenderError
from columbina.video.story import StoryError
from columbina.video.transcribe import Scripted, Transcriber, TranscriptionError, Whisper

__all__ = ["ENGINE", "VIDEO", "arguments", "run"]

VIDEO = "video"
ENGINE = "engine"

_FAILURES = (
    ProjectError,
    StoryError,
    MediaUnavailableError,
    UnreadableMediaError,
    TranscriptionError,
    RenderError,
)


def arguments(video: argparse.ArgumentParser) -> None:
    """Add what ``columbina video`` takes: what to do, with which project."""
    whats = video.add_subparsers(dest="what", metavar="WHAT", required=True)

    def what(name: str, said: str) -> argparse.ArgumentParser:
        one = whats.add_parser(name, help=said)
        one.add_argument("project", metavar="DIR", help="the directory of the project")
        return one

    imported = what("import", "take files in as sources")
    imported.add_argument("paths", nargs="+", metavar="FILE")
    transcribing = what("transcribe", "write down what was said in a source")
    transcribing.add_argument("source", metavar="SOURCE", help="the id of the source")
    transcribing.add_argument("--language", help="its language, such as zh or en")
    transcribing.add_argument("--again", action="store_true", help="although it was transcribed")
    transcribing.add_argument("--model", metavar="DIR", help="a model of faster-whisper")
    transcribing.add_argument("--python", help="the Python that has faster-whisper")
    transcribing.add_argument("--words", metavar="FILE", help="its words, written down before")
    applying = what("apply", "commit the story")
    applying.add_argument("--dry", action="store_true", help="say what it would do, and do nothing")
    applying.add_argument("--pause-most-ms", type=int, help="how long a pause lasts at most")
    applying.add_argument("--fresh", action="store_true", help="write the story anew instead")
    correcting = what("correct", "show a word that was heard wrong as it was said")
    correcting.add_argument("sentence", metavar="SENTENCE", help="such as talk.3")
    correcting.add_argument("wrong", metavar="WRONG")
    correcting.add_argument("right", metavar="RIGHT")
    overlaying = what(
        "overlay", "lay pictures, sounds and titles over the film, or change its style"
    )
    overlaying.add_argument("action", choices=["add", "change", "remove", "style"])
    overlaying.add_argument("--layer", help="the layer to change or take away")
    overlaying.add_argument("--kind", choices=["picture", "sound", "title"])
    overlaying.add_argument("--source", help="the source that it shows or plays")
    overlaying.add_argument("--text", help="the words of a title")
    overlaying.add_argument("--sentence", help="the sentence that it lies at, such as talk.3")
    overlaying.add_argument("--words", help="words of that sentence: it lasts while they are said")
    overlaying.add_argument("--at-frame", type=int, help="the frame where it begins")
    overlaying.add_argument("--frames", type=int, help="how many frames it lasts")
    overlaying.add_argument(
        "--position",
        choices=["full", "top-left", "top-right", "bottom-left", "bottom-right", "center"],
    )
    for option in ("--scale", "--opacity", "--volume", "--speed"):
        overlaying.add_argument(option, type=float)
    overlaying.add_argument("--fade-ms", type=int)
    overlaying.add_argument("--source-ms", type=int, help="where in its source it begins")
    overlaying.add_argument("--captions", choices=["on", "off"])
    overlaying.add_argument("--duck", choices=["on", "off"])
    overlaying.add_argument("--crossfade", type=int, metavar="FRAMES", help="with --sentence")
    rendering = what("render", "make a preview, or the film with --final")
    rendering.add_argument("--final", action="store_true", help="the film, and not a preview")
    undoing = what("undo", "take the last commit back")
    undoing.add_argument("--redo", action="store_true", help="do again what was taken back")
    what("info", "say how the project stands")
    engine = whats.add_parser(
        ENGINE, help="say whether the engine answers for a project, starting one if none does"
    )
    engine.add_argument(
        "--project",
        metavar="DIR",
        required=True,
        help="the directory of the project, where the engine keeps its lock and its logs",
    )
    engine.add_argument(
        "--json", action="store_true", help="answer in one document of JSON, for another program"
    )


def _transcriber(args: argparse.Namespace) -> Transcriber | None:
    if getattr(args, "words", None) and args.what == "transcribe":
        return Scripted(Path(args.words))
    model = getattr(args, "model", None) or os.environ.get("COLUMBINA_WHISPER_MODEL")
    if model:
        return Whisper(Path(model), getattr(args, "python", None) or sys.executable)
    return None


def _switch(said: str | None) -> bool | None:
    return None if said is None else said == "on"


async def _done(args: argparse.Namespace, cwd: Path, directory: Path) -> str:
    tools = await located(os.environ.get("PATH", ""), [], cwd=cwd)
    editor = Editor(directory, tools, _transcriber(args))
    if args.what == "import":
        return await editor.imported([Path(path) for path in args.paths])
    if args.what == "transcribe":
        return await editor.transcribed(args.source, args.language, again=args.again)
    if args.what == "apply":
        return await editor.applied(
            dry=args.dry, pause_most_ms=args.pause_most_ms, fresh=args.fresh
        )
    if args.what == "correct":
        return await editor.corrected(args.sentence, args.wrong, args.right)
    if args.what == "overlay":
        if args.action == "style":
            crossfade = None if args.crossfade is None else (str(args.sentence), args.crossfade)
            return await editor.styled(
                captions=_switch(args.captions), duck=_switch(args.duck), crossfade=crossfade
            )
        placing = None
        if args.sentence is not None or args.at_frame is not None or args.frames is not None:
            placing = Placing(args.sentence, args.words, args.at_frame, args.frames)
        shape = Shape(
            args.position,
            args.scale,
            args.opacity,
            args.volume,
            args.fade_ms,
            args.speed,
            args.source_ms,
        )
        return await editor.layered(
            args.action,
            layer=args.layer,
            kind=args.kind,
            asset=args.source,
            text=args.text,
            placing=placing,
            shape=shape,
        )
    if args.what == "render":
        return await editor.rendered("final" if args.final else "preview")
    if args.what == "undo":
        return await editor.undone(redo=args.redo)
    return await editor.info()


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


def run(args: argparse.Namespace, say: Callable[[str], None]) -> int:
    """Carry out ``columbina video``, and return its exit code."""
    if args.what == ENGINE:
        return _engine(args, say)
    try:
        said = asyncio.run(_done(args, Path.cwd(), Path(args.project).resolve()))
    except _FAILURES as error:
        say(f"columbina: {error}")
        return 2
    say(said)
    return 0
