"""Guards: what keeps a run within bounds when nobody is watching it.

A loop runs a turn for as long as its model has calls to make. That is right
for a conversation, where a person can stop it, and wrong for a run that
nobody watches: a subagent, a benchmark, a task in the night. The plugins in
this package set the bounds. Each is a plugin of its own, and a program
without them has no bounds.
"""
