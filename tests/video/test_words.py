"""Words and the stretches that are cut from them: ``timeline`` and ``transcribe``."""

import json
from collections.abc import Sequence
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

from columbina import process
from columbina.process import Completed
from columbina.video import whisper_worker
from columbina.video.media import Silence
from columbina.video.timeline import (
    LEAD_US,
    Cut,
    Entry,
    Timeline,
    Transcript,
    Word,
    cuts,
    frames,
    kept_words,
)
from columbina.video.transcribe import (
    Scripted,
    TranscriptionError,
    Whisper,
    fitted,
    sentences_of,
    transcript_of,
)


def word(start: float, end: float, text: str) -> Word:
    return Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)


TALK = [
    word(0.2, 0.5, "Hello,"),
    word(0.6, 0.8, "uh"),
    word(0.9, 1.3, "today"),
    word(1.35, 1.6, "we"),
    word(1.65, 2.0, "cut."),
    word(3.2, 3.5, "This"),
    word(3.55, 3.8, "this"),
    word(3.85, 4.1, "tool"),
    word(4.2, 4.6, "works."),
]
QUIET = (Silence(0, 200_000), Silence(2_000_000, 3_200_000), Silence(4_600_000, 5_000_000))


def transcript() -> Transcript:
    return transcript_of("talk", "d" * 64, TALK, QUIET, 5_000_000)


def played(timeline: Timeline, quiet: Sequence[Silence] = QUIET) -> list[Cut]:
    return cuts(timeline, {"talk": transcript()}, {"talk": tuple(quiet)}, {"talk": 5_000_000})


# Sentences


def test_a_sentence_ends_with_a_stop_a_long_pause_or_many_words() -> None:
    stops = [
        word(0, 0.1, "One."),
        word(0.2, 0.3, "两"),
        word(0.3, 0.4, "个。"),
        word(0.5, 0.6, "Why?"),
    ]
    paused = [word(0, 0.1, "a"), word(0.9, 1.0, "b")]
    many = [word(index / 10, index / 10 + 0.05, "w") for index in range(45)]

    assert sentences_of(stops) == ((0, 1), (1, 3), (3, 4))
    assert sentences_of(paused) == ((0, 1), (1, 2))
    assert sentences_of(many) == ((0, 40), (40, 45))
    assert sentences_of([]) == ()


def test_the_times_are_moved_to_where_the_pauses_end() -> None:
    early = [word(0.1, 0.3, "a"), word(1.85, 2.0, "b"), word(2.9, 3.0, "c")]
    quiet = [Silence(0, 200_000), Silence(1_000_000, 2_000_000), Silence(3_100_000, 9_000_000)]

    moved, shift = fitted(early, quiet, 2_500_000)

    # The word after the first silence is 100 ms early, the next 150 ms: the middle is 125 ms.
    assert shift == 125_000
    assert [one.start_us for one in moved] == [225_000, 1_975_000, 2_500_000]
    assert fitted(early, [], 3_000_000) == (early, 0)


def test_a_transcript_keeps_the_words_in_order_and_trimmed() -> None:
    made = transcript_of(
        "talk", "d", [word(1, 2, " world. "), word(0, 0.5, " Hi"), word(3, 4, "  ")], [], 5_000_000
    )

    assert [one.text for one in made.words] == ["Hi", "world."]
    assert made.sentences == ((0, 2),)
    assert made.text(1) == "world."
    assert made.model_copy(update={"shown": {1: "World!"}}).text(1) == "World!"


# The transcribers


async def test_words_written_down_before_are_read(tmp_path: Path) -> None:
    given = tmp_path / "words.json"
    given.write_text(json.dumps([{"start_us": 0, "end_us": 10, "text": "hi"}]), encoding="utf-8")

    assert await Scripted(given).words(tmp_path / "sound.wav", None) == [
        Word(start_us=0, end_us=10, text="hi")
    ]


@pytest.mark.parametrize("written", ["{", '{"words": []}', '[{"start_us": "x"}]'])
async def test_words_that_cannot_be_read_are_said(tmp_path: Path, written: str) -> None:
    given = tmp_path / "words.json"
    given.write_text(written, encoding="utf-8")

    with pytest.raises(TranscriptionError, match="cannot be read"):
        await Scripted(given).words(tmp_path / "sound.wav", None)
    with pytest.raises(TranscriptionError, match="cannot be read"):
        await Scripted(tmp_path / "none.json").words(tmp_path / "sound.wav", None)


def ended(code: int | None, output: str = "", error: str = "") -> Completed:
    return Completed(
        exit_code=code, timed_out=False, output=output, output_bytes=len(output),
        omitted_bytes=0, saved_bytes=0, error_output=error,
    )  # fmt: skip


@pytest.fixture
def model(tmp_path: Path) -> Path:
    folder = tmp_path / "model"
    folder.mkdir()
    (folder / "model.bin").write_bytes(b"weights")
    return folder


async def test_faster_whisper_is_run_apart_and_its_words_are_read(
    tmp_path: Path, model: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ran: list[list[str]] = []

    async def run(argv: Sequence[str], **options: Any) -> Completed:
        ran.append(list(argv))
        assert options["env"]["PYTHONUTF8"] == "1"
        return ended(0, '{"start_us": 0, "end_us": 5, "text": "你好"}\n\n')

    monkeypatch.setattr(process, "run", run)

    told = await Whisper(model, "python3", "cpu").words(tmp_path / "sound.wav", "zh")

    assert told == [Word(start_us=0, end_us=5, text="你好")]
    assert ran == [
        [
            "python3",
            "-m",
            "columbina.video.whisper_worker",
            str(tmp_path / "sound.wav"),
            str(model),
            "zh",
            "cpu",
        ]
    ]


@pytest.mark.parametrize(
    ("answer", "said"),
    [
        (
            ended(2, error="faster-whisper is not installed.\n"),
            "failed: faster-whisper is not installed",
        ),
        (ended(1), "failed: 1"),
        (ended(0, "not a word\n"), "wrote what is no word"),
    ],
)
async def test_what_faster_whisper_cannot_do_is_said(
    tmp_path: Path, model: Path, monkeypatch: pytest.MonkeyPatch, answer: Completed, said: str
) -> None:
    async def run(argv: Sequence[str], **options: Any) -> Completed:
        return answer

    monkeypatch.setattr(process, "run", run)

    with pytest.raises(TranscriptionError, match=said):
        await Whisper(model).words(tmp_path / "sound.wav", None)


async def test_faster_whisper_without_a_model_or_a_python_is_said(
    tmp_path: Path, model: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(
        TranscriptionError, match="huggingface-cli download Systran/faster-whisper-small"
    ):
        await Whisper(tmp_path / "nothing").words(tmp_path / "sound.wav", None)

    async def run(argv: Sequence[str], **options: Any) -> Completed:
        raise OSError("no such program")

    monkeypatch.setattr(process, "run", run)
    with pytest.raises(TranscriptionError, match="could not be started: no such program"):
        await Whisper(model, "nowhere").words(tmp_path / "sound.wav", None)


class _Array:
    def __init__(self, values: list[float]) -> None:
        self.values = values

    def astype(self, kind: object) -> "_Array":
        return self

    def __truediv__(self, by: float) -> list[float]:
        return [value / by for value in self.values]


def test_the_worker_hands_the_samples_over_and_writes_the_words(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import wave

    sound = tmp_path / "sound.wav"
    with wave.open(str(sound), "wb") as writing:
        writing.setnchannels(1)
        writing.setsampwidth(2)
        writing.setframerate(16000)
        writing.writeframes(b"\x00\x40" * 4)
    asked: dict[str, Any] = {}

    class Model:
        def __init__(self, path: str, device: str, local_files_only: bool) -> None:
            asked.update(path=path, device=device, local=local_files_only)

        def transcribe(self, samples: object, **options: Any) -> tuple[object, object]:
            asked.update(samples=samples, **options)
            said = SimpleNamespace(start=0.5, end=0.75, word=" 你好", probability=0.9)
            return iter([SimpleNamespace(words=[said]), SimpleNamespace(words=None)]), None

    def frombuffer(raw: bytes, dtype: object) -> _Array:
        return _Array([16384.0] * (len(raw) // 2))

    numpy = SimpleNamespace(int16="int16", float32="float32", frombuffer=frombuffer)
    modules: dict[str, object] = {
        "numpy": numpy,
        "faster_whisper": SimpleNamespace(WhisperModel=Model),
        "ctranslate2": SimpleNamespace(get_cuda_device_count=lambda: 1),
    }

    def found(name: str) -> object:
        return modules[name]

    monkeypatch.setattr(whisper_worker, "_module", found)

    assert whisper_worker.main([str(sound), "model", "", "auto"]) == 0

    assert json.loads(capsys.readouterr().out) == {
        "start_us": 500_000, "end_us": 750_000, "text": " 你好", "confidence": 0.9,
    }  # fmt: skip
    assert (asked["device"], asked["local"], asked["language"], asked["word_timestamps"]) == (
        "cuda",
        True,
        None,
        True,
    )
    assert asked["samples"] == [0.5] * 4


def test_the_worker_says_where_faster_whisper_is_missing(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def missing(name: str) -> object:
        raise ImportError(name)

    monkeypatch.setattr(whisper_worker, "_module", missing)

    assert whisper_worker.main(["s", "m", "zh", "cpu"]) == 2
    assert "pip install faster-whisper" in capsys.readouterr().err
    assert whisper_worker._device("cpu") == "cpu"  # pyright: ignore[reportPrivateUsage]
    assert whisper_worker._device("auto") == "cpu"  # pyright: ignore[reportPrivateUsage]


# Where the cuts fall


def test_a_film_of_every_word_is_a_stretch_for_each_pause_too_long() -> None:
    timeline = Timeline(
        main=tuple(
            Entry(asset="talk", sentence=n, kept=kept_words(transcript(), n)) for n in (0, 1)
        )
    )

    first, second = played(timeline)

    # It begins a little before the first word, in the silence before it.
    assert first.start_us == 200_000 - LEAD_US
    # The pause of 1.2 s between the sentences is cut to one second, in its middle.
    assert (first.end_us, second.start_us) == (2_000_000 + 500_000, 3_200_000 - 500_000)
    assert second.end_us == 4_600_000 + LEAD_US


def test_a_word_that_is_cut_where_nobody_pauses_makes_a_tight_cut() -> None:
    timeline = Timeline(main=(Entry(asset="talk", sentence=0, kept=(0, 2, 3, 4)),))

    hello, rest = played(timeline)

    assert (hello.end_us, hello.tight) == (600_000, False)
    assert (rest.start_us, rest.tight) == (900_000, True)


def test_a_cut_into_a_pause_keeps_a_little_of_it_on_each_side() -> None:
    # A short pause after "Hello," and a long one before "today".
    quiet = (*QUIET, Silence(500_000, 600_000), Silence(800_000, 1_100_000))
    words = [*TALK[:2], word(1.1, 1.3, "today"), *TALK[3:]]
    said = transcript_of("talk", "d", words, quiet, 5_000_000)
    timeline = Timeline(main=(Entry(asset="talk", sentence=0, kept=(0, 2, 3, 4)),))

    hello, rest = cuts(timeline, {"talk": said}, {"talk": quiet}, {"talk": 5_000_000})

    assert hello.end_us == 600_000
    assert (rest.start_us, rest.tight) == (1_100_000 - LEAD_US, False)


def test_a_pause_that_is_named_is_kept_as_long_as_that() -> None:
    entries = (
        Entry(asset="talk", sentence=0, kept=kept_words(transcript(), 0), pause_us=400_000),
        Entry(asset="talk", sentence=1, kept=kept_words(transcript(), 1)),
    )

    first, second = played(Timeline(main=entries))

    assert second.start_us - first.end_us == 1_200_000 - 400_000


def test_a_short_pause_joins_two_sentences_into_one_stretch() -> None:
    entries = tuple(
        Entry(asset="talk", sentence=n, kept=kept_words(transcript(), n)) for n in (0, 1)
    )

    (whole,) = played(Timeline(main=entries, pause_most_us=2_000_000))

    assert (whole.start_us, whole.end_us, whole.entry) == (80_000, 4_720_000, 0)


def test_sentences_that_do_not_follow_each_other_are_apart() -> None:
    entries = tuple(
        Entry(asset="talk", sentence=n, kept=kept_words(transcript(), n)) for n in (1, 0)
    )

    second, first = played(Timeline(main=entries))

    assert (second.entry, first.entry) == (0, 1)
    assert first.start_us == 80_000


def test_the_last_word_ends_with_its_source_where_no_silence_follows() -> None:
    entries = (Entry(asset="talk", sentence=1, kept=kept_words(transcript(), 1)),)
    first = transcript_of("talk", "d", [word(0, 0.3, "a")], [], 1_000_000)

    (only,) = played(Timeline(main=entries), ())
    (alone,) = cuts(
        Timeline(main=(Entry(asset="talk", sentence=0, kept=(0,)),)),
        {"talk": first},
        {"talk": ()},
        {"talk": 1_000_000},
    )

    assert (only.start_us, only.end_us) == (3_200_000, 4_600_000 + LEAD_US)
    assert (alone.start_us, alone.end_us) == (0, 300_000 + LEAD_US)


@pytest.mark.parametrize(
    ("us", "rate", "frame"),
    [(1_000_000, "30/1", 30), (1_001_000, "30000/1001", 30), (20_000, "25/1", 0)],
)
def test_a_time_falls_in_a_frame(us: int, rate: str, frame: int) -> None:
    assert frames(us, rate) == frame
