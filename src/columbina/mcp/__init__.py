"""Tools from servers that speak the Model Context Protocol.

A server of that protocol offers tools of its own, and a model can call them
like the tools that come with Columbina. What a server says, the names of
its tools, what they are for and what they give back, is text that somebody
else wrote. So each of these is given a shape here before it reaches a
model: see :mod:`columbina.mcp.server`.
"""
