"""The package: its version, and the bundles that come with it."""

import re
from importlib import resources
from importlib.metadata import version

import pytest

import columbina
from columbina.kernel import flatten, import_plugin
from support.launch import bundle

BUNDLES = ["base", "cli", "headless", "minimal"]


def test_version_matches_the_installed_distribution() -> None:
    assert columbina.__version__ == version("columbina")


def test_version_looks_like_a_release_number() -> None:
    assert re.match(r"^\d+\.\d+\.\d+", columbina.__version__)


def test_the_bundles_come_with_the_package() -> None:
    files = resources.files("columbina.bundles").iterdir()

    assert sorted(file.name for file in files if file.name.endswith(".yml")) == [
        f"{name}.yml" for name in BUNDLES
    ]


@pytest.mark.parametrize("name", BUNDLES)
def test_every_entry_of_a_bundle_names_a_plugin_that_is_there(name: str) -> None:
    entries = flatten([bundle(name)], lookup=bundle)

    assert entries
    for entry in entries:
        # A name that leads nowhere raises here, and says which one it is.
        import_plugin(entry.plugin)


def test_every_bundle_builds_on_the_one_before_it() -> None:
    ids = {name: [entry.id for entry in flatten([bundle(name)], lookup=bundle)] for name in BUNDLES}

    assert ids["base"][: len(ids["minimal"])] == ids["minimal"]
    assert ids["headless"] == [*ids["base"], "frontend"]
    assert ids["cli"][: len(ids["base"])] == ids["base"]
    assert ids["cli"][-1] == "frontend"
