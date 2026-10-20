"""What the assistant says in Feishu by itself, in Chinese and in English.

The model answers in the language that the user writes in. These are the few things that the
program says itself: about pairing, the commands, skills and files, and how a turn ended. An
app in Feishu speaks Chinese and one in Lark English, unless the user chooses.
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

    post: str
    """The key of the language in a message of rich text."""
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
    skills: str
    no_skills: str
    skill_installed: str
    skill_replaced: str
    skill_left_alone: str
    not_a_skill: str
    skill_removed: str
    no_such_skill: str
    skill_not_removable: str
    unread: str
    too_large: str
    file_failed: str
    unknown_command: str
    failed: str
    blocked: str
    cut_off: str
    empty: str
    interrupted: str


CHINESE = Words(
    post="zh_cn",
    pair_please="这是一个私人助手。请发送配对码，配对码在 Columbina 里能看到。",
    paired="配对成功，以后我只回复你。发 /help 看看我能做什么。",
    pair_not_kept="配对没能记下来：{reason}。请过一会儿再发一次配对码。",
    help=(
        "直接给我发消息就能聊天。\n\n"
        "- 发飞书文档的链接，我能读分享给我的文档"
        "（在文档右上角「…」>「…更多」>「添加文档应用」里加上我）。\n"
        "- 把 SKILL.md 当文件发给我，就装上一个技能；其他文本文件发给我，我会读。\n"
        "- /new 开始新对话\n"
        "- /stop 停下正在写的回答\n"
        "- /skills 列出技能，/skills rm 名字 删掉一个技能\n"
        "- /help 显示这段说明"
    ),
    new="已开始新对话。",
    new_dropped="已开始新对话，没答完的 {count} 条消息不再回答。",
    new_failed="没能开始新对话：{reason}。可以再发一次 /new 试试。",
    no_session="现在没有可用的对话。发 /new 开始一个新对话。",
    stopped="已停下。",
    stopped_dropped="已停下，排在后面的 {count} 条消息也不再回答。",
    nothing_to_stop="现在没有在写的回答。",
    skills="我有这些技能：\n\n{lines}",
    no_skills="还没有技能。把 SKILL.md 当文件发给我就能装上。",
    skill_installed="已装上技能 {name}。",
    skill_replaced="已更新技能 {name}，换掉了原来的 SKILL.md。",
    skill_left_alone="技能目录里已有同名的其他文件，它们保持原样：{path}",
    not_a_skill="这个文件不是能用的技能：{reason}",
    skill_removed="已删掉技能 {name}。",
    no_such_skill="我自己的技能目录里没有叫 {name} 的技能。",
    skill_not_removable="技能 {name} 的目录里还有别的文件，请在电脑上删掉这个目录：{path}",
    unread="我现在只看得懂文字和文本文件。",
    too_large="文件太大了，我只收 {most} 以内的文本文件。",
    file_failed="这个文件没能收下：{reason}",
    unknown_command="不认识这个命令。发 /help 看看有哪些。",
    failed="（这次没能回答：{detail}。如果一直这样，请检查 Columbina 里的模型设置和密钥。）",
    blocked="（这次的回答被拦下了：{detail}）",
    cut_off="（回答太长，被截断了。）",
    empty="（这次没有写出回答。）",
    interrupted="（Columbina 中途退出了，这次的回答没写完。）",
)

ENGLISH = Words(
    post="en_us",
    pair_please="This is a personal assistant. Send the pairing code, which Columbina shows.",
    paired="Paired. From now on I answer only you. Send /help to see what I can do.",
    pair_not_kept="The pairing could not be written down: {reason}. Send the pairing code again "
    "in a moment.",
    help=(
        "Write to me, and I answer.\n\n"
        "- Send me a link to a document of Lark, and I read it, if it is shared with me (in the "
        'document, under "…", "… More", "Add document app").\n'
        "- Send me a SKILL.md as a file to add a skill; other text files I read.\n"
        "- /new begins a new conversation\n"
        "- /stop stops the answer that I am writing\n"
        "- /skills lists the skills, /skills rm NAME deletes one\n"
        "- /help shows this"
    ),
    new="A new conversation has begun.",
    new_dropped="A new conversation has begun. The {count} messages that were not answered yet "
    "will not be.",
    new_failed="A new conversation could not begin: {reason}. Send /new to try again.",
    no_session="There is no conversation now. Send /new to begin one.",
    stopped="Stopped.",
    stopped_dropped="Stopped. The {count} messages that were waiting will not be answered.",
    nothing_to_stop="I am not writing an answer now.",
    skills="These are my skills:\n\n{lines}",
    no_skills="I have no skills yet. Send me a SKILL.md as a file to add one.",
    skill_installed="The skill {name} is added.",
    skill_replaced="The skill {name} is updated: its SKILL.md was replaced.",
    skill_left_alone="Other files of the skill are left as they were: {path}",
    not_a_skill="This file is no skill that can be used: {reason}",
    skill_removed="The skill {name} is deleted.",
    no_such_skill="My own directory of skills has no skill called {name}.",
    skill_not_removable="The directory of the skill {name} holds other files too. Delete it on "
    "the computer: {path}",
    unread="For now I read only text and text files.",
    too_large="The file is too large. I take text files of up to {most}.",
    file_failed="The file could not be taken: {reason}",
    unknown_command="I do not know that command. Send /help to see the commands.",
    failed="(I could not answer this time: {detail}. If it goes on, check the model and its key "
    "in Columbina.)",
    blocked="(The answer was stopped: {detail})",
    cut_off="(The answer was too long and was cut off.)",
    empty="(I wrote no answer this time.)",
    interrupted="(Columbina stopped before the answer was done.)",
)


def words_for(language: Language) -> Words:
    """Return what the assistant says by itself in a language."""
    return CHINESE if language == "zh" else ENGLISH
