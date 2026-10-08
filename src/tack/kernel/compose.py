"""Compositions: an application described as a list of plugin entries.

An application is assembled from layers. A layer is a list of operations: an
operation inserts entries, or changes an entry that an earlier operation
inserted. The first layer may extend a named layer, which is then applied
before it. The layers after the first are patches on top.

In YAML a layer is either a bare list of operations or a mapping::

    extends: base
    patch:
      - insert:
          - id: cli
            plugin: tack.apps.cli
      - id: llm
        config:
          model: deepseek-flash
"""

from __future__ import annotations

import importlib
import os
import re
from collections.abc import Callable, Mapping, Sequence
from typing import Annotated, Any, cast

import yaml
from pydantic import (
    BaseModel,
    ConfigDict,
    Discriminator,
    PrivateAttr,
    Tag,
    ValidationError,
    model_validator,
)

from tack.kernel.context import Context
from tack.kernel.errors import CompositionError, InvalidPluginError
from tack.kernel.fiber import Fiber, FiberState
from tack.kernel.plugin import Plugin, resolve_plugin


class Entry(BaseModel):
    """One plugin of an application.

    Attributes:
        id: The name by which later operations address the entry.
        plugin: Where to import the plugin from: ``package.module`` for a
            module that is itself a plugin, or ``package.module:attribute``.
        config: The raw config handed to the plugin. Strings in it may refer
            to environment variables as ``${NAME}`` or ``${NAME:-default}``.
        disabled: Whether to leave the plugin out.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    plugin: str
    config: Any = None
    disabled: bool = False


class Insert(BaseModel):
    """An operation that adds entries."""

    model_config = ConfigDict(frozen=True, extra="forbid")

    insert: tuple[Entry, ...]


class Change(BaseModel):
    """An operation that replaces fields of the entry called ``id``.

    Only the fields that are written out are replaced. ``config`` is replaced
    as a whole, never merged, and may be set to null to clear it.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    plugin: str | None = None
    config: Any = None
    disabled: bool | None = None

    @model_validator(mode="after")
    def _only_config_may_be_null(self) -> Change:
        for name in ("plugin", "disabled"):
            if name in self.model_fields_set and getattr(self, name) is None:
                raise ValueError(f"{name} cannot be null; leave it out to keep what the entry has")
        return self


def _kind_of(operation: object) -> str:
    if isinstance(operation, Mapping):
        return "insert" if "insert" in operation else "change"
    return "insert" if isinstance(operation, Insert) else "change"


Operation = Annotated[
    Annotated[Insert, Tag("insert")] | Annotated[Change, Tag("change")],
    Discriminator(_kind_of),
]


class Layer(BaseModel):
    """A list of operations, optionally on top of a named layer.

    Attributes:
        extends: The name of the layer to apply first, if any.
        patch: The operations, applied in order.
    """

    model_config = ConfigDict(frozen=True, extra="forbid")

    extends: str | None = None
    patch: tuple[Operation, ...] = ()

    _source: str = PrivateAttr(default="<layer>")

    @property
    def source(self) -> str:
        """Where the layer was read from. Error messages start with it."""
        return self._source


def parse_layer(text: str, *, source: str = "<layer>") -> Layer:
    """Read a layer from YAML.

    Args:
        text: The YAML document.
        source: A name for the document, such as its path, for error messages.

    Raises:
        CompositionError: The document is not YAML or does not describe a layer.
    """
    try:
        data: object = yaml.safe_load(text)
    except yaml.YAMLError as error:
        raise CompositionError(f"{source}: not valid YAML: {error}") from error
    if data is None:
        data = {}
    elif isinstance(data, list):
        data = {"patch": cast("list[object]", data)}
    elif not isinstance(data, dict):
        raise CompositionError(
            f"{source}: a layer is a list of operations, or a mapping with 'extends' and 'patch'"
        )
    try:
        layer = Layer.model_validate(data)
    except ValidationError as error:
        raise CompositionError(f"{source}: {error}") from error
    layer._source = source  # pyright: ignore[reportPrivateUsage]
    return layer


def flatten(layers: Sequence[Layer], lookup: Callable[[str], Layer] | None = None) -> list[Entry]:
    """Turn a stack of layers into one list of entries.

    The layers are applied in order. Only the first may extend another layer.

    Args:
        layers: The first layer, followed by any patches.
        lookup: Finds a layer by the name given in ``extends``. It raises
            ``KeyError`` for a name it does not know.

    Raises:
        CompositionError: An operation cannot be applied, a layer that is
            extended does not exist, or the layers extend one another in a
            circle.
    """
    entries: list[Entry] = []
    for position, layer in enumerate(layers):
        if position == 0:
            for ancestor in reversed(_ancestors(layer, lookup)):
                entries = _apply(entries, ancestor)
        elif layer.extends is not None:
            raise CompositionError(f"{layer.source}: only the first layer may extend another")
        entries = _apply(entries, layer)
    return entries


def _ancestors(layer: Layer, lookup: Callable[[str], Layer] | None) -> list[Layer]:
    """Return the layers that ``layer`` extends, nearest first."""
    ancestors: list[Layer] = []
    visited: list[str] = []
    current = layer
    while current.extends is not None:
        name = current.extends
        if name in visited:
            circle = " -> ".join([*visited[visited.index(name) :], name])
            raise CompositionError(
                f"{layer.source}: the layers extend one another in a circle: {circle}"
            )
        visited.append(name)
        try:
            if lookup is None:
                raise KeyError(name)
            current = lookup(name)
        except KeyError:
            raise CompositionError(
                f"{current.source}: there is no layer {name!r} to extend"
            ) from None
        ancestors.append(current)
    return ancestors


def _apply(entries: list[Entry], layer: Layer) -> list[Entry]:
    result = list(entries)
    positions = {entry.id: position for position, entry in enumerate(result)}
    for operation in layer.patch:
        if isinstance(operation, Insert):
            for entry in operation.insert:
                if entry.id in positions:
                    raise CompositionError(f"{layer.source}: entry {entry.id!r} already exists")
                positions[entry.id] = len(result)
                result.append(entry)
            continue
        position = positions.get(operation.id)
        if position is None:
            raise CompositionError(f"{layer.source}: there is no entry {operation.id!r} to change")
        changes = {name: getattr(operation, name) for name in operation.model_fields_set}
        del changes["id"]
        result[position] = result[position].model_copy(update=changes)
    return result


_REFERENCE = re.compile(
    r"\$\$"
    r"|\$\{(?P<name>[A-Za-z_][A-Za-z0-9_]*)(?::-(?P<default>[^{}$]*))?\}"
    r"|(?P<malformed>\$\{)"
)


def interpolate(value: Any, env: Mapping[str, str]) -> Any:
    """Return ``value`` with environment variables filled into its strings.

    ``${NAME}`` becomes the value of the variable, ``${NAME:-default}`` falls
    back to the default when the variable is not set, and ``$$`` is a literal
    dollar sign. A default is taken as written and cannot contain braces or
    dollar signs. Mappings and lists are searched all the way down. Everything
    that is not a string is returned unchanged.

    Raises:
        CompositionError: A variable is not set and has no default, or a
            ``${`` does not start a reference of one of these forms.
    """
    if isinstance(value, str):

        def fill(match: re.Match[str]) -> str:
            if match["malformed"] is not None:
                raise CompositionError(
                    f"cannot read the variable reference in {value!r}: write ${{NAME}} or "
                    "${NAME:-default}, without braces or dollar signs in the default, "
                    "and $$ for a dollar sign"
                )
            name = match["name"]
            if name is None:
                return "$"
            if name in env:
                return env[name]
            default = match["default"]
            if default is None:
                raise CompositionError(f"environment variable {name!r} is not set")
            return default

        return _REFERENCE.sub(fill, value)
    if isinstance(value, dict):
        items = cast("dict[Any, Any]", value)
        return {key: interpolate(item, env) for key, item in items.items()}
    if isinstance(value, list):
        return [interpolate(item, env) for item in cast("list[Any]", value)]
    return value


def import_plugin(path: str) -> Plugin[Any]:
    """Import the plugin that an entry names.

    Args:
        path: ``package.module`` for a module that is itself a plugin, or
            ``package.module:attribute`` for a plugin kept in a module.

    Raises:
        CompositionError: The module cannot be imported, the attribute does not
            exist, or what was found is not a plugin.
    """
    module_name, colon, attribute = path.partition(":")
    if not module_name or module_name.startswith(".") or (colon and not attribute):
        raise CompositionError(
            f"{path!r} is not an import path: write 'package.module' or 'package.module:attribute'"
        )
    try:
        target: object = importlib.import_module(module_name)
    except Exception as error:
        # Importing runs the module, which may raise anything.
        raise CompositionError(
            f"cannot import {module_name!r}: {type(error).__name__}: {error}"
        ) from error
    if attribute:
        if not hasattr(target, attribute):
            raise CompositionError(f"module {module_name!r} has no attribute {attribute!r}")
        target = getattr(target, attribute)
    try:
        return resolve_plugin(target)
    except InvalidPluginError as error:
        raise CompositionError(f"{path!r} is not a plugin: {error}") from error


class Composition:
    """The plugins that :func:`mount` loaded.

    Attributes:
        fibers: The fiber of each entry that was loaded, by entry id, in the
            order of the entries.
    """

    __slots__ = ("_entries", "fibers")

    def __init__(self, loaded: Sequence[tuple[Entry, Fiber]]) -> None:
        self._entries = {entry.id: entry for entry, _ in loaded}
        self.fibers = {entry.id: fiber for entry, fiber in loaded}

    def problems(self) -> list[str]:
        """Describe each loaded entry that is not active, in entry order.

        Call this once the application has settled. An entry is then inactive
        because it failed to load or because a service it injects is missing.
        """
        problems: list[str] = []
        for entry_id, fiber in self.fibers.items():
            label = f"{entry_id} ({self._entries[entry_id].plugin})"
            state = fiber.state
            missing = [repr(key.name) for key in fiber.missing]
            if state is FiberState.ACTIVE:
                continue
            if state is FiberState.FAILED:
                error = fiber.error
                problems.append(f"{label} failed: {type(error).__name__}: {error}")
            elif state is FiberState.PENDING and missing:
                noun = "service" if len(missing) == 1 else "services"
                problems.append(f"{label} is waiting for the {noun} {', '.join(missing)}")
            else:
                problems.append(f"{label} is {state.value}")
        return problems


def mount(
    ctx: Context, entries: Sequence[Entry], *, env: Mapping[str, str] | None = None
) -> Composition:
    """Load the enabled entries as plugins of ``ctx``.

    Every plugin is imported and every config filled in before anything is
    loaded, so a mistake in one entry leaves the context as it was. The order
    of the entries does not decide the order in which plugins become active:
    each loads as soon as the services it injects are available.

    Args:
        ctx: The context that will own the plugins.
        entries: The entries, as returned by :func:`flatten`.
        env: The variables for ``${NAME}`` references. Defaults to the
            environment of the process.

    Raises:
        CompositionError: Two entries share an id, a plugin cannot be imported,
            or a config refers to a variable that is not set.
        InactiveEffectError: The fiber that owns ``ctx`` is not loaded.
    """
    variables = os.environ if env is None else env
    seen: set[str] = set()
    prepared: list[tuple[Entry, Plugin[Any], Any]] = []
    for entry in entries:
        if entry.id in seen:
            raise CompositionError(f"entry {entry.id!r} appears twice")
        seen.add(entry.id)
        if entry.disabled:
            continue
        try:
            plugin = import_plugin(entry.plugin)
            config = interpolate(entry.config, variables)
        except CompositionError as error:
            raise CompositionError(f"entry {entry.id!r}: {error}") from error
        prepared.append((entry, plugin, config))
    return Composition([(entry, ctx.plugin(plugin, config)) for entry, plugin, config in prepared])
