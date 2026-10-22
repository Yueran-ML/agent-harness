"""What a message to the bot says: ``columbina.wecom.messages``."""

from columbina.wecom.messages import File, Text, Unread, read


def test_text_and_voice_are_read_as_words() -> None:
    assert read({"msgtype": "text", "text": {"content": "你好"}}) == Text("你好")
    assert read({"msgtype": "voice", "voice": {"content": "明天几点开会"}}) == Text("明天几点开会")
    assert read({"msgtype": "text"}) == Text("")
    assert read({"msgtype": "text", "text": {"content": 7}}) == Text("")


def test_a_message_of_text_and_pictures_is_read_as_its_text() -> None:
    body = {
        "msgtype": "mixed",
        "mixed": {
            "msg_item": [
                {"msgtype": "text", "text": {"content": "看看这张图"}},
                {"msgtype": "image", "image": {"url": "https://x/1", "aeskey": "k"}},
                {"msgtype": "text", "text": {"content": ""}},
                {"msgtype": "text", "text": {"content": "还有这个"}},
                "odd",
            ]
        },
    }

    assert read(body) == Text("看看这张图\n还有这个")
    assert read({"msgtype": "mixed", "mixed": {"msg_item": "odd"}}) == Text("")


def test_a_file_is_where_it_lies_and_the_key_it_is_encrypted_with() -> None:
    sent = {"msgtype": "file", "file": {"url": "https://x/f", "aeskey": "abc"}}

    assert read(sent) == File("https://x/f", "abc")
    assert read({"msgtype": "file", "file": {"url": "https://x/f"}}) == Unread("file")
    assert read({"msgtype": "file", "file": {"url": "", "aeskey": "abc"}}) == Unread("file")
    assert read({"msgtype": "file"}) == Unread("file")


def test_what_holds_no_words_is_not_read() -> None:
    assert read({"msgtype": "image", "image": {"url": "https://x/1"}}) == Unread("image")
    assert read({"msgtype": "video"}) == Unread("video")
    assert read({}) == Unread("")


def test_what_a_message_quotes_is_kept_apart_and_given_to_the_agent_before_it() -> None:
    said = read(
        {
            "msgtype": "text",
            "text": {"content": "这句什么意思"},
            "quote": {"msgtype": "text", "text": {"content": "第一行\n第二行"}},
        }
    )

    assert said == Text("这句什么意思", "第一行\n第二行")
    assert isinstance(said, Text)
    assert said.asked() == "> 第一行\n> 第二行\n\n这句什么意思"
    # A quote of what holds no words, or of nothing, is none.
    pictured = {"msgtype": "text", "text": {"content": "?"}, "quote": {"msgtype": "image"}}
    assert read(pictured) == Text("?")
    assert read({"msgtype": "text", "text": {"content": "?"}, "quote": "odd"}) == Text("?")
    assert Text("只有这句").asked() == "只有这句"
