"""The film, made by ffmpeg: what the timeline is compiled into, and the run that makes it.

**The plan.** The stretches of the main line are put on frames, one after the other, less what
two of them cross into each other. A word of the main line has a time in the film where the
stretch that holds it is played; a layer begins and ends at such times, or at frames that it
names. A layer whose word was cut lies nowhere, and is left out.

**The graph.** Each stretch of the main line is taken out of its source, made as large as the
film and its sound of one rate, and the stretches are joined. A source is read once for the
stretches that go forward in it: a stretch that goes back in it opens it again, from there, so
that nothing that is decoded waits for long. Pictures are laid over the film where their layers
say, sounds are mixed under it and made quieter while somebody speaks, and titles and captions
are drawn last, from a file of styles.

**Safety.** No path is ever written into the graph. The graph is a file of the run, the styles
are another, both by names of this program's own, and ffmpeg runs in the directory of the run.
Every source is opened by the protocol of files, of the formats that are taken.

**The film** is written in the directory of the run, looked at, and only then moved to ``out/``:
it has to last as long as the plan, give or take two frames, and hold a picture and a sound.
"""

import sys
from collections.abc import Sequence
from dataclasses import dataclass, field
from fractions import Fraction
from pathlib import Path
from typing import Literal

from columbina.video import media
from columbina.video.media import Silence, Tools
from columbina.video.project import Asset, Canvas, Manifest, Project
from columbina.video.story import line_of
from columbina.video.timeline import Cut, Layer, Timeline, Transcript, cuts

__all__ = [
    "Compiled",
    "Cue",
    "Plan",
    "RenderError",
    "ass",
    "captions",
    "compiled",
    "planned",
    "rendered",
    "srt",
    "timed",
]

Quality = Literal["preview", "final"]

_AUDIO_RATE = 48_000
_PREVIEW_HEIGHT = 360
_CAPTION_CHARACTERS = 42
"""How many characters a caption holds at most, or half as many of Chinese or Japanese."""

_RENDER_SECONDS = 6 * 60 * 60.0


class RenderError(Exception):
    """The film could not be made. The words say why."""


@dataclass(frozen=True)
class Plan:
    """The stretches of the main line on the frames of the film.

    Attributes:
        rate: The rate of the frames.
        cuts: The stretches, in order.
        starts: Where each begins in the film, in frames.
        lengths: How many frames each lasts.
        fades: How many frames each crosses into the one after it.
        frames: How many frames the film lasts.
    """

    rate: Fraction
    cuts: tuple[Cut, ...]
    starts: tuple[int, ...]
    lengths: tuple[int, ...]
    fades: tuple[int, ...]
    frames: int

    def us(self, frame: int) -> int:
        """The time of a frame, in microseconds."""
        return round(Fraction(frame) / self.rate * 1_000_000)

    def frame(self, us: int) -> int:
        """The frame that a time falls in."""
        return round(Fraction(us, 1_000_000) * self.rate)

    @property
    def length_us(self) -> int:
        """How long the film lasts."""
        return self.us(self.frames)


@dataclass(frozen=True)
class Cue:
    """A line of words that is shown for a while: a caption or a title.

    Attributes:
        start_us: When it is shown, in the film.
        end_us: When it is gone.
        text: The words.
        style: ``caption``, or the position of a title.
    """

    start_us: int
    end_us: int
    text: str
    style: str = "caption"


@dataclass(frozen=True)
class Compiled:
    """What ffmpeg is run with to make a film.

    Attributes:
        inputs: The arguments that open the sources, in order.
        graph: The filter graph, which is written into a file of the run.
        maps: The arguments that pick the outputs of the graph.
        styles: The file of styles that the graph draws from, if it draws any.
        layers: The layers that lie nowhere, by their ids.
    """

    inputs: list[str]
    graph: str
    maps: list[str]
    styles: str | None = None
    layers: tuple[str, ...] = field(default=())


def _rate(canvas: Canvas | None) -> Fraction:
    if canvas is None:
        return Fraction(1000)
    top, _, bottom = canvas.rate.partition("/")
    return Fraction(int(top), int(bottom))


def planned(
    timeline: Timeline,
    transcripts: dict[str, Transcript],
    silences: dict[str, tuple[Silence, ...]],
    lengths: dict[str, int],
    canvas: Canvas | None,
) -> Plan:
    """Put the stretches of the main line on the frames of the film."""
    rate = _rate(canvas)
    played = cuts(timeline, transcripts, silences, lengths)
    sizes = [
        max(
            1,
            round(Fraction(cut.end_us, 1_000_000) * rate)
            - round(Fraction(cut.start_us, 1_000_000) * rate),
        )
        for cut in played
    ]
    starts: list[int] = []
    fades: list[int] = []
    at = 0
    for place, cut in enumerate(played):
        fade = 0
        # A sentence crosses into the next one, and no stretch into more than the whole of either.
        if place + 1 < len(played) and played[place + 1].entry != cut.entry:
            fade = max(
                0,
                min(timeline.crossfades.get(cut.entry, 0), sizes[place] - 1, sizes[place + 1] - 1),
            )
        starts.append(at)
        fades.append(fade)
        at += sizes[place] - fade
    return Plan(rate, tuple(played), tuple(starts), tuple(sizes), tuple(fades), at)


def timed(
    plan: Plan, transcripts: dict[str, Transcript], word: tuple[str, int], end: bool = False
) -> int | None:
    """When a word of the main line begins in the film, or ends; ``None`` if it was cut."""
    asset, index = word
    transcript = transcripts.get(asset)
    if transcript is None or index >= len(transcript.words):
        return None
    said = transcript.words[index]
    at = said.end_us if end else said.start_us
    for cut, start in zip(plan.cuts, plan.starts, strict=True):
        if cut.asset == asset and cut.start_us <= at <= cut.end_us:
            return plan.us(start) + (at - plan.us(plan.frame(cut.start_us)))
    return None


def _wide(text: str) -> int:
    """How wide a text is, counting a character of Chinese or Japanese as two."""
    return sum(2 if ord(character) > 0x2E80 else 1 for character in text)


def captions(plan: Plan, timeline: Timeline, transcripts: dict[str, Transcript]) -> list[Cue]:
    """The captions of the main line: the words that are kept, a few at a time, as they are said."""
    shown: list[Cue] = []
    for entry in timeline.main:
        transcript = transcripts[entry.asset]
        chunk: list[int] = []
        for index in [*entry.kept, None]:
            if index is not None:
                trying = [*chunk, index]
                text = line_of([(transcript.text(one), False) for one in trying])
                if not chunk or _wide(text) <= _CAPTION_CHARACTERS:
                    chunk = trying
                    continue
            if not chunk:
                continue
            start = timed(plan, transcripts, (entry.asset, chunk[0]))
            end = timed(plan, transcripts, (entry.asset, chunk[-1]), end=True)
            text = line_of([(transcript.text(one), False) for one in chunk])
            if start is not None and end is not None and end > start:
                shown.append(Cue(start, end, text))
            chunk = [index] if index is not None else []
    return shown


def _clock(us: int, sep: str, digits: int) -> str:
    hours, rest = divmod(us, 3_600_000_000)
    minutes, rest = divmod(rest, 60_000_000)
    seconds, rest = divmod(rest, 1_000_000)
    part = rest // (10 ** (6 - digits))
    return f"{hours:02d}:{minutes:02d}:{seconds:02d}{sep}{part:0{digits}d}"


def srt(cues: Sequence[Cue]) -> str:
    """Write captions as a file of SubRip."""
    blocks = [
        f"{number}\n{_clock(cue.start_us, ',', 3)} --> {_clock(cue.end_us, ',', 3)}\n{cue.text}\n"
        for number, cue in enumerate(cues, start=1)
    ]
    return "\n".join(blocks)


_ALIGN = {
    "full": 5,
    "center": 5,
    "top-left": 7,
    "top-right": 9,
    "bottom-left": 1,
    "bottom-right": 3,
    "caption": 2,
}


def _font() -> str:
    """A font that has Chinese, on the system that this runs on."""
    if sys.platform == "win32":
        return "Microsoft YaHei"
    if sys.platform == "darwin":
        return "PingFang SC"
    return "Noto Sans CJK SC"


def ass(cues: Sequence[Cue], canvas: Canvas) -> str:
    """Write captions and titles as a file of styles for the drawing of subtitles."""
    caption = round(canvas.height * 0.05)
    title = round(canvas.height * 0.08)
    margin = round(canvas.height * 0.06)
    lines = [
        "[Script Info]",
        "ScriptType: v4.00+",
        f"PlayResX: {canvas.width}",
        f"PlayResY: {canvas.height}",
        "WrapStyle: 0",
        "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, "
        "BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV",
    ]
    for name, align in _ALIGN.items():
        size = caption if name == "caption" else title
        lines.append(
            f"Style: {name},{_font()},{size},&H00FFFFFF,&H00000000,&H80000000,"
            f"{0 if name == 'caption' else -1},1,{max(1, size // 16)},0,{align},"
            f"{margin},{margin},{margin}"
        )
    lines += ["", "[Events]", "Format: Layer, Start, End, Style, Text"]
    for cue in cues:
        text = cue.text.replace("\\", "/").replace("{", "(").replace("}", ")").replace("\n", "\\N")
        start, end = _clock(cue.start_us, ".", 2)[1:], _clock(cue.end_us, ".", 2)[1:]
        lines.append(
            f"Dialogue: {0 if cue.style == 'caption' else 1},{start},{end},{cue.style},{text}"
        )
    return "\n".join(lines) + "\n"


def _seconds(value: Fraction) -> str:
    return f"{float(value):.6f}"


class _Inputs:
    """The sources that ffmpeg opens, each from where it is read on."""

    def __init__(self, manifest: Manifest) -> None:
        self.paths = {asset.id: Path(asset.path) for asset in manifest.assets}
        self.arguments: list[str] = []
        self.count = 0
        # Where each source that is open is read up to, and the input that reads it.
        self.reading: dict[str, tuple[int, Fraction, Fraction]] = {}

    def open(self, asset: str, start: Fraction, *, loop: bool = False) -> tuple[int, Fraction]:
        """Open a source from a time on, and return its input and the time that it begins at."""
        seek = ["-ss", _seconds(start)] if start > 0 and not loop else []
        self.arguments += [*seek, *media.GUARDS, "-i", media.source(self.paths[asset])]
        self.count += 1
        return self.count - 1, start if seek else Fraction(0)

    def forward(self, asset: str, start: Fraction, end: Fraction) -> tuple[int, Fraction]:
        """The input that a stretch is read from: the one that reads its source, going forward."""
        known = self.reading.get(asset)
        if known is not None and start >= known[2]:
            self.reading[asset] = (known[0], known[1], end)
            return known[0], known[1]
        number, begins = self.open(asset, start)
        self.reading[asset] = (number, begins, end)
        return number, begins


def compiled(
    plan: Plan,
    timeline: Timeline,
    manifest: Manifest,
    transcripts: dict[str, Transcript],
    quality: Quality,
) -> Compiled:
    """Compile a film into what ffmpeg is run with."""
    canvas = manifest.canvas
    assets = {asset.id: asset for asset in manifest.assets}
    inputs = _Inputs(manifest)
    graph: list[str] = []
    rate = f"{plan.rate.numerator}/{plan.rate.denominator}"
    fit = ""
    if canvas is not None:
        fit = (
            f"scale={canvas.width}:{canvas.height}:force_original_aspect_ratio=decrease,"
            f"pad={canvas.width}:{canvas.height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,format=yuv420p"
        )
    sound = f"aresample={_AUDIO_RATE},aformat=sample_fmts=fltp:channel_layouts=stereo"
    pieces: list[tuple[str | None, str]] = []
    for place, cut in enumerate(plan.cuts):
        first = Fraction(plan.frame(cut.start_us)) / plan.rate
        last = first + Fraction(plan.lengths[place]) / plan.rate
        number, begins = inputs.forward(cut.asset, first, last)
        start, end = _seconds(first - begins), _seconds(last - begins)
        graph.append(
            f"[{number}:a]atrim=start={start}:end={end},asetpts=PTS-STARTPTS,{sound}[a{place}]"
        )
        picture: str | None = None
        if canvas is not None:
            picture = f"v{place}"
            if assets[cut.asset].video is not None:
                graph.append(
                    f"[{number}:v]fps={rate},trim=start={start}:end={end},setpts=PTS-STARTPTS,{fit}[{picture}]"
                )
            else:
                graph.append(
                    f"color=c=black:s={canvas.width}x{canvas.height}:r={rate}:"
                    f"d={_seconds(last - first)},format=yuv420p,setsar=1[{picture}]"
                )
        pieces.append((picture, f"a{place}"))
    video, voice = _joined_line(graph, plan, pieces)
    sounds, pictures, floating = _layers(graph, inputs, plan, timeline, transcripts, assets, canvas)
    audio = _mixed(graph, voice, sounds, duck=timeline.duck)
    styles: str | None = None
    if canvas is not None and video is not None:
        video, floating_titles, styles = _drawn(
            graph, pictures, plan, timeline, transcripts, canvas, video
        )
        floating += floating_titles
        if quality == "preview" and canvas.height > _PREVIEW_HEIGHT:
            graph.append(f"[{video}]scale=-2:{_PREVIEW_HEIGHT}[small]")
            video = "small"
    maps = ["-map", f"[{audio}]"] if video is None else ["-map", f"[{video}]", "-map", f"[{audio}]"]
    return Compiled(inputs.arguments, ";\n".join(graph), maps, styles, tuple(floating))


def _joined_line(
    graph: list[str], plan: Plan, pieces: list[tuple[str | None, str]]
) -> tuple[str | None, str]:
    """Join the stretches of the main line, and cross those that fade into each other."""
    groups: list[list[int]] = [[]]
    for place in range(len(pieces)):
        groups[-1].append(place)
        if plan.fades[place] and place + 1 < len(pieces):
            groups.append([])
    pictured = pieces[0][0] is not None
    joined: list[tuple[str | None, str, int]] = []
    for number, group in enumerate(groups):
        frames = sum(plan.lengths[place] for place in group)
        if len(group) == 1:
            picture, voice = pieces[group[0]]
            joined.append((picture, voice, frames))
            continue
        labels = "".join(
            (f"[{pieces[place][0]}]" if pictured else "") + f"[{pieces[place][1]}]"
            for place in group
        )
        picture = f"g{number}v" if pictured else None
        out = (f"[{picture}]" if pictured else "") + f"[g{number}a]"
        graph.append(f"{labels}concat=n={len(group)}:v={1 if pictured else 0}:a=1{out}")
        joined.append((picture, f"g{number}a", frames))
    picture, voice, frames = joined[0]
    for number, (next_picture, next_voice, next_frames) in enumerate(joined[1:], start=1):
        last = groups[number - 1][-1]
        fade = plan.fades[last]
        seconds = _seconds(Fraction(fade) / plan.rate)
        if pictured:
            offset = _seconds(Fraction(frames - fade) / plan.rate)
            graph.append(
                f"[{picture}][{next_picture}]xfade=transition=fade:duration={seconds}:offset={offset}[x{number}v]"
            )
            picture = f"x{number}v"
        graph.append(f"[{voice}][{next_voice}]acrossfade=d={seconds}[x{number}a]")
        voice = f"x{number}a"
        frames += next_frames - fade
    return picture, voice


def _span(
    plan: Plan, transcripts: dict[str, Transcript], layer: Layer, lasting_us: int | None
) -> tuple[int, int] | None:
    """When a layer begins and ends in the film, or ``None`` where it lies nowhere.

    Args:
        plan: The plan of the film.
        transcripts: The transcripts of the main line.
        layer: The layer.
        lasting_us: How long its source plays from where the layer takes it, if it ends.
    """
    start = (
        timed(plan, transcripts, layer.anchor)
        if layer.anchor is not None
        else plan.us(layer.at_frame or 0)
    )
    if start is None or start >= plan.length_us:
        return None
    if layer.until is not None:
        end = timed(plan, transcripts, layer.until, end=True)
        if end is None:
            return None
    elif layer.frames is not None:
        end = start + plan.us(layer.frames)
    elif lasting_us is not None:
        end = start + round(lasting_us / layer.speed)
    else:
        end = plan.length_us
    end = min(end, plan.length_us)
    return (start, end) if end > start else None


@dataclass(frozen=True)
class _Laid:
    """A picture that is laid over the film: its label in the graph, when, and where."""

    label: str
    begin: str
    end: str
    place: str


def _faded(begin: Fraction, end: Fraction, fade: Fraction, *, alpha: bool) -> str:
    if fade <= 0:
        return ""
    if alpha:
        return (
            f",fade=t=in:st={_seconds(begin)}:d={_seconds(fade)}:alpha=1,"
            f"fade=t=out:st={_seconds(end - fade)}:d={_seconds(fade)}:alpha=1"
        )
    out = _seconds(end - begin - fade)
    return f",afade=t=in:st=0:d={_seconds(fade)},afade=t=out:st={out}:d={_seconds(fade)}"


def _layers(
    graph: list[str],
    inputs: _Inputs,
    plan: Plan,
    timeline: Timeline,
    transcripts: dict[str, Transcript],
    assets: dict[str, Asset],
    canvas: Canvas | None,
) -> tuple[list[str], list[_Laid], list[str]]:
    """Take the pictures and the sounds of the layers out of their sources, to where they lie.

    Returns:
        The labels of the sounds, the pictures, and the ids of the layers that lie nowhere.
    """
    rate = f"{plan.rate.numerator}/{plan.rate.denominator}"
    sounds: list[str] = []
    pictures: list[_Laid] = []
    floating: list[str] = []
    for number, layer in enumerate(timeline.layers):
        if layer.kind == "title":
            continue
        asset = assets.get(layer.asset or "")
        pictured = (
            layer.kind == "picture"
            and canvas is not None
            and asset is not None
            and asset.video is not None
        )
        sounding = layer.kind == "sound" and asset is not None and asset.audio is not None
        if asset is None or not (pictured or sounding):
            floating.append(layer.id)
            continue
        still = asset.kind == "image"
        span = _span(
            plan, transcripts, layer, None if still else max(0, asset.duration_us - layer.source_us)
        )
        if span is None:
            floating.append(layer.id)
            continue
        begin, end = Fraction(span[0], 1_000_000), Fraction(span[1], 1_000_000)
        fade = min(Fraction(layer.fade_us, 1_000_000), (end - begin) / 2)
        taken = _seconds((end - begin) * Fraction(layer.speed).limit_denominator(1000))
        source, begins = inputs.open(asset.id, Fraction(layer.source_us, 1_000_000), loop=still)
        skip = _seconds(Fraction(layer.source_us, 1_000_000) - begins)
        if pictured and canvas is not None:
            reading = "loop=loop=-1:size=1:start=0," if still else ""
            speed = f",setpts=PTS/{layer.speed:g}" if layer.speed != 1 else ""
            graph.append(
                f"[{source}:v]{reading}fps={rate},trim=start={0 if still else skip}:"
                f"duration={taken},setpts=PTS-STARTPTS{speed},setpts=PTS+{_seconds(begin)}/TB,"
                f"{_shaped(layer, canvas)},"
                f"format=yuva420p,colorchannelmixer=aa={layer.opacity:g}"
                f"{_faded(begin, end, fade, alpha=True)}[p{number}]"
            )
            pictures.append(
                _Laid(f"p{number}", _seconds(begin), _seconds(end), _place(layer, canvas))
            )
        else:
            tempo = f",atempo={layer.speed:g}" if layer.speed != 1 else ""
            graph.append(
                f"[{source}:a]atrim=start={skip}:duration={taken},asetpts=PTS-STARTPTS{tempo},"
                f"volume={layer.volume:g}{_faded(begin, end, fade, alpha=False)},"
                f"adelay={round(begin * 1000)}:all=1,aresample={_AUDIO_RATE},"
                f"aformat=sample_fmts=fltp:channel_layouts=stereo[s{number}]"
            )
            sounds.append(f"s{number}")
    return sounds, pictures, floating


def _shaped(layer: Layer, canvas: Canvas) -> str:
    """Make a picture as large as its layer says: the whole film, or a part of it."""
    if layer.position == "full":
        return (
            f"scale={canvas.width}:{canvas.height}:force_original_aspect_ratio=increase,"
            f"crop={canvas.width}:{canvas.height},setsar=1"
        )
    wide = max(2, round(canvas.width * layer.scale / 2) * 2)
    return f"scale={wide}:-2,setsar=1"


def _place(layer: Layer, canvas: Canvas) -> str:
    """Where a picture stands over the film, as the coordinates of an overlay."""
    margin = round(min(canvas.width, canvas.height) * 0.04)
    return {
        "full": "0:0",
        "center": "(W-w)/2:(H-h)/2",
        "top-left": f"{margin}:{margin}",
        "top-right": f"W-w-{margin}:{margin}",
        "bottom-left": f"{margin}:H-h-{margin}",
        "bottom-right": f"W-w-{margin}:H-h-{margin}",
    }[layer.position]


def _mixed(graph: list[str], voice: str, sounds: list[str], *, duck: bool) -> str:
    """Mix the sounds of the layers under the voice, quieter while it speaks if so asked."""
    if not sounds:
        return voice
    bed = sounds[0]
    if len(sounds) > 1:
        graph.append(
            f"{''.join(f'[{one}]' for one in sounds)}amix=inputs={len(sounds)}:normalize=0[bed]"
        )
        bed = "bed"
    if duck:
        graph.append(f"[{voice}]asplit=2[voice][key]")
        graph.append(
            f"[{bed}][key]sidechaincompress=threshold=0.03:ratio=8:attack=20:release=400[ducked]"
        )
        voice, bed = "voice", "ducked"
    graph.append(f"[{voice}][{bed}]amix=inputs=2:duration=first:normalize=0[mixed]")
    return "mixed"


def _drawn(
    graph: list[str],
    pictures: list[_Laid],
    plan: Plan,
    timeline: Timeline,
    transcripts: dict[str, Transcript],
    canvas: Canvas,
    video: str,
) -> tuple[str, list[str], str | None]:
    """Lay the pictures over the film, and draw the titles and captions on it.

    Returns:
        The label of the film, the ids of the titles that lie nowhere, and the file of styles.
    """
    for laid in pictures:
        graph.append(
            f"[{video}][{laid.label}]overlay={laid.place}:eof_action=pass:"
            f"enable='between(t,{laid.begin},{laid.end})'[o{laid.label}]"
        )
        video = f"o{laid.label}"
    cues = captions(plan, timeline, transcripts) if timeline.captions else []
    floating: list[str] = []
    for layer in timeline.layers:
        if layer.kind != "title":
            continue
        span = _span(plan, transcripts, layer, None)
        if span is None:
            floating.append(layer.id)
            continue
        cues.append(Cue(span[0], span[1], layer.text or "", layer.position))
    if not cues:
        return video, floating, None
    graph.append(f"[{video}]subtitles=film.ass[drawn]")
    return "drawn", floating, ass(cues, canvas)


def _encoding(quality: Quality, pictured: bool) -> list[str]:
    audio = ["-c:a", "aac", "-b:a", "96k" if quality == "preview" else "192k"]
    if not pictured:
        return audio
    if quality == "preview":
        return [
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
            *audio,
        ]
    return ["-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p", *audio]


async def rendered(
    project: Project,
    tools: Tools,
    timeline: Timeline,
    transcripts: dict[str, Transcript],
    silences: dict[str, tuple[Silence, ...]],
    quality: Quality,
    name: str,
) -> tuple[Path, Path | None, Plan, Compiled]:
    """Make the film of a timeline, and its captions, and move them to ``out/``.

    Returns:
        The film, the file of captions if there are any, the plan and what ffmpeg was run with.

    Raises:
        RenderError: There is nothing to make, or what was made is not the film.
        MediaUnavailableError: ffmpeg cannot be run.
        UnreadableMediaError: ffmpeg failed.
        ChangedSourceError: A source is another file than the one that was taken in.
    """
    manifest = project.manifest()
    if not timeline.main:
        raise RenderError("the main line is empty: nothing is kept of what was said")
    used = {entry.asset for entry in timeline.main} | {
        layer.asset for layer in timeline.layers if layer.asset
    }
    lengths: dict[str, int] = {}
    for asset in manifest.assets:
        if asset.id in used:
            lengths[asset.id] = (await project.checked(asset)).duration_us
    plan = planned(timeline, transcripts, silences, lengths, manifest.canvas)
    made = compiled(plan, timeline, manifest, transcripts, quality)
    pictured = manifest.canvas is not None
    ending = "mp4" if pictured else "m4a"
    out = project.directory / "out"
    out.mkdir(parents=True, exist_ok=True)
    with media.apart(project.cache) as cwd:
        (cwd / "graph.txt").write_text(made.graph, encoding="utf-8")
        if made.styles is not None:
            (cwd / "film.ass").write_text(made.styles, encoding="utf-8")
        reading = (
            ["-/filter_complex", "graph.txt"]
            if tools.avfilter >= 10
            else ["-filter_complex_script", "graph.txt"]
        )
        argv = [
            tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
            *made.inputs, *reading, *made.maps, *_encoding(quality, pictured),
            "-movflags", "+faststart", f"film.{ending}",
        ]  # fmt: skip
        await media.ran(tools, argv, cwd=cwd, seconds=_RENDER_SECONDS)
        facts = await media.probed(tools, cwd / f"film.{ending}", place=project.cache)
        slack = 2 * 1_000_000 * plan.rate.denominator // plan.rate.numerator + 50_000
        if abs(facts.duration_us - plan.length_us) > slack:
            raise RenderError(
                f"the film lasts {facts.duration_us / 1e6:.3f} s, and the plan "
                f"{plan.length_us / 1e6:.3f} s. It was not kept"
            )
        if facts.audio is None or (pictured and facts.video is None):
            raise RenderError("the film lacks its picture or its sound. It was not kept")
        target = out / f"{name}.{ending}"
        (cwd / f"film.{ending}").replace(target)
    cues = captions(plan, timeline, transcripts)
    written_captions = None
    if cues:
        written_captions = out / f"{name}.srt"
        written_captions.write_text(srt(cues), encoding="utf-8")
    return target, written_captions, plan, made
