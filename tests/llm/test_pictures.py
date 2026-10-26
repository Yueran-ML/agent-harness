"""Pictures that tools give back, as each protocol sends them, and what they cost."""

import struct

import pytest

from columbina import pictures
from columbina.llm import (
    AssistantMessage,
    CallConfig,
    LlmRequest,
    Message,
    Picture,
    TextBlock,
    ToolCallBlock,
    ToolMessage,
    ToolSchema,
    UserMessage,
)
from columbina.llm.adapters import anthropic_messages, openai_chat
from columbina.token_meter import PER_PICTURE, estimate_message

SHOT = Picture(media_type="image/png", data="iVBORw0K")
READ = ToolSchema(name="read", description="Read.", parameters={"type": "object"})


def calls(*ids: str) -> AssistantMessage:
    return AssistantMessage(
        content=tuple(ToolCallBlock(call_id=one, name="read", arguments="{}") for one in ids)
    )


def result(call_id: str, text: str, *shown: Picture) -> ToolMessage:
    return ToolMessage(call_id=call_id, content=(TextBlock(text=text),), pictures=shown)


def said(text: str) -> UserMessage:
    return UserMessage(content=(TextBlock(text=text),))


CONVERSATION: tuple[Message, ...] = (
    said("Look."),
    calls("c1", "c2"),
    result("c1", "(A picture.)", SHOT, SHOT),
    result("c2", "words"),
    said("And?"),
)


def openai_body(**config: object) -> list[object]:
    settings = openai_chat.Config(provider="openai", base_url="https://x.test", **config)  # type: ignore[arg-type]
    request = LlmRequest(
        config=CallConfig(provider="openai", model="m"), messages=CONVERSATION, tools=(READ,)
    )
    return openai_chat.build_body(request, settings)["messages"]  # type: ignore[return-value]


def test_openai_is_shown_the_pictures_after_the_results_of_the_reply() -> None:
    messages = openai_body(pictures=True)

    assert [message["role"] for message in messages] == [  # type: ignore[index]
        "user",
        "assistant",
        "tool",
        "tool",
        "user",
        "user",
    ]
    assert messages[2]["content"] == "(A picture.)"  # type: ignore[index]
    assert messages[4] == {
        "role": "user",
        "content": [
            {"type": "text", "text": "The pictures that the call c1 gave back:"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0K"}},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0K"}},
        ],
    }


def test_pictures_of_the_last_results_go_last() -> None:
    settings = openai_chat.Config(provider="openai", base_url="https://x.test", pictures=True)
    request = LlmRequest(
        config=CallConfig(provider="openai", model="m"),
        messages=(said("Look."), calls("c1"), result("c1", "(A picture.)", SHOT)),
    )

    messages: list[dict[str, object]] = openai_chat.build_body(request, settings)["messages"]  # type: ignore[assignment]

    assert [message["role"] for message in messages] == ["user", "assistant", "tool", "user"]


def test_a_model_that_takes_no_pictures_is_told_so_in_their_place() -> None:
    messages = openai_body()

    assert [message["role"] for message in messages] == [  # type: ignore[index]
        "user",
        "assistant",
        "tool",
        "tool",
        "user",
    ]
    assert messages[2]["content"] == (  # type: ignore[index]
        "(A picture.)\n(A picture was given back here, and this model takes no pictures.)"
    )


def test_anthropic_is_shown_the_pictures_in_the_result() -> None:
    settings = anthropic_messages.Config(provider="anthropic", base_url="https://x.test")
    request = LlmRequest(
        config=CallConfig(provider="anthropic", model="claude"),
        messages=(
            said("Look."),
            calls("c1", "c2"),
            result("c1", "(A picture.)", SHOT),
            result("c2", "", SHOT),
        ),
        tools=(READ,),
    )

    turns = anthropic_messages.build_body(request, settings)["messages"]

    image = {
        "type": "image",
        "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0K"},
    }
    assert turns[-1]["content"] == [  # type: ignore[index]
        {
            "type": "tool_result",
            "tool_use_id": "c1",
            "content": [{"type": "text", "text": "(A picture.)"}, image],
        },
        {"type": "tool_result", "tool_use_id": "c2", "content": [image]},
    ]


def test_a_picture_costs_tokens_of_its_own() -> None:
    plain = result("c1", "(A picture.)")

    assert estimate_message(result("c1", "(A picture.)", SHOT, SHOT)) == (
        estimate_message(plain) + 2 * PER_PICTURE
    )


def test_a_message_without_pictures_is_written_without_them() -> None:
    assert "pictures" not in result("c1", "words").model_dump_json()
    assert '"pictures":[{' in result("c1", "words", SHOT).model_dump_json()


# What a picture is, by its bytes


def png(width: int, height: int) -> bytes:
    return b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR" + struct.pack(">II", width, height)


def jpeg(width: int, height: int) -> bytes:
    app = b"\xff\xe0" + struct.pack(">H", 6) + b"JFIF"
    frame = b"\xff\xc0" + struct.pack(">HBHH", 11, 8, height, width) + b"\x00" * 6
    return b"\xff\xd8" + app + frame


def webp(chunk: bytes, body: bytes) -> bytes:
    return b"RIFF" + b"\x00" * 4 + b"WEBP" + chunk + b"\x00" * 4 + body


@pytest.mark.parametrize(
    ("data", "kind", "size"),
    [
        (png(800, 600), "image/png", (800, 600)),
        (jpeg(640, 480), "image/jpeg", (640, 480)),
        (b"GIF89a" + struct.pack("<HH", 32, 16), "image/gif", (32, 16)),
        (b"GIF87a" + b"\x01", "image/gif", None),
        (
            webp(b"VP8X", b"\x00" * 4 + (99).to_bytes(3, "little") + (49).to_bytes(3, "little")),
            "image/webp",
            (100, 50),
        ),
        (webp(b"VP8L", b"\x2f" + ((9) | (19 << 14)).to_bytes(4, "little")), "image/webp", (10, 20)),
        (webp(b"VP8 ", b"\x00" * 6 + struct.pack("<HH", 30, 40)), "image/webp", (30, 40)),
        (webp(b"VP8?", b""), "image/webp", None),
        (b"\xff\xd8\xff\xe0\x00\x02\x00\x00\x00\x00", "image/jpeg", None),
        (b"\xff\xd8\x00", None, None),
        (b"\xff\xd8\xff\xe0\x00\x10" + b"\x00" * 14 + b"\x00", "image/jpeg", None),
        (jpeg(1, 1)[:10], "image/jpeg", None),
        (b"plain text", None, None),
    ],
)
def test_a_picture_is_known_by_its_bytes_with_its_size(
    data: bytes, kind: str | None, size: tuple[int, int] | None
) -> None:
    assert pictures.kind_of(data) == kind
    assert pictures.size_of(data) == size


def test_a_header_that_breaks_off_says_no_size() -> None:
    assert pictures.size_of(png(1, 1)[:20]) is None
