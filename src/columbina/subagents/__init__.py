"""Subagents: helpers that an agent hands a task to.

A task that stands by itself can be done by another agent, which works on it
with a conversation of its own and gives back what it found. The agent that
handed it over reads the answer and none of the steps that led to it. So a
search through a code base costs the conversation one answer, and several
such tasks can be worked on side by side.

What kinds of subagent there are, and where they are written down, is in
:mod:`columbina.subagents.types`.
"""
