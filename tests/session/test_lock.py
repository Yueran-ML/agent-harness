"""The file lock that keeps a session to one writer."""

from pathlib import Path

from columbina.session.lock import acquire


def test_a_free_lock_can_be_taken(tmp_path: Path) -> None:
    lock = acquire(tmp_path / "lock")

    assert lock is not None
    assert (tmp_path / "lock").exists()
    lock.release()


def test_a_lock_that_is_held_cannot_be_taken_again(tmp_path: Path) -> None:
    held = acquire(tmp_path / "lock")
    assert held is not None

    assert acquire(tmp_path / "lock") is None

    held.release()


def test_a_released_lock_can_be_taken_again(tmp_path: Path) -> None:
    first = acquire(tmp_path / "lock")
    assert first is not None
    first.release()

    second = acquire(tmp_path / "lock")

    assert second is not None
    second.release()


def test_releasing_twice_is_harmless(tmp_path: Path) -> None:
    lock = acquire(tmp_path / "lock")
    assert lock is not None

    lock.release()
    lock.release()

    again = acquire(tmp_path / "lock")
    assert again is not None
    again.release()


def test_two_locks_do_not_get_in_each_others_way(tmp_path: Path) -> None:
    one = acquire(tmp_path / "one")
    other = acquire(tmp_path / "other")

    assert one is not None
    assert other is not None
    one.release()
    other.release()
