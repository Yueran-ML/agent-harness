"""The benchmark of Columbina: tasks that can be judged without a person, and what judges them.

A task is a directory under ``bench/tasks``, see :mod:`bench.task`. An attempt
at a task runs ``columbina run`` on a copy of its working directory, and is
judged by what the directory holds afterwards and by the calls of tools that
it made, see :mod:`bench.grade`. ``python -m bench`` checks the tasks, runs
attempts and makes the tables of results.

None of this is part of the package that is installed.
"""
