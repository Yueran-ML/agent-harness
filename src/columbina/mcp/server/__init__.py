"""One server of the Model Context Protocol, and what its tools look like to a model.

Three things of a server are text that somebody else wrote, and each has a
module that gives it a shape which a model and a service can take.

:mod:`~columbina.mcp.server.names`
    The name under which a tool is offered. It says which server the tool is
    of, and it is made of the characters that a tool name may have.
:mod:`~columbina.mcp.server.schema`
    What a tool is for, and what its arguments look like.
:mod:`~columbina.mcp.server.content`
    What a call gave back, as the text that a model reads.

None of the three talks to a server. The first two know nothing of the SDK
of the protocol, and the third takes a result as the SDK hands it over.

:mod:`~columbina.mcp.server.connection`
    The connection to the server: made, kept, and made again where it is
    lost. It hands out the tools as the server lists them, and calls them by
    the names that the server gave them.
"""
