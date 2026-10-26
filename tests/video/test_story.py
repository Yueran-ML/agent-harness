"""The story: the main line as text, written and read back."""

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from columbina.video.story import (
    PAUSE_SHOWN_US,
    StoryError,
    line_of,
    natural_pause,
    normal,
    read,
    whole,
    written,
)
from columbina.video.timeline import PAUSE_LEAST_US, Entry, Layer, Timeline, Transcript, Word
from columbina.video.transcribe import transcript_of


def word(start: float, end: float, text: str) -> Word:
    return Word(start_us=round(start * 1e6), end_us=round(end * 1e6), text=text)


def talk() -> Transcript:
    return transcript_of(
        "talk",
        "d" * 64,
        [
            word(0.2, 0.5, "Hello,"),
            word(0.6, 0.8, "uh"),
            word(0.9, 1.3, "today"),
            word(1.35, 2.0, "we cut."),
            word(3.2, 3.5, "This"),
            word(3.55, 3.8, "this"),
            word(3.85, 4.6, "works."),
            word(5.0, 5.2, "我"),
            word(5.25, 5.4, "我"),
            word(5.45, 5.9, "觉得"),
            word(6.0, 6.4, "好。"),
        ],
        [],
        70_000_000,
    )


def transcripts() -> dict[str, Transcript]:
    return {"talk": talk()}


DIGEST = "3f9a61c2e0d4" + "0" * 52


def story_of(timeline: Timeline) -> str:
    return written(timeline, transcripts(), DIGEST)


def lines(text: str) -> list[str]:
    return text.splitlines()[2:]


def test_the_story_of_every_word_says_every_sentence_and_its_pauses() -> None:
    text = story_of(Timeline(main=whole(talk())))

    assert text.splitlines()[:2] == ["<!-- columbina story: 3f9a61c2e0d4 -->", ""]
    assert lines(text) == [
        "[talk.0 00:00] Hello, uh today we cut.",
        "[pause 1.0s]",
        "[talk.1 00:03] This this works.",
        "[talk.2 00:05] 我我觉得好。",
    ]


def test_what_was_cut_is_written_struck_through_where_it_was_said() -> None:
    said = talk()
    main = (
        Entry(asset="talk", sentence=2, kept=(8, 9, 10)),
        Entry(asset="talk", sentence=0, kept=(0, 2, 3), pause_us=400_000),
    )
    layer = Layer(
        id="picture-1", kind="picture", asset="screen", anchor=("talk", 2), until=("talk", 3)
    )

    text = story_of(Timeline(main=main, layers=(layer,)))

    assert said.sentences[1] == (4, 7)
    assert lines(text) == [
        "[talk.2 00:05] ~~我~~我觉得好。",
        "[talk.0 00:00] Hello, ~~uh~~ today we cut.",
        "  > picture screen picture-1",
        "[pause 0.4s]",
        "[talk.1 00:03] ~~This this works.~~",
    ]


def test_a_title_is_noted_by_its_words() -> None:
    title = Layer(id="title-1", kind="title", text="Welcome", anchor=("talk", 0))

    text = story_of(
        Timeline(
            main=whole(talk()), layers=(title, Layer(id="t2", kind="title", text="x", at_frame=0))
        )
    )

    assert "  > title Welcome title-1" in lines(text)


def test_sentences_of_a_source_before_what_is_kept_come_first() -> None:
    main = (Entry(asset="talk", sentence=2, kept=(7, 9, 10)),)

    assert lines(story_of(Timeline(main=main))) == [
        "[talk.0 00:00] ~~Hello, uh today we cut.~~",
        "[talk.1 00:03] ~~This this works.~~",
        "[talk.2 00:05] 我~~我~~觉得好。",
    ]


def test_a_source_of_which_nothing_is_kept_is_written_at_the_end() -> None:
    other = transcript_of("other", "e", [word(0, 1, "Elsewhere.")], [], 2_000_000)

    text = written(Timeline(main=whole(talk())[:1]), {"talk": talk(), "other": other}, DIGEST)

    assert lines(text)[-1] == "[other.0 00:00] ~~Elsewhere.~~"


# Reading a story back


def committed(text: str) -> tuple[Entry, ...]:
    return read(text, transcripts()).main


def test_words_that_are_struck_or_taken_away_are_cut() -> None:
    text = story_of(Timeline(main=whole(talk())))

    edited = text.replace("Hello, uh today", "Hello, ~~uh~~ today").replace(
        "This this works.", "this works."
    )

    first, second, third = committed(edited)
    assert first.kept == (0, 2, 3)
    # Of a word said twice, the one said first is cut.
    assert second.kept == (5, 6)
    assert third.kept == (7, 8, 9, 10)


def test_of_a_word_said_three_times_the_last_is_kept() -> None:
    edited = story_of(Timeline(main=whole(talk()))).replace("我我觉得", "我觉得")

    assert committed(edited)[2].kept == (8, 9, 10)


def test_a_line_taken_away_or_struck_through_is_a_sentence_cut() -> None:
    text = story_of(Timeline(main=whole(talk())))
    taken = text.replace("[talk.1 00:03] This this works.\n", "")
    struck = text.replace("This this works.", "~~This this works.~~")

    assert [one.sentence for one in committed(taken)] == [0, 2]
    assert [one.sentence for one in committed(struck)] == [0, 2]


def test_words_that_were_cut_come_back_when_the_strokes_are_taken_away() -> None:
    main = (Entry(asset="talk", sentence=0, kept=(0, 3)),)
    text = story_of(Timeline(main=main))

    assert committed(text.replace("~~uh today~~", "uh today"))[0].kept == (0, 1, 2, 3)
    assert [
        one.sentence for one in committed(text.replace("~~This this works.~~", "This this works."))
    ] == [0, 1]


def test_a_line_that_is_moved_moves_its_sentence() -> None:
    text = lines(story_of(Timeline(main=whole(talk()))))
    moved = "\n".join(["<!-- columbina story: 3f9a61c2e0d4 -->", text[3], text[0], text[2]])

    assert [one.sentence for one in committed(moved)] == [2, 0, 1]


def test_a_pause_is_named_shortened_or_taken_away() -> None:
    text = story_of(Timeline(main=whole(talk())))

    assert committed(text)[0].pause_us == 1_000_000
    assert committed(text.replace("[pause 1.0s]", "[pause 0.25 s]"))[0].pause_us == 250_000
    assert committed(text.replace("[pause 1.0s]\n", ""))[0].pause_us == PAUSE_LEAST_US
    # Where the source has no long pause, a sentence without a line keeps the pause that it has.
    assert committed(text)[1].pause_us is None


def test_notes_comments_and_blank_lines_are_passed_over() -> None:
    text = story_of(Timeline(main=whole(talk())))
    noted = text.replace(
        "[pause 1.0s]", "<!-- the first cut -->\n  > picture over it\n\n[pause 1.0s]"
    )

    assert committed(noted) == committed(text)
    assert committed("﻿" + text) == committed(text)


HEAD = "<!-- columbina story: 3f9a61c2e0d4 -->"


@pytest.mark.parametrize(
    ("old", "new", "said"),
    [
        (HEAD, "# My story", "line 1"),
        ("[pause 1.0s]", "pause", "line 4: it is neither"),
        ("[talk.1", "[nobody.1", "line 5: there is no sentence nobody.1"),
        ("[talk.2", "[talk.9", "there is no sentence talk.9"),
        ("[talk.2 00:05]", "[talk.1 00:05]", "line 6: the sentence talk.1 is in the story twice"),
        ("today we", "tomorrow we", "line 3: its words are not those of talk.0"),
        ("today we", "we today", "line 3: its words are not those of talk.0"),
        ("Hello, uh", "Hello, ~~u~~h", "line 3"),
        (f"{HEAD}\n", f"{HEAD}\n[pause 1s]\n", "line 2: a pause comes after a sentence"),
    ],
)
def test_a_story_that_cannot_be_read_is_refused_with_its_line(
    old: str, new: str, said: str
) -> None:
    text = story_of(Timeline(main=whole(talk())))

    with pytest.raises(StoryError, match=said):
        committed(text.replace(old, new))
    with pytest.raises(StoryError, match="line 1"):
        committed("")


def test_words_are_matched_by_their_letters_alone() -> None:
    assert normal("Ｈello, World！") == "helloworld"  # noqa: RUF001 - letters as wide as a character
    assert normal("你好。") == "你好"
    assert (
        line_of([("Hello,", False), ("uh", True), ("世界", False), ("好", True), ("ok", True)])
        == "Hello, ~~uh~~ 世界~~好 ok~~"
    )
    assert line_of([]) == ""


def test_the_pause_in_a_source_is_known_only_between_sentences_that_follow_each_other() -> None:
    said = talk()
    first, second, third = whole(said)

    assert natural_pause(said, first, second) == 1_200_000
    assert natural_pause(said, second, third) == 400_000
    assert natural_pause(said, third, None) is None
    assert natural_pause(said, first, third) is None
    assert natural_pause(said, first, Entry(asset="other", sentence=0, kept=(0,))) is None
    assert natural_pause(said, first.model_copy(update={"kept": ()}), second) is None
    assert PAUSE_SHOWN_US < 1_200_000


@settings(max_examples=60, deadline=None)
@given(st.data())
def test_a_story_reads_back_as_the_main_line_it_was_written_from(data: st.DataObject) -> None:
    said = talk()
    entries: list[Entry] = []
    for number in data.draw(st.permutations(list(range(len(said.sentences))))):
        first, last = said.sentences[number]
        kept = tuple(index for index in range(first, last) if data.draw(st.booleans()))
        if any(normal(said.text(index)) for index in kept):
            entries.append(Entry(asset="talk", sentence=number, kept=kept))
    timeline = Timeline(main=tuple(entries))

    again = read(written(timeline, {"talk": said}, DIGEST), {"talk": said}).main

    assert [(one.sentence, [i for i in one.kept if normal(said.text(i))]) for one in again] == [
        (one.sentence, [i for i in one.kept if normal(said.text(i))]) for one in entries
    ]
