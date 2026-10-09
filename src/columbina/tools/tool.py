"""A tool: something a model can ask the harness to do."""

from __future__ import annotations

import json
import re
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from functools import cached_property
from pathlib import Path
from typing import Any, Generic, Literal, TypeVar, cast, get_args

from pydantic import BaseModel, JsonValue, ValidationError

from columbina.llm.types import ToolSchema
from columbina.session.log import Session

ArgsT = TypeVar("ArgsT", bound=BaseModel)
ValueT = TypeVar("ValueT")

_NAME = re.compile(r"[A-Za-z0-9_-]{1,64}")
_JSON_KINDS: dict[type[object], str] = {
    list: "a list",
    str: "a string",
    int: "a number",
    float: "a number",
    bool: "a boolean",
    type(None): "null",
}
_SUBSCHEMAS = ("properties", "$defs")
_SUBSCHEMA_LISTS = ("anyOf", "allOf", "oneOf", "prefixItems")


class ToolFailure(Exception):  # noqa: N818 - a failure that a tool reports, not a bug
    """Raised by a tool to tell the model that a call did not succeed.

    The message is what the model reads, so it should say what to do next.

    Attributes:
        code: A stable code for programs, such as ``INVALID_ARGS``.
    """

    def __init__(self, message: str, *, code: str = "TOOL_ERROR") -> None:
        super().__init__(message)
        self.code = code


class ToolRun:
    """What a tool is told about the call it serves, and what it may ask for.

    What it is told is fixed when the run is made. A run passes through the
    hands of listeners on its way to the tool, and none of them can change
    the working directory or the target under it.

    Args:
        call_id: The id that the model gave the call.
        cwd: The working directory of the agent.
        session: The session that the call belongs to.
        target: What the checks were told that the call acts on. The pipeline
            fills this in. A tool that works on a file compares it with what
            it finds when it runs, and stops if the two differ.
    """

    __slots__ = ("_call_id", "_concludes_turn", "_cwd", "_session", "_target")

    def __init__(
        self, call_id: str, cwd: Path, session: Session, target: str | None = None
    ) -> None:
        self._call_id = call_id
        self._cwd = cwd
        self._session = session
        self._target = target
        self._concludes_turn = False

    @property
    def call_id(self) -> str:
        """The id that the model gave the call."""
        return self._call_id

    @property
    def cwd(self) -> Path:
        """The working directory of the agent."""
        return self._cwd

    @property
    def session(self) -> Session:
        """The session that the call belongs to."""
        return self._session

    @property
    def target(self) -> str | None:
        """What the checks were told that the call acts on, if the tool says such a thing."""
        return self._target

    @property
    def concludes_turn(self) -> bool:
        """Whether the tool asked for the turn to end."""
        return self._concludes_turn

    def conclude_turn(self) -> None:
        """Ask for the turn to end once the results of this step are in."""
        self._concludes_turn = True


def _without_titles(schema: dict[str, Any]) -> dict[str, JsonValue]:
    """Return a JSON Schema without the titles that pydantic adds to it.

    A model does not need them, and they cost tokens on every request. A
    parameter that happens to be called ``title`` is kept.
    """
    cleaned: dict[str, JsonValue] = {}
    for key, value in schema.items():
        if key == "title":
            continue
        if key in _SUBSCHEMAS:
            cleaned[key] = {name: _without_titles(sub) for name, sub in value.items()}
        elif key in _SUBSCHEMA_LISTS:
            cleaned[key] = [_without_titles(sub) for sub in value]
        elif key == "items" and isinstance(value, dict):
            cleaned[key] = _without_titles(cast("dict[str, Any]", value))
        else:
            cleaned[key] = value
    return cleaned


Effect = Literal["none", "read", "write", "run"]
"""What a call of a tool does to the world outside the conversation."""


@dataclass(frozen=True)
class Tool(Generic[ArgsT, ValueT]):
    """Something a model can ask the harness to do.

    Attributes:
        name: The name by which the model calls the tool.
        description: What the model is told about the tool.
        parameters: The pydantic model of the arguments. The model is shown
            its JSON Schema, and the arguments of a call are validated
            against it before the tool runs.
        execute: Does the work. It gets the validated arguments and a
            :class:`ToolRun`. To tell the model that the call did not
            succeed, it raises :class:`ToolFailure`.
        render: Turns the value that ``execute`` returned into the text that
            the model reads. By default the value is converted with ``str``.
        timeout_seconds: How long a call may take. By default the limit of
            the tool service applies.
        concurrency_safe: Whether a call may run beside other calls of the
            same reply. Either a fixed answer or a function of the arguments.
            A tool that changes anything is not safe.
        effect: What a call does to the world outside the conversation:
            nothing, it reads, it writes, or it runs a program. Checks that
            decide whether a call may run go by this.
        target: Says what a call acts on, as text that a rule can be matched
            against: a path for a tool that works on a file, the command for
            a tool that runs one.
        paged: Whether the tool hands out what it has in pages: it bounds
            what one call returns by itself, and says how to ask for more.
            Whatever cuts a large result short leaves such a tool alone.

    Raises:
        ValueError: The name is not one that every provider accepts, or the
            effect is none of the four.
        TypeError: ``parameters`` is not a pydantic model.
    """

    name: str
    description: str
    parameters: type[ArgsT]
    execute: Callable[[ArgsT, ToolRun], Awaitable[ValueT]]
    render: Callable[[ArgsT, ValueT], str] | None = None
    timeout_seconds: float | None = None
    concurrency_safe: bool | Callable[[ArgsT], bool] = False
    effect: Effect = "none"
    target: Callable[[ArgsT, ToolRun], str] | None = None
    paged: bool = False

    def __post_init__(self) -> None:
        if self.effect not in get_args(Effect):
            raise ValueError(
                f"{self.effect!r} is not an effect: a tool says none, read, write or run"
            )
        if _NAME.fullmatch(self.name) is None:
            raise ValueError(
                f"{self.name!r} is not a usable tool name: it may hold up to 64 letters, "
                "digits, dashes and underscores"
            )
        parameters = cast("object", self.parameters)
        if not (isinstance(parameters, type) and issubclass(parameters, BaseModel)):
            raise TypeError(f"the parameters of the tool {self.name!r} must be a pydantic model")

    @cached_property
    def schema(self) -> ToolSchema:
        """What a model is told about the tool."""
        parameters = _without_titles(self.parameters.model_json_schema())
        parameters.pop("description", None)
        return ToolSchema(name=self.name, description=self.description, parameters=parameters)

    def parse(self, arguments: str) -> ArgsT:
        """Read the arguments of a call, as the model wrote them.

        Raises:
            ToolFailure: The arguments are not a JSON object, or they do not
                fit the parameters. The code is ``INVALID_ARGS``.
        """
        text = arguments if arguments.strip() else "{}"
        try:
            given: object = json.loads(text)
        except ValueError as error:
            raise ToolFailure(
                f"The arguments are not valid JSON: {error}", code="INVALID_ARGS"
            ) from error
        if not isinstance(given, dict):
            kind = _JSON_KINDS[type(given)]
            raise ToolFailure(
                f"The arguments must be a JSON object, not {kind}.", code="INVALID_ARGS"
            )
        try:
            # Validated as JSON, not as Python objects: a strict model takes a JSON
            # array for a tuple and a JSON string for a date only this way.
            return self.parameters.model_validate_json(text)
        except ValidationError as error:
            problems = "\n".join(
                f"- {'.'.join(str(part) for part in problem['loc'])}: {problem['msg']}"
                for problem in error.errors()
            )
            raise ToolFailure(
                f"The arguments do not fit the tool {self.name}:\n{problems}", code="INVALID_ARGS"
            ) from error

    def is_concurrency_safe(self, args: ArgsT) -> bool:
        """Tell whether a call with ``args`` may run beside other calls of the same reply."""
        safe = self.concurrency_safe
        return safe if isinstance(safe, bool) else safe(args)
