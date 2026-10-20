"""Feishu: a personal assistant that the user talks to in Feishu, or in Lark.

The user makes an app of their own in the developer console of Feishu, gives it a bot, and
hands Columbina its App ID and App Secret. Columbina then keeps a long connection to Feishu
from this machine, hears what the user writes to the bot, and answers there with an agent of
its own.

What there is:

- :mod:`columbina.feishu.app` names the two platforms, and where their APIs are.
- :mod:`columbina.feishu.store` keeps the app, its App Secret, and how the assistant is doing.
- :mod:`columbina.feishu.client` asks the REST API in the name of the app.
- :mod:`columbina.feishu.frames` writes and reads the frames of the long connection.
- :mod:`columbina.feishu.link` keeps the long connection, and hears the events of the app.
- :mod:`columbina.feishu.messages` reads what a message says, and writes an answer as messages.
- :mod:`columbina.feishu.documents` is the tool with which the agent reads documents of Feishu.
- :mod:`columbina.feishu.words` holds what the assistant says by itself, in Chinese and English.
- :mod:`columbina.feishu.bot` is the assistant: the frontend of ``columbina feishu``.
- :mod:`columbina.feishu.setup` sets the app up, says how the assistant is doing, and forgets it.
"""
