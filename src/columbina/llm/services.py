"""The services that a model can come from, by name.

Many services speak the OpenAI Chat Completions protocol, and one adapter
talks to all of them. They differ in little: where they are, which
environment variable holds their key, and a few habits. What is known of a
service is written down here once, so that a user names a service and not
its address.

A service that is not listed here needs no entry: its address is enough for
the adapter, and :func:`of_ones_own` makes a :class:`Service` of it.

What stands here was read in the documents of each service. Of the services
that are listed, the tests of this project talk only to DeepSeek.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any, Literal

__all__ = [
    "KEY_OF_ONES_OWN",
    "PRESETS",
    "Service",
    "adapter_config",
    "of_ones_own",
    "preset",
]

KEY_OF_ONES_OWN = "COLUMBINA_API_KEY"
"""The environment variable that may hold the key of a service that is not listed here."""


@dataclass(frozen=True, slots=True)
class Service:
    """A service that speaks the OpenAI Chat Completions protocol.

    Attributes:
        name: The name by which a user chooses the service. Its adapter is
            registered under it, and its stored key is kept under it.
        title: What the service is called, for a list.
        base_url: Its address, up to but without ``/chat/completions``.
        key_env: The environment variable that may hold its key, or ``None``
            if no variable does.
        key_required: Whether the service is talked to only with a key. One
            that runs on the machine of the user asks for none.
        reasoning_style: How the service is told how much to reason.
        max_tokens_field: The name under which it takes the limit on the
            length of a reply.
        model: The model that is used where nothing names one, or ``None``
            if a model has to be named.
        context_window: How many tokens that model takes, or ``None`` if
            that is not known.
        base_url_env: An environment variable that may name another address
            for the service, or ``None``.
    """

    name: str
    title: str
    base_url: str
    key_env: str | None = None
    key_required: bool = True
    reasoning_style: Literal["deepseek", "openai", "none"] = "none"
    max_tokens_field: Literal["max_tokens", "max_completion_tokens"] = "max_tokens"
    model: str | None = None
    context_window: int | None = None
    base_url_env: str | None = None


PRESETS: tuple[Service, ...] = (
    Service(
        name="deepseek",
        title="DeepSeek",
        base_url="https://api.deepseek.com",
        key_env="DEEPSEEK_API_KEY",
        reasoning_style="deepseek",
        model="deepseek-flash",
        context_window=1_048_576,
        base_url_env="DEEPSEEK_BASE_URL",
    ),
    Service(
        name="openai",
        title="OpenAI",
        base_url="https://api.openai.com/v1",
        key_env="OPENAI_API_KEY",
        reasoning_style="openai",
        # The models that reason refuse the older name, and the others take both.
        max_tokens_field="max_completion_tokens",
    ),
    Service(
        name="openrouter",
        title="OpenRouter",
        base_url="https://openrouter.ai/api/v1",
        key_env="OPENROUTER_API_KEY",
    ),
    Service(
        name="moonshot",
        title="Moonshot AI (Kimi)",
        base_url="https://api.moonshot.cn/v1",
        key_env="MOONSHOT_API_KEY",
    ),
    Service(
        name="zhipu",
        title="Zhipu AI (GLM)",
        base_url="https://open.bigmodel.cn/api/paas/v4",
        key_env="ZAI_API_KEY",
    ),
    Service(
        name="siliconflow",
        title="SiliconFlow",
        base_url="https://api.siliconflow.cn/v1",
        key_env="SILICONFLOW_API_KEY",
    ),
    Service(
        name="ollama",
        title="Ollama on this machine",
        base_url="http://localhost:11434/v1",
        key_required=False,
    ),
    Service(
        name="lmstudio",
        title="LM Studio on this machine",
        base_url="http://localhost:1234/v1",
        key_required=False,
    ),
)
"""The services that are known by name, in the order in which they are listed."""


def preset(name: str) -> Service | None:
    """Return the service that is known by this name, or ``None`` if none is."""
    return next((service for service in PRESETS if service.name == name), None)


def of_ones_own(name: str, base_url: str) -> Service:
    """Return a service that is not listed here, from its address.

    Nothing is known of it but where it is. So nothing is assumed: it is
    sent a key if one is found, and it is not told how much to reason.

    Args:
        name: The name that its user gave it.
        base_url: Its address, up to but without ``/chat/completions``.
    """
    return Service(
        name=name, title=base_url, base_url=base_url, key_env=KEY_OF_ONES_OWN, key_required=False
    )


def _as_written(text: str) -> str:
    """Return a text so that a config holds it as it is: its dollar signs stand for themselves."""
    return text.replace("$", "$$")


def adapter_config(service: Service, keys: str, env: Mapping[str, str]) -> dict[str, Any]:
    """Return the config of the adapter that talks to a service.

    Args:
        service: The service.
        keys: Where the file of stored keys lies, as a config says it. It
            may refer to environment variables.
        env: The environment. The variable of the service that names another
            address is looked up here.

    Returns:
        The config, as it stands in a plugin entry.
    """
    base_url = service.base_url
    if service.base_url_env is not None:
        base_url = env.get(service.base_url_env) or base_url
    config: dict[str, Any] = {"provider": service.name, "base_url": _as_written(base_url)}
    if service.key_env is not None:
        config["api_key_env"] = service.key_env
    config["api_key_file"] = keys
    if not service.key_required:
        config["api_key_required"] = False
    if service.reasoning_style != "none":
        config["reasoning_style"] = service.reasoning_style
    if service.max_tokens_field != "max_tokens":
        config["max_tokens_field"] = service.max_tokens_field
    return config
