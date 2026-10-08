"""A tool: something a model can ask the harness to do."""

from __future__ import annotations

import json
import re
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from functools import cached_property
from pathlib import Path
from typing import Any, Generic, TypeVar, cast

from pydantic import BaseModel, JsonValue, ValidationError

from tack.llm.types import ToolSchema
from tack.session.log import Session

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

    Attributes:
        call_id: The id that the model gave the call.
        cwd: The working directory of the agent.
        session: The session that the call belongs to.
    """

    __slots__ = ("_concludes_turn", "call_id", "cwd", "session")

    def __init__(self, call_id: str, cwd: Path, session: Session) -> None:
        self.call_id = call_id
        self.cwd = cwd
        self.session = session
        self._concludes_turn = False

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

    Raises:
        ValueError: The name is not one that every provider accepts.
        TypeError: ``parameters`` is not a pydantic model.
    """

    name: str
    description: str
    parameters: type[ArgsT]
    execute: Callable[[ArgsT, ToolRun], Awaitable[ValueT]]
    render: Callable[[ArgsT, ValueT], str] | None = None
    timeout_seconds: float | None = None
    concurrency_safe: bool | Callable[[ArgsT], bool] = False

    def __post_init__(self) -> None:
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
        try:
            given: object = json.loads(arguments) if arguments.strip() else {}
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
            return self.parameters.model_validate(given)
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
