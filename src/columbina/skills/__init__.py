"""Skills: instructions for one kind of task, which a model loads when it meets such a task.

A skill is written once, by the people an agent works for, and lies in a
directory of its own. A model is told what skills there are, in a line each,
and loads the text of one when a task calls for it. So a hundred skills cost
a hundred lines, and the text of a skill costs something only when it is used.

What a skill is on disk, and where skills are looked for, is in
:mod:`columbina.skills.files`.
"""
