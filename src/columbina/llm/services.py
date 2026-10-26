"""The services that a model can come from, by name.

Many services speak the OpenAI Chat Completions protocol, and one adapter
talks to all of them. They differ in little: where they are, which
environment variable holds their key, and a few habits. Anthropic is spoken
to in its own protocol, by an adapter of its own, see
:mod:`columbina.llm.adapters.anthropic_messages`. What is known of a service
is written down here once, so that a user names a service and not its
address.

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
    """A service that a model comes from.

    Attributes:
        name: The name by which a user chooses the service. Its adapter is
            registered under it, and its stored key is kept under it.
        title: What the service is called, for a list.
        base_url: Its address, up to but without ``/chat/completions``, or
            for Anthropic without ``/v1``.
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
        key_page: The page of the service on which a key is made, as its own
            documents link to it, or ``None`` for a service that asks for none.
        effort: How hard its model is to think, as the user chose it for the
            service: ``default`` asks for nothing, and the others ask for that
            much reasoning. ``None`` where the user chose nothing, and the
            profile decides.
        protocol: What it speaks: the protocol of OpenAI, or that of
            Anthropic. The first is spoken to by
            :mod:`columbina.llm.adapters.openai_chat`, the second by
            :mod:`columbina.llm.adapters.anthropic_messages`, which knows how
            the service is told how much to reason.
        balance: How the service is asked how much money its account has
            left, or ``None`` if it cannot be asked with its key: ``deepseek``
            for ``GET /user/balance``.
        drops_unbound_thinking: Whether the service can be asked to leave out
            thinking that was bound to a beginning of the conversation that
            has changed since, as a summary changes it, rather than refuse
            the request.
        pictures: Whether its models take pictures, over the protocol of
            OpenAI. Those of Anthropic all do.
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
    key_page: str | None = None
    effort: Literal["default", "off", "low", "high", "max"] | None = None
    protocol: Literal["openai-chat", "anthropic"] = "openai-chat"
    balance: Literal["deepseek"] | None = None
    drops_unbound_thinking: bool = False
    pictures: bool = False


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
        key_page="https://platform.deepseek.com/api_keys",
        balance="deepseek",
    ),
    Service(
        name="openai",
        title="OpenAI",
        base_url="https://api.openai.com/v1",
        key_env="OPENAI_API_KEY",
        reasoning_style="openai",
        # The models that reason refuse the older name, and the others take both.
        max_tokens_field="max_completion_tokens",
        key_page="https://platform.openai.com/api-keys",
        pictures=True,
    ),
    Service(
        name="anthropic",
        title="Anthropic (Claude)",
        base_url="https://api.anthropic.com",
        key_env="ANTHROPIC_API_KEY",
        model="claude-opus-5-5",
        context_window=1_000_000,
        base_url_env="ANTHROPIC_BASE_URL",
        key_page="https://platform.claude.com/settings/keys",
        protocol="anthropic",
        drops_unbound_thinking=True,
    ),
    Service(
        name="openrouter",
        title="OpenRouter",
        base_url="https://openrouter.ai/api/v1",
        key_env="OPENROUTER_API_KEY",
        key_page="https://openrouter.ai/keys",
        pictures=True,
    ),
    Service(
        name="moonshot",
        title="Moonshot AI (Kimi)",
        base_url="https://api.moonshot.cn/v1",
        key_env="MOONSHOT_API_KEY",
        # The console of Moonshot moved to the domain of Kimi. Its address for requests stays.
        key_page="https://platform.kimi.com/console/api-keys",
    ),
    Service(
        name="zhipu",
        title="Zhipu AI (GLM)",
        base_url="https://open.bigmodel.cn/api/paas/v4",
        key_env="ZAI_API_KEY",
        key_page="https://bigmodel.cn/usercenter/proj-mgmt/apikeys",
    ),
    Service(
        name="siliconflow",
        title="SiliconFlow",
        base_url="https://api.siliconflow.cn/v1",
        key_env="SILICONFLOW_API_KEY",
        key_page="https://cloud.siliconflow.cn/account/ak",
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
    if service.pictures:
        config["pictures"] = True
    # What another address takes, such as one that answers as Anthropic does, is not known.
    if service.drops_unbound_thinking and base_url == service.base_url:
        config["drop_unbound_thinking"] = True
    return config
