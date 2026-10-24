"""What the assistant says in QQ by itself, in Chinese and in English.

The model answers in the language that the user writes in. These are the few things that the
program says itself: about pairing, the commands, and how a turn ended. QQ is used in Chinese,
so that is what it says unless the user chooses English.
"""

from dataclasses import dataclass
from typing import Literal

__all__ = ["CHINESE", "ENGLISH", "Language", "Words", "words_for"]

Language = Literal["zh", "en"]


@dataclass(frozen=True, slots=True)
class Words:
    """What the assistant says by itself in one language.

    The texts that hold ``{...}`` are filled in with :meth:`str.format`.
    """

    stopped_here: str
    """What an answer says where its turn was stopped."""
    dropped_here: str
    """What an answer says where its message will not be answered."""
    pair_please: str
    paired: str
    pair_not_kept: str
    help: str
    new: str
    new_dropped: str
    new_failed: str
    no_session: str
    stopped: str
    stopped_dropped: str
    nothing_to_stop: str
    unread: str
    group_failed: str
    unknown_command: str
    failed: str
    blocked: str
    cut_off: str
    empty: str
    interrupted: str


CHINESE = Words(
    stopped_here="（停下了，没有答完。）",
    dropped_here="（这条不再回答。）",
    pair_please="这是一个私人助手。请发送配对码，配对码在 Columbina 里能看到。",
    paired="配对成功，私聊里以后我只回复你。发 /help 看看我能做什么。",
    pair_not_kept="配对没能记下来：{reason}。请过一会儿再发一次配对码。",
    help=(
        "直接给我发消息就能聊天。\n"
        "/new 开始新对话\n"
        "/stop 停下正在写的回答\n"
        "/help 显示这段说明\n"
        "把我拉进群后，群里谁 @ 我我都会回。"
    ),
    new="已开始新对话。",
    new_dropped="已开始新对话，没答完的 {count} 条消息不再回答。",
    new_failed="没能开始新对话：{reason}。可以再发一次 /new 试试。",
    no_session="现在没有可用的对话。发 /new 开始一个新对话。",
    stopped="已停下。",
    stopped_dropped="已停下，排在后面的 {count} 条消息也不再回答。",
    nothing_to_stop="现在没有在写的回答。",
    unread="我现在只看得懂文字。",
    group_failed="这个群的对话没能打开：{reason}。",
    unknown_command="不认识这个命令。发 /help 看看有哪些。",
    failed="（这次没能回答：{detail}。）",
    blocked="（这次的回答被拦下了：{detail}）",
    cut_off="（回答太长，被截断了。）",
    empty="（这次没有写出回答。）",
    interrupted="（Columbina 中途退出了，这次的回答没写完。）",
)

ENGLISH = Words(
    stopped_here="(Stopped before the answer was done.)",
    dropped_here="(This one is not answered.)",
    pair_please="This is a personal assistant. Send the pairing code, which Columbina shows.",
    paired="Paired. From now on I answer only you in a chat of two. Send /help to see what I "
    "can do.",
    pair_not_kept="The pairing could not be written down: {reason}. Send the pairing code again "
    "in a moment.",
    help=(
        "Write to me, and I answer.\n"
        "/new begins a new conversation\n"
        "/stop stops the answer that I am writing\n"
        "/help shows this\n"
        "Put me in a group, and I answer whoever mentions me there."
    ),
    new="A new conversation has begun.",
    new_dropped="A new conversation has begun. The {count} messages that were not answered yet "
    "will not be.",
    new_failed="A new conversation could not begin: {reason}. Send /new to try again.",
    no_session="There is no conversation now. Send /new to begin one.",
    stopped="Stopped.",
    stopped_dropped="Stopped. The {count} messages that were waiting will not be answered.",
    nothing_to_stop="I am not writing an answer now.",
    unread="For now I read only text.",
    group_failed="The conversation of this group could not be opened: {reason}.",
    unknown_command="I do not know that command. Send /help to see the commands.",
    failed="(I could not answer this time: {detail}.)",
    blocked="(The answer was stopped: {detail})",
    cut_off="(The answer was too long and was cut off.)",
    empty="(I wrote no answer this time.)",
    interrupted="(Columbina stopped before the answer was done.)",
)


def words_for(language: Language) -> Words:
    """Return what the assistant says by itself in a language."""
    return CHINESE if language == "zh" else ENGLISH
