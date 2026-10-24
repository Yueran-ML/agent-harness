"""The services that a model can come from, by name."""

import re
from urllib.parse import urlsplit

import pytest

from columbina.kernel import Entry, Insert, interpolate
from columbina.llm.adapters.anthropic_messages import Config as AnthropicConfig
from columbina.llm.adapters.openai_chat import Config
from columbina.llm.services import (
    KEY_OF_ONES_OWN,
    PRESETS,
    Service,
    adapter_config,
    of_ones_own,
    preset,
)
from support.launch import bundle

KEYS = "${COLUMBINA_HOME}/credentials.json"
ENV = {"COLUMBINA_HOME": "/home/of/columbina"}


def known(name: str) -> Service:
    service = preset(name)
    assert service is not None
    return service


def entry_of(profile: str, entry_id: str) -> Entry:
    """An entry of a profile that comes with the package."""
    return next(
        entry
        for operation in bundle(profile).patch
        if isinstance(operation, Insert)
        for entry in operation.insert
        if entry.id == entry_id
    )


def test_the_services_are_listed_with_the_one_of_the_profiles_first() -> None:
    assert [service.name for service in PRESETS] == [
        "deepseek",
        "openai",
        "anthropic",
        "openrouter",
        "moonshot",
        "zhipu",
        "siliconflow",
        "ollama",
        "lmstudio",
    ]


def test_a_service_is_found_by_its_name_as_it_is_written() -> None:
    assert preset("openai") is PRESETS[1]
    assert preset("lmstudio") is PRESETS[-1]
    assert preset("OpenAI") is None
    assert preset("nowhere") is None


def test_where_each_service_is_and_which_variable_holds_its_key() -> None:
    # As the documents of the services say it. A change here is a change to check there.
    assert {service.name: (service.base_url, service.key_env) for service in PRESETS} == {
        "deepseek": ("https://api.deepseek.com", "DEEPSEEK_API_KEY"),
        "openai": ("https://api.openai.com/v1", "OPENAI_API_KEY"),
        "anthropic": ("https://api.anthropic.com", "ANTHROPIC_API_KEY"),
        "openrouter": ("https://openrouter.ai/api/v1", "OPENROUTER_API_KEY"),
        "moonshot": ("https://api.moonshot.cn/v1", "MOONSHOT_API_KEY"),
        "zhipu": ("https://open.bigmodel.cn/api/paas/v4", "ZAI_API_KEY"),
        "siliconflow": ("https://api.siliconflow.cn/v1", "SILICONFLOW_API_KEY"),
        "ollama": ("http://localhost:11434/v1", None),
        "lmstudio": ("http://localhost:1234/v1", None),
    }


def test_where_each_service_gives_out_its_keys() -> None:
    # As the documents of each service link to the page, on 2026-10-15, and of Anthropic on
    # 2026-10-21.
    assert {service.name: service.key_page for service in PRESETS} == {
        "deepseek": "https://platform.deepseek.com/api_keys",
        "openai": "https://platform.openai.com/api-keys",
        "anthropic": "https://platform.claude.com/settings/keys",
        "openrouter": "https://openrouter.ai/keys",
        "moonshot": "https://platform.kimi.com/console/api-keys",
        "zhipu": "https://bigmodel.cn/usercenter/proj-mgmt/apikeys",
        "siliconflow": "https://cloud.siliconflow.cn/account/ak",
        "ollama": None,
        "lmstudio": None,
    }


def test_what_is_called_what() -> None:
    assert [service.title for service in PRESETS] == [
        "DeepSeek",
        "OpenAI",
        "Anthropic (Claude)",
        "OpenRouter",
        "Moonshot AI (Kimi)",
        "Zhipu AI (GLM)",
        "SiliconFlow",
        "Ollama on this machine",
        "LM Studio on this machine",
    ]


def test_the_services_that_have_a_model_that_is_used_where_none_is_named() -> None:
    # Of Anthropic the model that its documents name first, with the window that they give it.
    assert {
        service.name: (service.model, service.context_window)
        for service in PRESETS
        if service.model is not None or service.context_window is not None
    } == {"deepseek": ("deepseek-flash", 1_048_576), "anthropic": ("claude-opus-5-5", 1_000_000)}


def test_the_habits_of_the_services() -> None:
    assert {
        service.name: (service.reasoning_style, service.max_tokens_field, service.base_url_env)
        for service in PRESETS
        if (service.reasoning_style, service.max_tokens_field, service.base_url_env)
        != ("none", "max_tokens", None)
    } == {
        "deepseek": ("deepseek", "max_tokens", "DEEPSEEK_BASE_URL"),
        "openai": ("openai", "max_completion_tokens", None),
        "anthropic": ("none", "max_tokens", "ANTHROPIC_BASE_URL"),
    }


def test_anthropic_alone_is_spoken_to_in_its_own_protocol() -> None:
    assert {service.name for service in PRESETS if service.protocol == "anthropic"} == {"anthropic"}
    assert of_ones_own("mine", "https://api.anthropic.com").protocol == "openai-chat"


def test_no_two_services_share_a_name_or_a_variable() -> None:
    names = [service.name for service in PRESETS]
    variables = [service.key_env for service in PRESETS if service.key_env is not None]

    assert len(set(names)) == len(names)
    assert len(set(variables)) == len(variables)
    assert KEY_OF_ONES_OWN == "COLUMBINA_API_KEY"
    assert KEY_OF_ONES_OWN not in variables


@pytest.mark.parametrize("service", PRESETS, ids=[service.name for service in PRESETS])
def test_what_is_written_of_a_service_makes_sense(service: Service) -> None:
    address = urlsplit(service.base_url)

    assert re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", service.name)
    assert service.title.strip() == service.title != ""
    assert not service.base_url.endswith(("/", "/chat/completions"))
    if service.key_required:
        # What is sent to a service across a network is encrypted, and it has a key.
        assert address.scheme == "https"
        assert service.key_env is not None
        assert re.fullmatch(r"[A-Z][A-Z0-9_]*_API_KEY", service.key_env)
    else:
        # A service that asks for no key is one that does not leave the machine.
        assert (address.scheme, address.hostname) == ("http", "localhost")
        assert service.key_env is None
    # The size of a window is the size of the window of one model.
    assert service.context_window is None or service.model is not None
    # A key is made on a page of the service, which is reached as anything else is.
    assert (service.key_page is not None) == service.key_required
    assert service.key_page is None or urlsplit(service.key_page).scheme == "https"
    # And the adapter of its protocol takes the config that is made of it.
    taken = AnthropicConfig if service.protocol == "anthropic" else Config
    taken.model_validate(interpolate(adapter_config(service, KEYS, ENV), ENV))


def test_the_config_of_an_adapter_says_what_is_known_of_its_service() -> None:
    assert adapter_config(known("openai"), KEYS, {}) == {
        "provider": "openai",
        "base_url": "https://api.openai.com/v1",
        "api_key_env": "OPENAI_API_KEY",
        "api_key_file": KEYS,
        "reasoning_style": "openai",
        "max_tokens_field": "max_completion_tokens",
    }
    assert adapter_config(known("moonshot"), KEYS, {}) == {
        "provider": "moonshot",
        "base_url": "https://api.moonshot.cn/v1",
        "api_key_env": "MOONSHOT_API_KEY",
        "api_key_file": KEYS,
    }
    # Anthropic is told nothing of how a service of OpenAI is spoken to. It is asked to leave
    # out thinking whose conversation changed, but not at another address, which may only
    # answer as Anthropic does.
    assert adapter_config(known("anthropic"), KEYS, {}) == {
        "provider": "anthropic",
        "base_url": "https://api.anthropic.com",
        "api_key_env": "ANTHROPIC_API_KEY",
        "api_key_file": KEYS,
        "drop_unbound_thinking": True,
    }
    elsewhere = {"ANTHROPIC_BASE_URL": "https://claude.example.test"}
    assert "drop_unbound_thinking" not in adapter_config(known("anthropic"), KEYS, elsewhere)
    assert adapter_config(known("ollama"), "elsewhere.json", {}) == {
        "provider": "ollama",
        "base_url": "http://localhost:11434/v1",
        "api_key_file": "elsewhere.json",
        "api_key_required": False,
    }


def test_the_variable_of_a_service_may_name_another_address() -> None:
    deepseek = known("deepseek")
    elsewhere = {"DEEPSEEK_BASE_URL": "https://mirror.example.test/v1"}

    assert adapter_config(deepseek, KEYS, {})["base_url"] == "https://api.deepseek.com"
    assert adapter_config(deepseek, KEYS, {"DEEPSEEK_BASE_URL": ""})["base_url"] == (
        "https://api.deepseek.com"
    )
    assert adapter_config(deepseek, KEYS, elsewhere)["base_url"] == (
        "https://mirror.example.test/v1"
    )
    # It is the variable of one service, and of no other.
    assert adapter_config(known("openai"), KEYS, elsewhere)["base_url"] == (
        "https://api.openai.com/v1"
    )


def test_a_dollar_sign_of_an_address_stands_for_itself() -> None:
    address = "http://localhost:8000/${COLUMBINA_HOME}/$$/v1"
    elsewhere = ENV | {"DEEPSEEK_BASE_URL": address}

    own = interpolate(adapter_config(of_ones_own("mine", address), KEYS, ENV), ENV)
    named = interpolate(adapter_config(known("deepseek"), KEYS, elsewhere), elsewhere)

    assert own["base_url"] == address
    assert named["base_url"] == address
    # The place of the keys is said the way a config says it, and is filled in.
    assert own["api_key_file"] == "/home/of/columbina/credentials.json"


def test_a_service_of_ones_own_is_sent_a_key_only_if_one_is_found() -> None:
    own = of_ones_own("mine", "http://10.0.0.5:8000/v1")

    assert own == Service(
        name="mine",
        title="http://10.0.0.5:8000/v1",
        base_url="http://10.0.0.5:8000/v1",
        key_env="COLUMBINA_API_KEY",
        key_required=False,
    )
    assert adapter_config(own, KEYS, {}) == {
        "provider": "mine",
        "base_url": "http://10.0.0.5:8000/v1",
        "api_key_env": "COLUMBINA_API_KEY",
        "api_key_file": KEYS,
        "api_key_required": False,
    }


def test_the_profiles_talk_to_the_first_service_the_way_its_entry_says() -> None:
    in_the_profile = entry_of("minimal", "model")
    elsewhere = ENV | {"DEEPSEEK_BASE_URL": "https://mirror.example.test/v1"}

    assert in_the_profile.plugin == "columbina.llm.adapters.openai_chat"
    for env in (ENV, elsewhere):
        # Naming the service changes nothing: it is the one that a profile names anyway.
        assert interpolate(adapter_config(PRESETS[0], KEYS, env), env) == interpolate(
            in_the_profile.config, env
        )


@pytest.mark.parametrize("profile", ["cli", "headless"])
def test_the_frontends_of_the_profiles_name_the_model_of_the_first_service(profile: str) -> None:
    first = PRESETS[0]

    assert interpolate(entry_of(profile, "frontend").config["model"], {}) == {
        "provider": first.name,
        "model": first.model,
        "context_window": str(first.context_window),
    }
