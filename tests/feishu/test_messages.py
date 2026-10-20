"""What a message of Feishu says, and how an answer is written: ``columbina.feishu.messages``."""

import json

import pytest

from columbina.feishu import messages
from columbina.feishu.messages import File, Text, Unread, parts, posted, read


def content(value: object) -> str:
    return json.dumps(value, ensure_ascii=False)


def test_text_is_read_as_it_is() -> None:
    assert read("text", content({"text": "你好 [链接](https://x.cn)"})) == Text(
        "你好 [链接](https://x.cn)"
    )


@pytest.mark.parametrize("given", ["not json", "[1]", content({"text": 3}), content({})])
def test_text_that_cannot_be_read_says_nothing(given: str) -> None:
    assert read("text", given) == Text("")


def test_rich_text_is_read_from_its_markdown_where_feishu_keeps_it() -> None:
    said = {
        "title": "Title",
        "content": [[{"tag": "text", "text": "flattened"}]],
        "content_v2": [[{"tag": "md", "text": "**bold** [link](https://x.cn)"}], [{"tag": "hr"}]],
    }

    assert read("post", content(said)) == Text("Title\n**bold** [link](https://x.cn)\n---")


def test_rich_text_without_its_markdown_is_read_from_its_pieces() -> None:
    said = {
        "content": [
            [
                {"tag": "text", "text": "first: ", "style": ["bold"]},
                {"tag": "a", "href": "https://www.feishu.cn", "text": "a link"},
                {"tag": "a", "text": "no address"},
                {"tag": "at", "user_id": "@_user_1", "user_name": "Tom"},
                {"tag": "at", "user_id": "@_user_2", "user_name": ""},
            ],
            [{"tag": "img", "image_key": "img_1"}],
            [{"tag": "media", "file_key": "file_1"}],
            [{"tag": "emotion", "emoji_type": "SMILE"}],
            [{"tag": "hr"}],
            [{"tag": "code_block", "language": "GO", "text": "func main() {}"}],
            [{"tag": "code_block", "text": "plain"}],
            [{"tag": "something newer"}, "no piece", {"tag": "text", "text": 3}],
            "no paragraph",
        ],
        "content_v2": [],
    }

    assert read("post", content(said)) == Text(
        "first: [a link](https://www.feishu.cn)no address@Tom\n"
        "[a picture that was not read]\n"
        "[a video that was not read]\n"
        "---\n"
        "```go\nfunc main() {}\n```\n"
        "```\nplain\n```"
    )


def test_rich_text_without_paragraphs_says_nothing() -> None:
    assert read("post", content({"title": "", "content": "none"})) == Text("")


def test_a_file_is_named_by_its_key_and_name() -> None:
    assert read("file", content({"file_key": "file_v2_1", "file_name": "SKILL.md"})) == File(
        "file_v2_1", "SKILL.md"
    )
    assert read("file", content({"file_key": "file_v2_1"})) == File("file_v2_1", "file")


def test_a_file_without_a_key_is_not_read() -> None:
    assert read("file", content({"file_name": "SKILL.md"})) == Unread("file")


@pytest.mark.parametrize("kind", ["image", "audio", "media", "sticker", "folder", "share_chat"])
def test_what_is_no_text_is_said_to_be_so(kind: str) -> None:
    assert read(kind, content({"image_key": "img_1"})) == Unread(kind)


def test_an_answer_is_rich_text_with_one_paragraph_of_markdown() -> None:
    assert json.loads(posted("## 标题\n\n- 一")) == {
        "zh_cn": {"content": [[{"tag": "md", "text": "## 标题\n\n- 一"}]]}
    }
    assert list(json.loads(posted("x", "en_us"))) == ["en_us"]


def request_size(part: str) -> int:
    body = {"content": posted(part), "msg_type": "post", "uuid": "u" * 36}
    return len(json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode())


def test_a_short_answer_is_one_part() -> None:
    assert parts("short") == ["short"]
    assert parts("") == [""]


def test_a_long_answer_is_cut_between_lines_into_parts_that_fit() -> None:
    lines = [f"第 {number} 行带一点“引号”和 \\ 反斜杠" for number in range(3000)]

    cut = parts("\n".join(lines))

    assert len(cut) > 1
    assert all(request_size(part) <= 28_000 for part in cut)
    assert "\n".join(cut) == "\n".join(lines)


def test_a_cut_inside_fenced_code_closes_it_and_opens_it_again() -> None:
    code = ["```python", *[f"print({number})" for number in range(200)], "```", "after"]

    cut = parts("\n".join(code), most=2_000)

    assert len(cut) > 1
    for part in cut[:-1]:
        assert part.endswith("\n```")
    for part in cut[1:]:
        assert part.startswith("```python\n")
    assert cut[-1].endswith("```\nafter")
    assert all(request_size(part) <= 2_000 for part in cut)


def test_a_part_that_closes_fenced_code_has_room_for_the_closing() -> None:
    code = "\n".join(["```", *["x" * 50 for _ in range(60)], "```"])
    cost = messages._cost  # pyright: ignore[reportPrivateUsage]
    # What a request holds besides the part: the rich text around it, and the rest of the body.
    room = cost(posted("")) + 200

    for most in range(700, 1_100):
        for part in parts(code, most=most):
            assert cost(part) <= most - room, (most, part[-20:])


def test_a_fence_of_tildes_and_a_longer_closing_fence_are_known() -> None:
    code = ["~~~~", *[f"x = {number}" for number in range(400)], "~~~~~", "after"]

    cut = parts("\n".join(code), most=1_500)

    assert len(cut) > 1
    assert cut[0].endswith("\n~~~~")
    assert cut[1].startswith("~~~~\n")
    assert not cut[-1].startswith("~~~~\n") or "~~~~~" in cut[-1]


def test_a_line_with_words_after_the_fence_does_not_close_it() -> None:
    code = ["```", "```python", *[f"y{number}" for number in range(400)], "```"]

    cut = parts("\n".join(code), most=1_500)

    assert len(cut) > 1
    assert cut[1].startswith("```\n")


def test_four_spaces_before_backticks_make_no_fence() -> None:
    lines = ["    ```", *[f"z{number}" for number in range(400)]]

    cut = parts("\n".join(lines), most=1_500)

    assert not cut[0].endswith("```")


def test_a_line_too_long_for_a_part_is_cut_into_pieces() -> None:
    line = "长" * 20_000

    cut = parts(line)

    assert len(cut) > 1
    assert "".join(cut) == line
    assert all(request_size(part) <= 28_000 for part in cut)
