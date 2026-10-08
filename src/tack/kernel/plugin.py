"""Plugins: the units the kernel loads and unloads."""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, TypeAlias, cast

from pydantic import BaseModel

from tack.kernel.errors import InvalidPluginError
from tack.kernel.services import ServiceKey

if TYPE_CHECKING:
    from tack.kernel.context import Context
    from tack.kernel.fiber import Disposer

ApplyResult: TypeAlias = "Disposer | None"
Apply: TypeAlias = "Callable[[Context, Any], ApplyResult | Awaitable[ApplyResult]]"


@dataclass(frozen=True, slots=True)
class Plugin:
    """A unit of functionality that the kernel can load and unload.

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
    """

    name: str
    apply: Apply
    inject: tuple[ServiceKey[Any], ...] = field(default=(), kw_only=True)
    config: type[BaseModel] | None = field(default=None, kw_only=True)


def resolve_plugin(candidate: object) -> Plugin:
    """Turn any accepted plugin shape into a :class:`Plugin`.

    Three shapes are accepted: a :class:`Plugin`; a module or object with an
    ``apply`` callable and optional ``name``, ``inject`` and ``Config``
    attributes; and a bare callable, which is its own ``apply``.

    Raises:
        InvalidPluginError: ``candidate`` has none of these shapes.
    """
    if isinstance(candidate, Plugin):
        return candidate
    apply = getattr(candidate, "apply", candidate)
    if not callable(apply):
        raise InvalidPluginError(
            f"{candidate!r} is not a plugin: expected a callable or an object with apply()"
        )
    name = getattr(candidate, "name", None) or getattr(candidate, "__name__", None)
    if name is None:
        name = type(candidate).__name__
    if not isinstance(name, str):
        raise InvalidPluginError(f"the name of plugin {candidate!r} must be a string")
    config = getattr(candidate, "Config", None)
    if config is not None and not (isinstance(config, type) and issubclass(config, BaseModel)):
        raise InvalidPluginError(f"the Config of plugin {name!r} must be a pydantic model")
    declared: object = getattr(candidate, "inject", ())
    if not isinstance(declared, list | tuple):
        raise InvalidPluginError(f"the inject of plugin {name!r} must be a list of service keys")
    inject: list[ServiceKey[Any]] = []
    for key in cast("Sequence[object]", declared):
        if not isinstance(key, ServiceKey):
            raise InvalidPluginError(
                f"the inject of plugin {name!r} must be a list of service keys"
            )
        inject.append(cast("ServiceKey[Any]", key))
    return Plugin(name, cast("Apply", apply), inject=tuple(inject), config=config)
