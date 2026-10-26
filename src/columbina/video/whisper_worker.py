"""faster-whisper, in a process of its own: what :class:`columbina.video.transcribe.Whisper` runs.

``python -m columbina.video.whisper_worker SOUND MODEL LANGUAGE DEVICE`` writes the words of the
sound to its output, one document of JSON on each line. The model is read from its directory and
nothing is downloaded. Where there is a card for CUDA and ``DEVICE`` is ``auto``, it is used.

faster-whisper reads files with PyAV, and the PyAV of today no longer takes what it is given. The
sound is 16 kHz and one channel already, so its samples are read here and handed over as they are.

faster-whisper, CTranslate2 and NumPy are not dependencies of this program: they are imported by
name, in this process alone.
"""

import importlib
import json
import sys
import wave
from collections.abc import Iterable, Iterator
from typing import Any

__all__ = ["main", "samples", "words_of"]


def _module(name: str) -> Any:
    return importlib.import_module(name)


def samples(path: str) -> Any:
    """Read the samples of a sound of 16 bits, as floats from -1 to 1."""
    numpy = _module("numpy")
    with wave.open(path, "rb") as sound:
        raw = sound.readframes(sound.getnframes())
    return numpy.frombuffer(raw, dtype=numpy.int16).astype(numpy.float32) / 32768.0


def words_of(segments: Iterable[Any]) -> Iterator[dict[str, object]]:
    """Turn what faster-whisper gives into words with times in microseconds."""
    for segment in segments:
        for word in segment.words or ():
            yield {
                "start_us": round(float(word.start) * 1_000_000),
                "end_us": round(float(word.end) * 1_000_000),
                "text": str(word.word),
                "confidence": float(word.probability),
            }


def _device(asked: str) -> str:
    """The device to run on: the one that is asked for, or a card for CUDA where there is one."""
    if asked != "auto":
        return asked
    try:
        return "cuda" if _module("ctranslate2").get_cuda_device_count() > 0 else "cpu"
    except Exception:
        return "cpu"


def main(argv: list[str]) -> int:
    """Write the words of a sound, and return the exit code."""
    sound, model, language, device = argv
    try:
        whisper = _module("faster_whisper")
    except ImportError:
        print(
            "faster-whisper is not installed. Install it into the Python of Columbina with "
            "`pip install faster-whisper`",
            file=sys.stderr,
        )
        return 2
    reader = whisper.WhisperModel(model, device=_device(device), local_files_only=True)
    segments, _ = reader.transcribe(
        samples(sound), language=language or None, word_timestamps=True, vad_filter=False
    )
    for word in words_of(segments):
        sys.stdout.write(json.dumps(word, ensure_ascii=False) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":  # pragma: no cover - the process that is run
    sys.exit(main(sys.argv[1:]))
