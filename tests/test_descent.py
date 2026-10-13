"""Which processes descend from which."""

from collections.abc import Callable

from hypothesis import given
from hypothesis import strategies as st

from columbina.descent import descended


def times(table: dict[int, int]) -> Callable[[int], int | None]:
    """Tells when each process of a table was made, and of no other."""
    return table.get


def test_what_a_held_process_made_descends_from_it() -> None:
    found = descended({10: 100}, {10: 1, 11: 10}, times({11: 200}))

    assert found == {11: 200}


def test_what_a_descendant_made_descends_too() -> None:
    parents = {11: 10, 12: 11, 13: 12}

    found = descended({10: 100}, parents, times({11: 200, 12: 300, 13: 400}))

    assert found == {11: 200, 12: 300, 13: 400}


def test_a_process_older_than_the_one_it_names_is_a_stranger() -> None:
    # A number is given out again when its process is gone. Whoever made 11 had the number
    # 10 before the process that has it now.
    found = descended({10: 500}, {11: 10}, times({11: 400}))

    assert found == {}


def test_what_a_stranger_made_is_left_alone() -> None:
    # 12 is younger than everyone, and all the same it descends from nothing that is held.
    found = descended({10: 500}, {11: 10, 12: 11}, times({11: 400, 12: 600}))

    assert found == {}


def test_one_made_at_the_very_moment_of_the_other_is_not_taken() -> None:
    # Of two with the same time nobody can say which was first, and taking a stranger ends
    # a program that has nothing to do with a run.
    found = descended({10: 500}, {11: 10}, times({11: 500}))

    assert found == {}


def test_a_held_process_is_not_among_what_is_found() -> None:
    found = descended({10: 100, 11: 200}, {11: 10, 12: 11}, times({12: 300}))

    assert found == {12: 300}


def test_a_process_whose_time_cannot_be_found_out_is_left_alone_with_all_it_made() -> None:
    found = descended({10: 100}, {11: 10, 12: 11}, times({12: 300}))

    assert found == {}


def test_what_descends_from_nothing_held_is_left_alone() -> None:
    # The one that made the held process, another that it made, what that one made, and
    # the first process of the machine, which names itself.
    parents = {0: 0, 1: 0, 10: 1, 20: 1, 21: 20}

    found = descended({10: 100}, parents, times({0: 1, 1: 2, 20: 150, 21: 250}))

    assert found == {}


def test_two_that_name_each_other_do_not_keep_the_search_going() -> None:
    found = descended({10: 100}, {10: 11, 11: 10}, times({11: 200}))

    assert found == {11: 200}


def test_only_what_names_a_process_of_the_run_is_asked_about() -> None:
    # A machine has hundreds of processes, and each question opens one of them.
    asked: list[int] = []
    parents = {process: process - 1 for process in range(1000, 2000)}
    parents[11] = 10
    parents[12] = 11

    def made(process: int) -> int:
        asked.append(process)
        return 100 + process

    found = descended({10: 100}, parents, made)

    assert found == {11: 111, 12: 112}
    assert asked == [11, 12]


@st.composite
def machines(draw: st.DrawFn) -> tuple[dict[int, int], dict[int, int], set[int]]:
    """Processes with the ones they name and their times, and some of them that are held.

    A process with a higher number is younger. Most name an older one, as a child names
    whoever made it. Some name a younger one, as a process does whose maker is gone and
    whose maker's number was given out again.
    """
    count = draw(st.integers(min_value=1, max_value=24))
    parents = {
        process: draw(st.integers(min_value=0, max_value=count)) for process in range(1, count + 1)
    }
    made = {process: 10 * process for process in parents}
    held = draw(st.sets(st.sampled_from(sorted(parents)), max_size=4))
    return parents, made, held


@given(machines())
def test_what_is_found_is_what_can_be_reached_by_children_younger_than_their_makers(
    machine: tuple[dict[int, int], dict[int, int], set[int]],
) -> None:
    parents, made, held = machine
    reached = set(held)
    grew = True
    while grew:
        grew = False
        for process, parent in parents.items():
            if process not in reached and parent in reached and made[process] > made[parent]:
                reached.add(process)
                grew = True

    found = descended({process: made[process] for process in held}, parents, times(made))

    assert found == {process: made[process] for process in reached - held}
