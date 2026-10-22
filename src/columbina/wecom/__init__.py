"""The assistant in WeCom: a conversation with its owner, through a smart bot of WeCom.

:mod:`columbina.wecom.bot` is the plugin that ``columbina wecom`` loads. The bot keeps a long
connection to WeCom, :mod:`columbina.wecom.link`, over which it hears what is written to it and
answers. :mod:`columbina.wecom.setup` keeps the bot, says how its assistant is doing, and
forgets it.
"""
