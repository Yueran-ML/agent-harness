"""Layers: parsing them, stacking them, and filling in environment variables."""

import pytest

from columbina.kernel import (
    Change,
    CompositionError,
    Entry,
    Insert,
    Layer,
    flatten,
    interpolate,
    parse_layer,
)

BASE = """
- insert:
    - id: session
      plugin: demo.session
    - id: llm
      plugin: demo.llm
      config:
        model: small
        temperature: 0
"""


def layers(**named: str) -> dict[str, Layer]:
    return {name: parse_layer(text, source=name) for name, text in named.items()}


def test_a_layer_may_be_a_bare_list_of_operations() -> None:
    layer = parse_layer(BASE, source="base")

    assert layer.extends is None
    assert layer.source == "base"
    assert [entry.id for entry in flatten([layer])] == ["session", "llm"]


def test_an_empty_file_is_an_empty_layer() -> None:
    assert flatten([parse_layer("")]) == []


def test_a_layer_can_be_built_without_yaml() -> None:
    layer = Layer(
        patch=(
            Insert(insert=(Entry(id="llm", plugin="demo.llm"),)),
            Change(id="llm", disabled=True),
        )
    )

    assert flatten([layer]) == [Entry(id="llm", plugin="demo.llm", disabled=True)]
    assert layer.source == "<layer>"


def test_a_layer_may_extend_another() -> None:
    known = layers(base=BASE)
    cli = parse_layer(
        """
        extends: base
        patch:
          - insert:
              - id: cli
                plugin: demo.cli
        """,
        source="cli",
    )

    entries = flatten([cli], known.__getitem__)

    assert [entry.id for entry in entries] == ["session", "llm", "cli"]
    assert entries[2].plugin == "demo.cli"
    assert entries[2].config is None
    assert entries[2].disabled is False


def test_a_change_replaces_the_whole_config() -> None:
    patch = parse_layer("- {id: llm, config: {model: large}}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].config == {"model": "large"}


def test_a_change_leaves_the_fields_it_does_not_mention() -> None:
    patch = parse_layer("- {id: llm, disabled: true}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].disabled is True
    assert entries[1].plugin == "demo.llm"
    assert entries[1].config == {"model": "small", "temperature": 0}


def test_a_change_can_swap_the_plugin_and_clear_the_config() -> None:
    patch = parse_layer("- {id: llm, plugin: demo.other_llm, config: null}")

    entries = flatten([parse_layer(BASE), patch])

    assert entries[1].plugin == "demo.other_llm"
    assert entries[1].config is None


def test_layers_are_applied_in_order() -> None:
    first = parse_layer("- {id: llm, disabled: true}", source="user")
    second = parse_layer("- {id: llm, disabled: false}", source="command line")

    entries = flatten([parse_layer(BASE), first, second])

    assert entries[1].disabled is False


def test_an_operation_can_change_an_entry_inserted_earlier_in_the_same_layer() -> None:
    layer = parse_layer(
        """
        - insert:
            - {id: cli, plugin: demo.cli}
        - {id: cli, config: {theme: dark}}
        """
    )

    entries = flatten([layer])

    assert entries[0].config == {"theme": "dark"}


def test_changing_an_unknown_entry_is_an_error() -> None:
    patch = parse_layer("- {id: lmm, disabled: true}", source="user patch")

    with pytest.raises(CompositionError, match="user patch: there is no entry 'lmm' to change"):
        flatten([parse_layer(BASE), patch])


def test_inserting_an_existing_id_is_an_error() -> None:
    patch = parse_layer("- insert: [{id: llm, plugin: demo.again}]", source="user patch")

    with pytest.raises(CompositionError, match="user patch: entry 'llm' already exists"):
        flatten([parse_layer(BASE), patch])


def test_only_the_first_layer_may_extend() -> None:
    known = layers(base=BASE)
    patch = parse_layer("extends: base", source="user patch")

    with pytest.raises(CompositionError, match="user patch: only the first layer may extend"):
        flatten([parse_layer(BASE), patch], known.__getitem__)


def test_a_layer_cannot_extend_itself_through_others() -> None:
    known = layers(one="extends: two", two="extends: one")

    with pytest.raises(CompositionError, match="two -> one -> two"):
        flatten([known["one"]], known.__getitem__)


def test_the_source_of_a_layer_is_not_mistaken_for_a_layer_name() -> None:
    base = parse_layer("- insert: [{id: x, plugin: demo.x}]", source="actual-base.yml")
    child = parse_layer("extends: base", source="base")

    entries = flatten([child], {"base": base}.__getitem__)

    assert [entry.id for entry in entries] == ["x"]


def test_extending_an_unknown_layer_is_an_error() -> None:
    known = layers(cli="extends: base")

    with pytest.raises(CompositionError, match="cli: there is no layer 'base' to extend"):
        flatten([known["cli"]], known.__getitem__)

    with pytest.raises(CompositionError, match="cli: there is no layer 'base' to extend"):
        flatten([known["cli"]])


@pytest.mark.parametrize(
    ("text", "complaint"),
    [
        ("- insert: [", "not valid YAML"),
        # The reader of YAML makes values of what it reads. One that cannot be made fails
        # with an error that is not one of its own, and so does a text that is nested too
        # deeply to be read.
        ("- id: llm\n  config:\n    since: 2026-13-01\n", "a value in it cannot be read: month"),
        ("- id: llm\n  config:\n    flag: !!bool maybe\n", "a value in it cannot be read: 'maybe'"),
        pytest.param(
            "- id: llm\n  config: " + "[" * 3000 + "]" * 3000,
            "it is nested too deeply",
            id="lists in lists, thousands deep",
        ),
        ("just a string", "list of operations"),
        ("- {id: llm, colour: blue}", "colour"),
        ("- {insert: [], id: llm}", "id"),
        ("- insert: [{id: llm}]", "plugin"),
        ("- {disabled: true}", "id"),
        ("- {id: llm, plugin: null}", "leave it out"),
        ("- {id: llm, disabled: null}", "leave it out"),
        ("extends: base\nextra: 1", "extra"),
    ],
)
def test_a_malformed_layer_is_rejected_with_its_source(text: str, complaint: str) -> None:
    with pytest.raises(CompositionError, match=complaint) as failure:
        parse_layer(text, source="broken.yml")

    assert str(failure.value).startswith("broken.yml: ")


def test_interpolate_fills_in_variables_everywhere() -> None:
    config = {
        "url": "http://${HOST}:${PORT}/v1",
        "models": ["${MODEL}", "fixed"],
        "nested": {"key_env": "${KEY_NAME}"},
        "retries": 3,
        "enabled": True,
        "nothing": None,
    }
    env = {"HOST": "localhost", "PORT": "11434", "MODEL": "qwen", "KEY_NAME": "MY_KEY"}

    assert interpolate(config, env) == {
        "url": "http://localhost:11434/v1",
        "models": ["qwen", "fixed"],
        "nested": {"key_env": "MY_KEY"},
        "retries": 3,
        "enabled": True,
        "nothing": None,
    }


def test_interpolate_uses_a_default_when_the_variable_is_missing() -> None:
    assert interpolate("${MODEL:-small}", {}) == "small"
    assert interpolate("${MODEL:-small}", {"MODEL": "large"}) == "large"
    assert interpolate("${MODEL:-}", {}) == ""


def test_interpolate_keeps_an_escaped_dollar_sign() -> None:
    assert interpolate("costs $$5, see $${HOME}", {"HOME": "/root"}) == "costs $5, see ${HOME}"


def test_interpolate_names_the_missing_variable() -> None:
    with pytest.raises(CompositionError, match="environment variable 'MODEL' is not set"):
        interpolate({"model": "${MODEL}"}, {})


@pytest.mark.parametrize(
    "text",
    [
        "${}",
        "${1X}",
        "${ NAME }",
        "${NAME",
        "${NAME:-{}}",
        "${NAME:-$$5}",
        "${NAME:-${OTHER}}",
    ],
)
def test_interpolate_rejects_a_reference_it_cannot_read(text: str) -> None:
    with pytest.raises(CompositionError, match="reference"):
        interpolate(text, {"NAME": "value", "OTHER": "other"})


def test_interpolate_leaves_a_lone_dollar_sign_alone() -> None:
    assert interpolate("costs $5 or 5$", {}) == "costs $5 or 5$"
