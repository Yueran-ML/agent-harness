"""Plugins: the units the kernel loads and unloads."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Generic, TypeAlias, TypeVar, cast

from pydantic import BaseModel

from columbina.kernel.errors import InvalidPluginError
from columbina.kernel.fiber import Disposer
from columbina.kernel.services import ServiceKey

if TYPE_CHECKING:
    from columbina.kernel.context import Context

ConfigT = TypeVar("ConfigT")

ApplyResult: TypeAlias = Disposer | None
"""What ``apply`` returns: a disposer that runs first on unload, or nothing."""

Apply: TypeAlias = Callable[["Context", ConfigT], ApplyResult | Awaitable[ApplyResult]]
"""The signature of ``apply``. It may be a plain function or a coroutine function."""


@dataclass(frozen=True, slots=True)
class Plugin(Generic[ConfigT]):
    """A unit of functionality that the kernel can load and unload.

    The type parameter is the type of the config that ``apply`` accepts. When a
    config model is given, type checkers require it to be that type.

    Attributes:
        name: A label for logs and error messages.
        apply: Called with the plugin's own context and its config each time
            the plugin loads. It may be a coroutine function. It may return a
            disposer, which runs first when the plugin unloads.
        inject: The services the plugin requires. The plugin loads only while
            all of them are available, unloads when one is withdrawn, and
            loads again when it returns.
        config: A pydantic model that the raw config is validated against, or
            ``None`` to hand the raw config to ``apply`` unchanged.

    Raises:
        InvalidPluginError: A field has the wrong shape.
    """

    name: str
    apply: Apply[ConfigT]
    inject: tuple[ServiceKey[Any], ...] = field(default=(), kw_only=True)
    config: type[ConfigT] | None = field(default=None, kw_only=True)

    def __post_init__(self) -> None:
        # The annotations describe what a type checker enforces. Callers that
        # are not type-checked, such as resolve_plugin, are checked here.
        name = cast("object", self.name)
        if not isinstance(name, str) or not name:
            raise InvalidPluginError(f"a plugin needs a name, not {name!r}")
        if not callable(cast("object", self.apply)):
            raise InvalidPluginError(f"the apply of plugin {name!r} must be callable")
        config = cast("object", self.config)
        if config is not None and not (isinstance(config, type) and issubclass(config, BaseModel)):
            raise InvalidPluginError(f"the config of plugin {name!r} must be a pydantic model")
        inject = cast("object", self.inject)
        keys = cast("Sequence[object]", inject) if isinstance(inject, list | tuple) else None
        if keys is None or not all(isinstance(key, ServiceKey) for key in keys):
            raise InvalidPluginError(
                f"the inject of plugin {name!r} must be a list or tuple of service keys"
            )
        object.__setattr__(self, "inject", tuple(keys))


def resolve_plugin(candidate: object) -> Plugin[Any]:
    """Turn any accepted plugin shape into a :class:`Plugin`.

    Three shapes are accepted: a :class:`Plugin`; a module or object with an
    ``apply`` callable and optional ``name``, ``inject`` and ``Config``
    attributes; and a bare callable, which is its own ``apply``.

    Raises:
        InvalidPluginError: ``candidate`` has none of these shapes.
    """
    if isinstance(candidate, Plugin):
        return cast("Plugin[Any]", candidate)
    apply = getattr(candidate, "apply", candidate)
    if not callable(apply):
        raise InvalidPluginError(
            f"{candidate!r} is not a plugin: expected a callable or an object with apply()"
        )
    name = getattr(candidate, "name", None) or getattr(candidate, "__name__", None)
    if name is None:
        name = type(candidate).__name__
    return Plugin(
        name,
        cast("Apply[Any]", apply),
        inject=getattr(candidate, "inject", ()),
        config=getattr(candidate, "Config", None),
    )
