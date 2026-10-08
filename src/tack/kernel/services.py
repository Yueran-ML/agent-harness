"""Typed service names."""

from dataclasses import dataclass
from typing import Generic, TypeVar

ServiceT = TypeVar("ServiceT")


@dataclass(frozen=True, slots=True)
class ServiceKey(Generic[ServiceT]):
    """The name of a service slot.

    The type parameter records what the slot holds, so that type checkers can
    follow a service from ``provide`` to ``use``. Keys compare by name: two keys
    with the same name address the same slot.
    """

    name: str
