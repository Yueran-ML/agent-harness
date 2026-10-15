"""The GitHub App that Columbina signs in with: ``columbina.github.app``."""

import pytest

from columbina.github import app as registered
from columbina.github.app import App, GitHubError, app

NAMED = {"COLUMBINA_GITHUB_CLIENT_ID": "Iv23liTest", "COLUMBINA_GITHUB_SLUG": "columbina-test"}


@pytest.fixture(autouse=True)
def unregistered(monkeypatch: pytest.MonkeyPatch) -> None:
    """A build of Columbina for which no app is registered, whatever this one has."""
    monkeypatch.setattr(registered, "CLIENT_ID", "")
    monkeypatch.setattr(registered, "SLUG", "")


def test_the_app_is_on_github_where_nothing_says_otherwise() -> None:
    assert app(NAMED) == App(
        client_id="Iv23liTest",
        web="https://github.com",
        api="https://api.github.com",
        slug="columbina-test",
    )


def test_the_app_that_is_registered_for_the_build_is_the_one_where_nothing_names_another(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(registered, "CLIENT_ID", "Iv23liBuilt")
    monkeypatch.setattr(registered, "SLUG", "columbina")

    assert (app({}).client_id, app({}).slug) == ("Iv23liBuilt", "columbina")
    assert app(NAMED).client_id == "Iv23liTest"


@pytest.mark.parametrize(
    "env",
    [{}, {"COLUMBINA_GITHUB_CLIENT_ID": "Iv23liTest"}, {"COLUMBINA_GITHUB_SLUG": "columbina"}],
)
def test_without_an_app_and_its_name_there_is_none_to_sign_in_with(env: dict[str, str]) -> None:
    with pytest.raises(GitHubError) as refused:
        app(env)

    assert refused.value.code == "NOT_REGISTERED"
    assert "COLUMBINA_GITHUB_CLIENT_ID" in str(refused.value)


def test_the_page_on_which_repositories_are_chosen_is_the_one_of_the_app() -> None:
    assert app(NAMED).install_url == "https://github.com/apps/columbina-test/installations/new"


def test_github_may_be_put_on_this_machine_and_its_addresses_lose_their_last_slash() -> None:
    env = NAMED | {
        "COLUMBINA_GITHUB_WEB": "http://127.0.0.1:8123/",
        "COLUMBINA_GITHUB_API": "http://localhost:8123/api/",
    }

    here = app(env)

    assert (here.web, here.api) == ("http://127.0.0.1:8123", "http://localhost:8123/api")
    assert here.install_url == "http://127.0.0.1:8123/apps/columbina-test/installations/new"


def test_another_github_is_reached_with_encryption() -> None:
    assert app(NAMED | {"COLUMBINA_GITHUB_WEB": "https://git.example.test"}).web == (
        "https://git.example.test"
    )


@pytest.mark.parametrize(
    ("variable", "said"),
    [
        ("COLUMBINA_GITHUB_WEB", "http://git.example.test"),
        ("COLUMBINA_GITHUB_API", "ftp://localhost/api"),
        ("COLUMBINA_GITHUB_API", "api.github.com"),
    ],
)
def test_an_address_that_cannot_be_one_of_github_is_refused(variable: str, said: str) -> None:
    with pytest.raises(GitHubError) as refused:
        app(NAMED | {variable: said})

    assert refused.value.code == "MISCONFIGURED"
    assert variable in str(refused.value)
