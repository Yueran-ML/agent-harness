"""What ``columbina setup`` does: the service of the model is chosen, and its key is stored."""

import json
from pathlib import Path

import pytest
import yaml

from columbina.apps import setup
from columbina.apps.setup import Chosen, Own, SetupError, chosen, forget, service_named
from columbina.llm import credentials
from columbina.llm.credentials import CredentialsError, keep
from columbina.llm.services import PRESETS, Service
from support.vault import Locked, Memory

KEY = "sk-test-0123456789"
OTHER = "sk-other-9876543210"
HEAD = "# Written by 'columbina setup'. It holds no key.\n"
ASK_SERVICE = "Service, by its number or its name [deepseek]: "
ASK_WINDOW = "How many tokens the model takes, if you know (Enter if not): "
ASK_ADDRESS = "Its address, up to but without /chat/completions (Enter goes back): "
NOT_SHOWN = "A key is not shown while it is typed."
AN_ADDRESS = (
    "An address begins with http:// or https:// and names a machine, such as "
    "https://api.example.com/v1."
)
NOT_ENCRYPTED = (
    "This address is not encrypted. The key and the conversation cross the network as they are."
)
A_NAME = (
    "A name of your own begins with a small letter and goes on with small letters, digits, "
    "'-' and '_'. It has 32 characters at most."
)
NOT_A_KEY = (
    "That cannot be a key: there is a character in it that no key has, such as a blank or a "
    "line break in the middle. Nothing of it was kept."
)
ADDED = (
    "A service that is not listed is added under a name of your own, if it speaks the OpenAI "
    "Chat Completions protocol."
)
IN_A_KEYRING = "the keyring of this system (vault Memory)"


def in_the_file(home: Path) -> str:
    """What a person is told of a key that no keyring took."""
    return (
        "No keyring of this system took the key: the keyring that is set up here keeps "
        f"nothing. It is in {home / 'credentials.json'}, as it is and not encrypted. Keep that "
        "file to yourself."
    )


class Person:
    """Answers the questions of the program in turn, and keeps what was asked and shown."""

    def __init__(self, *answers: str, keys: tuple[str, ...] = ("",)) -> None:
        self.answers = list(answers)
        self.keys = list(keys)
        self.asked: list[str] = []
        self.asked_unseen: list[str] = []
        self.shown: list[str] = []

    def ask(self, question: str) -> str:
        self.asked.append(question)
        if not self.answers:
            raise EOFError
        return self.answers.pop(0)

    def ask_unseen(self, question: str) -> str:
        self.asked_unseen.append(question)
        if not self.keys:
            raise EOFError
        return self.keys.pop(0)

    def show(self, told: str) -> None:
        self.shown.append(told)

    @property
    def told(self) -> str:
        return "\n".join(self.shown)

    @property
    def through(self) -> bool:
        """Whether every answer was asked for."""
        return not self.answers and not self.keys


@pytest.fixture
def home(tmp_path: Path) -> Path:
    return tmp_path / "home"


def set_up(home: Path, person: Person, env: dict[str, str] | None = None) -> None:
    setup.run(home, env or {}, person.ask, person.ask_unseen, person.show)


def by_hand(home: Path, text: str) -> None:
    """Write what was chosen by hand."""
    home.mkdir(parents=True, exist_ok=True)
    (home / "provider.yml").write_text(text, encoding="utf-8")


def written(home: Path) -> object:
    """What was chosen, as the file holds it."""
    return yaml.safe_load((home / "provider.yml").read_text(encoding="utf-8"))


def keys(home: Path) -> dict[str, str | None]:
    """The keys that are stored, as they lie in the file. ``None`` where a keyring has the key."""
    held = json.loads((home / "credentials.json").read_text(encoding="utf-8"))
    return {name: entry.get("key") for name, entry in held["keys"].items()}


def places(home: Path) -> dict[str, str]:
    """Where each key that is stored is sent."""
    return credentials.stored(home / "credentials.json")


# The first time


def test_the_first_time_the_answers_that_are_offered_are_the_ones_of_the_profiles(
    home: Path,
) -> None:
    person = Person("", "", keys=(KEY,))

    set_up(home, person)

    assert person.through
    assert person.asked == [ASK_SERVICE, "Model [deepseek-flash]: "]
    assert person.asked_unseen == ["The API key of deepseek, or Enter to store none: "]
    assert (home / "provider.yml").read_text(encoding="utf-8") == (
        f"{HEAD}use: deepseek\nservices:\n  deepseek:\n    model: deepseek-flash\n"
    )
    assert keys(home) == {"deepseek": KEY}
    assert sorted(found.name for found in home.iterdir()) == ["credentials.json", "provider.yml"]


def test_what_a_person_is_told_the_first_time(home: Path) -> None:
    person = Person("", "", keys=(KEY,))

    set_up(home, person)

    assert person.told == "\n".join(
        [
            "The model can come from one of these services.",
            "",
            "   1  deepseek     DeepSeek                   no key",
            "   2  openai       OpenAI                     no key",
            "   3  openrouter   OpenRouter                 no key",
            "   4  moonshot     Moonshot AI (Kimi)         no key",
            "   5  zhipu        Zhipu AI (GLM)             no key",
            "   6  siliconflow  SiliconFlow                no key",
            "   7  ollama       Ollama on this machine     asks for no key",
            "   8  lmstudio     LM Studio on this machine  asks for no key",
            "",
            "None of them is chosen yet. The profiles that come with Columbina talk to deepseek.",
            ADDED,
            "",
            NOT_SHOWN,
            "",
            "Stored. Columbina talks to deepseek now, with the model deepseek-flash.",
            f"What was chosen is in {home / 'provider.yml'}.",
            in_the_file(home),
        ]
    )
    assert KEY not in person.told


def test_a_key_that_a_keyring_took_lies_in_no_file_and_the_person_is_told_so(home: Path) -> None:
    vault = Memory().install()
    person = Person("", "", keys=(KEY,))

    set_up(home, person)

    assert person.shown[-3:] == [
        "Stored. Columbina talks to deepseek now, with the model deepseek-flash.",
        f"What was chosen is in {home / 'provider.yml'}.",
        f"The key is in {IN_A_KEYRING}. No file of Columbina holds it.",
    ]
    assert KEY not in person.told
    assert keys(home) == {"deepseek": None}
    assert places(home) == {"deepseek": "https://api.deepseek.com"}
    assert [provider for _, provider in vault.held] == ["deepseek"]
    for found in home.iterdir():
        assert KEY.encode() not in found.read_bytes()


def test_a_key_in_a_keyring_is_known_to_be_stored_the_next_time(home: Path) -> None:
    vault = Memory().install()
    set_up(home, Person("", "", keys=(KEY,)))
    held = dict(vault.held)
    person = Person("", "", keys=("",))

    set_up(home, person)

    assert (
        "   1  deepseek     DeepSeek                   a key is stored" in person.told.splitlines()
    )
    assert person.asked_unseen == ["A new key, or Enter to keep the one that is stored: "]
    # Nothing is said of where a key is that was not typed this time.
    assert person.shown[-1] == f"What was chosen is in {home / 'provider.yml'}."
    assert vault.held == held


# Which service


def test_a_service_is_chosen_by_its_name(home: Path) -> None:
    person = Person("openai", "gpt-test", "")

    set_up(home, person)

    assert person.through
    assert person.asked == [ASK_SERVICE, "Model: ", ASK_WINDOW]
    assert written(home) == {"use": "openai", "services": {"openai": {"model": "gpt-test"}}}
    assert person.shown[-3:] == [
        "Stored. Columbina talks to openai now, with the model gpt-test.",
        f"What was chosen is in {home / 'provider.yml'}.",
        "No key is stored for openai. Set the variable OPENAI_API_KEY before the program is "
        "started, or run this again.",
    ]
    assert not (home / "credentials.json").exists()


@pytest.mark.parametrize(
    ("typed", "name"),
    [("1", "deepseek"), (" 2 ", "openai"), ("8", "lmstudio"), ("9", "mine")],
)
def test_a_service_is_chosen_by_its_number_in_the_list(home: Path, typed: str, name: str) -> None:
    by_hand(home, "services:\n  mine:\n    base_url: http://localhost:8000/v1\n    model: tiny\n")
    person = Person(typed, "a-model", "")

    set_up(home, person)

    assert Chosen.model_validate(written(home)).use == name


def test_a_number_that_no_service_has_and_a_name_that_is_none_are_asked_for_again(
    home: Path,
) -> None:
    person = Person("0", "9", "Not A Name", "9lives", "x" * 33, "-", "x" * 32, "", "openrouter")
    person.answers += ["some/model", ""]

    set_up(home, person)

    assert person.through
    no_such = ["There is no service with the number 0.", "There is no service with the number 9."]
    assert [told for told in person.shown if told in (*no_such, A_NAME)] == [
        *no_such,
        A_NAME,
        A_NAME,
        A_NAME,
        A_NAME,
    ]
    # A name of 32 characters is one, and whoever goes back from its address is asked again.
    assert f"{'x' * 32} is not in the list, so it is added as a service of your own." in (
        person.shown
    )
    assert written(home) == {
        "use": "openrouter",
        "services": {"openrouter": {"model": "some/model"}},
    }


def test_the_service_that_is_in_use_is_the_one_that_is_offered(home: Path) -> None:
    by_hand(home, "use: moonshot\nservices:\n  moonshot:\n    model: kimi-test\n")
    person = Person("", "", "")

    set_up(home, person)

    assert person.asked == [
        "Service, by its number or its name [moonshot]: ",
        "Model [kimi-test]: ",
        ASK_WINDOW,
    ]
    assert "In use: moonshot, with the model kimi-test." in person.told
    assert written(home) == {"use": "moonshot", "services": {"moonshot": {"model": "kimi-test"}}}


def test_a_service_in_use_for_which_no_model_is_named_is_told_so(home: Path) -> None:
    by_hand(home, "use: openrouter\n")
    person = Person("", "some/model", "")

    set_up(home, person)

    assert "In use: openrouter, with no model." in person.told
    assert person.asked[:2] == ["Service, by its number or its name [openrouter]: ", "Model: "]


def test_an_address_keeps_what_stands_between_its_slashes(home: Path) -> None:
    person = Person("mine", "https://models.example.test/a//chat/completions", "tiny", "")

    set_up(home, person)

    assert Chosen.model_validate(written(home)).services["mine"].base_url == (
        "https://models.example.test/a/"
    )


def test_a_service_in_use_that_is_none_is_not_offered(home: Path) -> None:
    by_hand(home, "use: gone\n")
    person = Person("", "")

    set_up(home, person)

    assert person.asked[0] == ASK_SERVICE
    assert "None of them is chosen yet." in person.told
    assert written(home) == {
        "use": "deepseek",
        "services": {"deepseek": {"model": "deepseek-flash"}},
    }


# A service of one's own


def test_a_service_that_is_not_listed_is_added_by_its_address(home: Path) -> None:
    person = Person("my-server", "http://10.0.0.5:8000/v1", "", "tiny", "32768", keys=(KEY,))

    set_up(home, person)

    assert person.through
    assert person.asked == [ASK_SERVICE, ASK_ADDRESS, "Model: ", "Model: ", ASK_WINDOW]
    assert person.asked_unseen == ["The API key of my-server, or Enter to store none: "]
    assert written(home) == {
        "use": "my-server",
        "services": {
            "my-server": {
                "base_url": "http://10.0.0.5:8000/v1",
                "model": "tiny",
                "context_window": 32768,
            }
        },
    }
    assert keys(home) == {"my-server": KEY}
    assert "my-server is not in the list, so it is added as a service of your own." in (
        person.shown
    )
    assert "my-server has no model that is used by default, so one has to be named." in (
        person.shown
    )


def test_a_service_of_ones_own_is_listed_from_then_on_and_keeps_its_address(home: Path) -> None:
    set_up(home, Person("my-server", "https://models.example.test/v1", "tiny", ""))
    person = Person("", "", "")

    set_up(home, person)

    assert "   9  my-server    https://models.example.test/v1  no key" in person.told.splitlines()
    assert "In use: my-server, with the model tiny." in person.told
    assert person.asked == [
        "Service, by its number or its name [my-server]: ",
        "Model [tiny]: ",
        ASK_WINDOW,
    ]
    assert person.asked_unseen == ["The API key of my-server, or Enter to store none: "]
    assert written(home) == {
        "use": "my-server",
        "services": {"my-server": {"base_url": "https://models.example.test/v1", "model": "tiny"}},
    }
    # Nobody is told to set a variable for a service that may ask for no key.
    assert not any("No key is stored" in told for told in person.shown)


@pytest.mark.parametrize(
    "typed",
    [
        "models.example.test/v1",
        "ftp://models.example.test/v1",
        "https://",
        "https:///v1",
        "https://models. example.test/v1",
        "https://models.example.test/v1" + chr(0x1B) + "[2J",
        "https://someone:secret@models.example.test/v1",
        "https://models.example.test/v1?key=secret",
        "https://models.example.test/v1#part",
        "http://[::1/v1",
    ],
    ids=[
        "no scheme",
        "a scheme that is not of the web",
        "no machine",
        "a path and no machine",
        "a blank inside",
        "a character that a terminal acts on",
        "a name and a password",
        "a question",
        "a part of a page",
        "a bracket that is not closed",
    ],
)
def test_what_is_no_address_is_asked_for_again(home: Path, typed: str) -> None:
    person = Person("mine", typed, "https://ok.test/v1", "tiny", "")

    set_up(home, person)

    assert person.shown.count(AN_ADDRESS) == 1
    assert Chosen.model_validate(written(home)).services["mine"].base_url == "https://ok.test/v1"


@pytest.mark.parametrize(
    ("typed", "stored", "warned"),
    [
        ("https://models.example.test/v1", "https://models.example.test/v1", False),
        ("  https://models.example.test/v1/  ", "https://models.example.test/v1", False),
        (
            "https://models.example.test/v1/chat/completions",
            "https://models.example.test/v1",
            False,
        ),
        ("https://models.example.test/chat/completions/", "https://models.example.test", False),
        ("HTTPS://models.example.test", "HTTPS://models.example.test", False),
        ("http://models.example.test/v1", "http://models.example.test/v1", True),
        ("http://10.0.0.5:8000", "http://10.0.0.5:8000", True),
        ("http://localhost:8000/v1", "http://localhost:8000/v1", False),
        ("http://127.0.0.1:1234/v1", "http://127.0.0.1:1234/v1", False),
        ("http://[::1]:8000/v1", "http://[::1]:8000/v1", False),
        ("http://box.localhost/v1", "http://box.localhost/v1", False),
        ("http://notlocalhost/v1", "http://notlocalhost/v1", True),
    ],
)
def test_an_address_is_stored_without_what_the_adapter_adds_and_one_that_is_open_is_named(
    home: Path, typed: str, stored: str, warned: bool
) -> None:
    person = Person("mine", typed, "tiny", "")

    set_up(home, person)

    assert Chosen.model_validate(written(home)).services["mine"].base_url == stored
    assert (NOT_ENCRYPTED in person.shown) is warned
    assert AN_ADDRESS not in person.shown


def test_whoever_gives_no_address_is_asked_for_the_service_again(home: Path) -> None:
    person = Person("opnai", "", "openai", "gpt-test", "")

    set_up(home, person)

    assert person.asked == [ASK_SERVICE, ASK_ADDRESS, ASK_SERVICE, "Model: ", ASK_WINDOW]
    assert written(home) == {"use": "openai", "services": {"openai": {"model": "gpt-test"}}}


# The model, and how much it takes


def test_a_model_has_to_be_named_where_the_service_has_none_of_its_own(home: Path) -> None:
    person = Person("zhipu", "", "  ", "glm" + chr(0x1B) + "[2J", " glm-test ", "")

    set_up(home, person)

    none = "zhipu has no model that is used by default, so one has to be named."
    unseen = "The name of a model has no characters in it that cannot be seen."
    assert [told for told in person.shown if told in (none, unseen)] == [none, none, unseen]
    assert written(home) == {"use": "zhipu", "services": {"zhipu": {"model": "glm-test"}}}


def test_the_model_that_was_named_before_is_the_one_that_is_offered(home: Path) -> None:
    by_hand(home, "services:\n  deepseek:\n    model: deepseek-v4-pro\n")
    person = Person("deepseek", "", "")

    set_up(home, person)

    # What is known of the size is known of another model, so it is asked for.
    assert person.asked == [ASK_SERVICE, "Model [deepseek-v4-pro]: ", ASK_WINDOW]
    assert written(home) == {
        "use": "deepseek",
        "services": {"deepseek": {"model": "deepseek-v4-pro"}},
    }


def test_the_size_of_the_window_is_not_asked_for_where_it_is_known(home: Path) -> None:
    by_hand(
        home,
        "services:\n  mine:\n    base_url: http://localhost:8000/v1\n    model: tiny\n"
        "    context_window: 32768\n  deepseek:\n    context_window: 500000\n",
    )
    kept, of_the_list = Person("mine", ""), Person("deepseek", "")

    set_up(home, kept)
    set_up(home, of_the_list)

    assert ASK_WINDOW not in kept.asked + of_the_list.asked
    # And what was said of it before stays as it was said.
    assert Chosen.model_validate(written(home)).services == {
        "mine": Own(base_url="http://localhost:8000/v1", model="tiny", context_window=32768),
        "deepseek": Own(model="deepseek-flash", context_window=500000),
    }


def test_the_size_of_the_window_is_asked_for_again_for_another_model(home: Path) -> None:
    by_hand(
        home,
        "services:\n  mine:\n    base_url: http://localhost:8000/v1\n    model: tiny\n"
        "    context_window: 32768\n",
    )
    person = Person("mine", "large", "")

    set_up(home, person)

    assert person.asked[-1] == ASK_WINDOW
    assert Chosen.model_validate(written(home)).services["mine"] == Own(
        base_url="http://localhost:8000/v1", model="large"
    )


@pytest.mark.parametrize(
    ("typed", "tokens"),
    [("128000", 128000), (" 128,000 ", 128000), ("1_048_576", 1048576), ("1", 1), ("", None)],
)
def test_the_size_of_a_window_is_a_number_of_tokens_or_nothing(
    home: Path, typed: str, tokens: int | None
) -> None:
    person = Person("openai", "gpt-test", "many", "0", "-5", "128k", "1.5", typed)

    set_up(home, person)

    assert person.through
    assert person.shown.count("A number of tokens is a number, such as 128000.") == 5
    assert Chosen.model_validate(written(home)).services["openai"].context_window == tokens


# The key


def test_a_key_is_stored_without_what_was_typed_around_it(home: Path) -> None:
    person = Person("", "", keys=(f"  {KEY}\r\n",))

    set_up(home, person)

    assert keys(home) == {"deepseek": KEY}


def test_what_cannot_be_a_key_is_asked_for_again_and_is_not_repeated(home: Path) -> None:
    person = Person("", "", keys=("sk-test 0123", chr(0x201C) + KEY + chr(0x201D), KEY))

    set_up(home, person)

    assert person.through
    assert person.shown.count(NOT_A_KEY) == 2
    assert person.asked_unseen == ["The API key of deepseek, or Enter to store none: "] * 3
    assert "sk-test 0123" not in person.told
    assert KEY not in person.told
    assert keys(home) == {"deepseek": KEY}


def test_a_key_that_is_stored_stays_unless_a_new_one_is_typed(home: Path) -> None:
    keep(home / "credentials.json", "deepseek", KEY, "https://api.deepseek.com")
    stays, changes = Person("", ""), Person("", "", keys=(OTHER,))

    set_up(home, stays)
    kept = keys(home)
    set_up(home, changes)

    assert (
        "   1  deepseek     DeepSeek                   a key is stored" in stays.told.splitlines()
    )
    assert stays.shown[stays.shown.index(NOT_SHOWN) - 1] == "A key is stored for deepseek."
    assert stays.asked_unseen == ["A new key, or Enter to keep the one that is stored: "]
    assert kept == {"deepseek": KEY}
    assert keys(home) == {"deepseek": OTHER}
    # Nothing is said to be missing, and nothing new is said to be stored.
    assert stays.shown[-1] == f"What was chosen is in {home / 'provider.yml'}."


def test_a_key_in_the_variable_of_the_service_is_named_and_goes_first(home: Path) -> None:
    keep(home / "credentials.json", "deepseek", KEY, "https://api.deepseek.com")
    person = Person("", "")

    set_up(home, person, {"DEEPSEEK_API_KEY": f" {OTHER}\n", "OPENAI_API_KEY": " \n"})

    assert (
        "   1  deepseek     DeepSeek                   its key is in the variable DEEPSEEK_API_KEY"
    ) in person.told.splitlines()
    assert "   2  openai       OpenAI                     no key" in person.told.splitlines()
    assert person.shown[person.shown.index(NOT_SHOWN) - 1] == (
        "The variable DEEPSEEK_API_KEY holds a key, and it goes before one that is stored."
    )
    assert person.asked_unseen == ["A key to store as well, or Enter for none: "]
    assert person.shown[-1] == f"What was chosen is in {home / 'provider.yml'}."
    assert OTHER not in person.told
    assert keys(home) == {"deepseek": KEY}


def test_nobody_is_told_to_set_a_variable_that_holds_a_key(home: Path) -> None:
    person = Person("", "")

    set_up(home, person, {"DEEPSEEK_API_KEY": KEY})

    assert person.shown[-1] == f"What was chosen is in {home / 'provider.yml'}."
    assert not (home / "credentials.json").exists()


def test_a_service_on_this_machine_is_asked_for_a_key_only_in_case(home: Path) -> None:
    person = Person("ollama", "qwen3:14b", "")

    set_up(home, person)

    assert person.shown[person.shown.index(NOT_SHOWN) - 1] == (
        "Ollama on this machine asks for no key."
    )
    assert person.asked_unseen == ["A key, if yours asks for one, or Enter for none: "]
    assert person.shown[-1] == f"What was chosen is in {home / 'provider.yml'}."
    assert not (home / "credentials.json").exists()


def test_a_service_on_this_machine_may_have_a_key_all_the_same(home: Path) -> None:
    set_up(home, Person("lmstudio", "tiny", "", keys=(KEY,)))
    person = Person("lmstudio", "", "")

    set_up(home, person)

    assert keys(home) == {"lmstudio": KEY}
    assert (
        "   8  lmstudio     LM Studio on this machine  a key is stored" in person.told.splitlines()
    )
    assert person.asked_unseen == ["A new key, or Enter to keep the one that is stored: "]


def test_the_key_of_a_service_of_ones_own_may_be_in_the_variable_for_such_keys(
    home: Path,
) -> None:
    by_hand(home, "services:\n  mine:\n    base_url: http://localhost:8000/v1\n    model: tiny\n")
    person = Person("mine", "", "")

    set_up(home, person, {"COLUMBINA_API_KEY": KEY})

    assert (
        "   9  mine         http://localhost:8000/v1   its key is in the variable COLUMBINA_API_KEY"
    ) in person.told.splitlines()
    assert person.asked_unseen == ["A key to store as well, or Enter for none: "]


# What is stored, and when


def test_nothing_is_stored_before_the_last_question_is_answered(home: Path) -> None:
    for person in (Person(), Person("openai"), Person("openai", "gpt-test", ""), Person("", "")):
        person.keys.clear()

        with pytest.raises(EOFError):
            set_up(home, person)

    assert not home.exists()


def test_what_was_said_of_the_other_services_stays(home: Path) -> None:
    set_up(home, Person("openai", "gpt-test", "400000", keys=(KEY,)))
    set_up(home, Person("my-server", "http://localhost:8000/v1", "tiny", "", keys=(OTHER,)))

    set_up(home, Person("deepseek", ""))

    assert written(home) == {
        "use": "deepseek",
        "services": {
            "openai": {"model": "gpt-test", "context_window": 400000},
            "my-server": {"base_url": "http://localhost:8000/v1", "model": "tiny"},
            "deepseek": {"model": "deepseek-flash"},
        },
    }
    assert keys(home) == {"my-server": OTHER, "openai": KEY}


def test_the_address_that_a_person_gave_a_listed_service_by_hand_stays(home: Path) -> None:
    by_hand(home, "services:\n  moonshot:\n    base_url: https://api.moonshot.ai/v1\n")
    person = Person("moonshot", "kimi-test", "")

    set_up(home, person)

    assert written(home) == {
        "use": "moonshot",
        "services": {"moonshot": {"base_url": "https://api.moonshot.ai/v1", "model": "kimi-test"}},
    }


def test_the_file_of_what_was_chosen_holds_no_key_and_says_so(home: Path) -> None:
    set_up(home, Person("", "", keys=(KEY,)))

    text = (home / "provider.yml").read_text(encoding="utf-8")
    assert text.startswith(HEAD)
    assert KEY not in text
    assert "\r" not in (home / "provider.yml").read_bytes().decode()


def test_the_variables_that_go_before_what_is_stored_are_named(home: Path) -> None:
    quiet, told = Person("", ""), Person("", "")
    env = {
        "COLUMBINA_MODEL": "another-model",
        "COLUMBINA_CONTEXT_WINDOW": "1000",
        "COLUMBINA_PROVIDER": "openai",
        "COLUMBINA_HOME": str(home),
    }

    set_up(home, quiet)
    set_up(home, told, env)

    assert not any("goes before what is stored here" in line for line in quiet.shown)
    assert told.shown[-3:] == [
        "The variable COLUMBINA_PROVIDER is set, and it goes before what is stored here.",
        "The variable COLUMBINA_MODEL is set, and it goes before what is stored here.",
        "The variable COLUMBINA_CONTEXT_WINDOW is set, and it goes before what is stored here.",
    ]


# What is there cannot be used


@pytest.mark.parametrize(
    ("text", "problem"),
    [
        ("use: [", "is not YAML. Run 'columbina setup' again, or delete the file"),
        (
            "- deepseek\n",
            "is not what 'columbina setup' writes: the whole of it: Input should be a valid "
            "dictionary or instance of Chosen. Run 'columbina setup' again, or delete the file",
        ),
        (
            "use: deepseek\nkey: sk-test\n",
            "is not what 'columbina setup' writes: key: Extra inputs are not permitted. Run "
            "'columbina setup' again, or delete the file",
        ),
        (
            "services:\n  mine:\n    context_window: 0\n",
            "is not what 'columbina setup' writes: services.mine.context_window: Input should "
            "be greater than 0. Run 'columbina setup' again, or delete the file",
        ),
        (
            "services:\n  mine:\n    api_key: sk-test\n",
            "is not what 'columbina setup' writes: services.mine.api_key: Extra inputs are not "
            "permitted. Run 'columbina setup' again, or delete the file",
        ),
        (
            "use: 2026-13-01\n",
            "gives a value that cannot be read: month must be in 1..12. Run 'columbina setup' "
            "again, or delete the file",
        ),
        (
            "use: 2026-01-01\n",
            "is not what 'columbina setup' writes: use: Input should be a valid string. Run "
            "'columbina setup' again, or delete the file",
        ),
        (
            "use: " + "[" * 3000 + "]" * 3000 + "\n",
            "is nested too deeply. Run 'columbina setup' again, or delete the file",
        ),
        (
            "services:\n  mine: &said\n    model: tiny\n  other: *said\n",
            "uses a value again by its name, which is not read here. Run 'columbina setup' "
            "again, or delete the file",
        ),
    ],
    ids=[
        "no yaml",
        "a list",
        "something that is not written there",
        "a window of no size",
        "something that is not said of a service",
        "a date that there is none of",
        "a date for a name",
        "lists in lists, thousands deep",
        "a value that is used again by its name",
    ],
)
def test_a_file_of_another_shape_is_refused_with_what_is_wrong_with_it(
    home: Path, text: str, problem: str
) -> None:
    by_hand(home, text)

    for read in (lambda: chosen(home), lambda: set_up(home, Person()), lambda: go(home)):
        with pytest.raises(SetupError) as refused:
            read()

        assert str(refused.value) == f"{home / 'provider.yml'} {problem}"


def go(home: Path) -> None:
    forget(home, "deepseek", print)


def test_a_file_that_is_empty_or_not_there_says_that_nothing_is_chosen(home: Path) -> None:
    nothing = chosen(home)
    by_hand(home, "")
    empty = chosen(home)
    by_hand(home, "# Written by hand.\n")

    assert nothing == empty == chosen(home) == Chosen()
    assert (nothing.use, nothing.services) == (None, {})


def test_a_file_that_cannot_be_read_says_so(home: Path) -> None:
    (home / "provider.yml").mkdir(parents=True)

    with pytest.raises(SetupError) as no_file:
        chosen(home)

    (home / "provider.yml").rmdir()
    (home / "provider.yml").write_bytes(b"use: \xff\xfe\n")
    with pytest.raises(SetupError) as no_text:
        chosen(home)

    assert str(no_file.value).startswith(f"{home / 'provider.yml'} cannot be read: ")
    assert not str(no_file.value).endswith("None")
    assert str(no_text.value) == (
        f"{home / 'provider.yml'} cannot be read: it is not text in UTF-8"
    )


def test_keys_that_cannot_be_read_are_said_before_anything_is_asked(home: Path) -> None:
    home.mkdir()
    (home / "credentials.json").write_text(f"deepseek = {KEY}", encoding="utf-8")
    # Its user's alone, so that what is wrong with it is what it holds.
    (home / "credentials.json").chmod(0o600)
    person = Person("", "", keys=(OTHER,))

    with pytest.raises(SetupError) as refused:
        set_up(home, person)

    assert str(refused.value) == (
        f"the file of stored keys, {home / 'credentials.json'}, is not what 'columbina setup' "
        "writes, so it is not used. Delete it, and enter the keys again with 'columbina setup'"
    )
    assert person.asked == []
    assert person.shown == []
    assert not (home / "provider.yml").exists()


def test_a_key_that_cannot_be_stored_leaves_what_was_chosen_as_it_was(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def full(path: Path, provider: str, key: str, origin: str) -> None:
        raise CredentialsError(f"the file of stored keys, {path}, cannot be written: No space")

    monkeypatch.setattr(credentials, "keep", full)

    with pytest.raises(SetupError) as refused:
        set_up(home, Person("", "", keys=(KEY,)))

    assert str(refused.value) == (
        f"the file of stored keys, {home / 'credentials.json'}, cannot be written: No space"
    )
    assert not (home / "provider.yml").exists()


def test_what_was_chosen_is_written_whole_or_not_at_all(
    home: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    set_up(home, Person("", ""))
    before = (home / "provider.yml").read_text(encoding="utf-8")

    def refused_by_the_system(self: Path, target: Path) -> Path:
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(Path, "replace", refused_by_the_system)

    with pytest.raises(SetupError) as refused:
        set_up(home, Person("openai", "gpt-test", ""))

    assert str(refused.value) == f"{home / 'provider.yml'} cannot be written: Permission denied"
    assert (home / "provider.yml").read_text(encoding="utf-8") == before
    assert sorted(found.name for found in home.iterdir()) == ["provider.yml"]


# A service by its name


def test_a_listed_service_is_as_it_is_listed_where_nothing_was_said_of_it() -> None:
    said_of_another = Chosen(services={"openai": Own(model="gpt-test")})

    for service in PRESETS:
        assert service_named(service.name, Chosen()) is service
    assert service_named("deepseek", said_of_another) is PRESETS[0]


def test_what_was_said_of_a_listed_service_stands_in_the_place_of_what_is_known() -> None:
    deepseek = PRESETS[0]

    def with_said(**said: object) -> Service:
        return service_named("deepseek", Chosen(services={"deepseek": Own.model_validate(said)}))

    assert with_said() == deepseek
    assert with_said(base_url="https://mirror.example.test/v1") == Service(
        name="deepseek",
        title="DeepSeek",
        base_url="https://mirror.example.test/v1",
        key_env="DEEPSEEK_API_KEY",
        reasoning_style="deepseek",
        model="deepseek-flash",
        context_window=1_048_576,
        base_url_env="DEEPSEEK_BASE_URL",
    )
    # What is known of the size of the window is known of the one model.
    assert with_said(model="deepseek-flash").context_window == 1_048_576
    assert with_said(model="deepseek-v4-pro").model == "deepseek-v4-pro"
    assert with_said(model="deepseek-v4-pro").context_window is None
    assert with_said(model="deepseek-v4-pro", context_window=500).context_window == 500
    assert with_said(context_window=500).context_window == 500
    assert with_said(context_window=500).model == "deepseek-flash"


def test_a_service_of_ones_own_is_what_was_said_of_it() -> None:
    said = Own(base_url="http://localhost:8000/v1", model="tiny", context_window=32768)

    assert service_named("mine", Chosen(services={"mine": said})) == Service(
        name="mine",
        title="http://localhost:8000/v1",
        base_url="http://localhost:8000/v1",
        key_env="COLUMBINA_API_KEY",
        key_required=False,
        model="tiny",
        context_window=32768,
    )


def test_a_name_that_no_service_has_is_refused_with_the_names_that_there_are() -> None:
    there = Chosen(
        services={
            "zebra": Own(base_url="http://localhost:1/v1"),
            "mine": Own(base_url="http://localhost:2/v1"),
            "half": Own(model="a model and no address"),
            "openai": Own(model="gpt-test"),
            "moonshot": Own(base_url="https://api.moonshot.ai/v1"),
        }
    )

    for name in ("nowhere", "half", "Deepseek", ""):
        with pytest.raises(SetupError) as refused:
            service_named(name, there)

        assert str(refused.value) == (
            f"there is no service {name!r}. The services are: deepseek, openai, openrouter, "
            "moonshot, zhipu, siliconflow, ollama, lmstudio, mine, zebra. Another one is added "
            "with 'columbina setup'"
        )


# Forgetting


def test_what_is_stored_for_a_service_is_deleted_and_the_rest_stays(home: Path) -> None:
    set_up(home, Person("openai", "gpt-test", "", keys=(KEY,)))
    set_up(home, Person("my-server", "http://localhost:8000/v1", "tiny", "", keys=(OTHER,)))
    shown: list[str] = []

    forget(home, "openai", shown.append)

    assert shown == ["The key of openai is deleted.", "What was said of openai is deleted."]
    assert written(home) == {
        "use": "my-server",
        "services": {"my-server": {"base_url": "http://localhost:8000/v1", "model": "tiny"}},
    }
    assert keys(home) == {"my-server": OTHER}


def test_a_key_in_a_keyring_is_deleted_with_what_was_said_of_its_service(home: Path) -> None:
    vault = Memory().install()
    set_up(home, Person("openai", "gpt-test", "", keys=(KEY,)))
    set_up(home, Person("my-server", "http://localhost:8000/v1", "tiny", "", keys=(OTHER,)))
    shown: list[str] = []

    forget(home, "openai", shown.append)

    assert shown == ["The key of openai is deleted.", "What was said of openai is deleted."]
    assert [provider for _, provider in vault.held] == ["my-server"]
    assert places(home) == {"my-server": "http://localhost:8000"}


def test_a_key_that_its_keyring_does_not_give_up_leaves_everything_as_it_was(home: Path) -> None:
    Memory().install()
    set_up(home, Person("openai", "gpt-test", "", keys=(KEY,)))
    chosen_before = (home / "provider.yml").read_bytes()
    Locked().install()
    shown: list[str] = []

    with pytest.raises(SetupError) as refused:
        forget(home, "openai", shown.append)

    assert str(refused.value) == (
        "the key of openai is kept in the keyring of this system, and it could not be taken "
        "out of it: Failed to unlock the collection"
    )
    assert shown == []
    assert (home / "provider.yml").read_bytes() == chosen_before
    assert places(home) == {"openai": "https://api.openai.com"}


def test_a_service_that_is_forgotten_is_no_longer_in_use(home: Path) -> None:
    set_up(home, Person("openai", "gpt-test", ""))
    shown: list[str] = []

    forget(home, "openai", shown.append)

    assert shown == [
        "What was said of openai is deleted.",
        "openai is no longer in use. The profile says again where the model comes from.",
    ]
    assert written(home) == {"services": {}}
    assert chosen(home) == Chosen()


def test_a_key_alone_and_a_use_alone_are_forgotten_too(home: Path) -> None:
    keep(home / "credentials.json", "moonshot", KEY, "https://api.moonshot.cn")
    by_hand(home, "use: gone\nservices:\n  openai:\n    model: gpt-test\n")
    before = (home / "provider.yml").read_text(encoding="utf-8")
    key: list[str] = []
    use: list[str] = []

    forget(home, "moonshot", key.append)
    untouched = (home / "provider.yml").read_text(encoding="utf-8")
    forget(home, "gone", use.append)

    assert key == ["The key of moonshot is deleted."]
    assert untouched == before
    assert keys(home) == {}
    assert use == ["gone is no longer in use. The profile says again where the model comes from."]
    assert written(home) == {"services": {"openai": {"model": "gpt-test"}}}


def test_where_nothing_is_stored_for_a_service_nothing_is_forgotten(home: Path) -> None:
    set_up(home, Person("openai", "gpt-test", "", keys=(KEY,)))
    before = (home / "provider.yml").read_bytes(), (home / "credentials.json").read_bytes()
    shown: list[str] = []

    for name in ("deepseek", "nowhere"):
        with pytest.raises(SetupError) as refused:
            forget(home, name, shown.append)

        assert str(refused.value) == f"nothing is stored for {name!r}"
    assert shown == []
    assert (
        (home / "provider.yml").read_bytes(),
        (home / "credentials.json").read_bytes(),
    ) == before


def test_keys_that_cannot_be_read_are_not_forgotten(home: Path) -> None:
    home.mkdir()
    (home / "credentials.json").write_text(json.dumps({"deepseek": KEY}), encoding="utf-8")

    with pytest.raises(SetupError) as refused:
        forget(home, "deepseek", print)

    assert "is not what 'columbina setup' writes" in str(refused.value)
    assert KEY not in str(refused.value)


# Where a key goes, and what is said to be stored


def test_a_key_is_stored_for_the_place_that_its_service_is_at(home: Path) -> None:
    set_up(home, Person("", "", keys=(KEY,)))
    set_up(home, Person("my-server", "http://10.0.0.5:8000/v1", "tiny", "", keys=(OTHER,)))
    set_up(home, Person("ollama", "qwen3:14b", "", keys=(KEY,)))

    assert places(home) == {
        "deepseek": "https://api.deepseek.com",
        "my-server": "http://10.0.0.5:8000",
        "ollama": "http://localhost:11434",
    }


def test_a_key_that_was_stored_for_another_address_is_told_as_that_and_asked_for_again(
    home: Path,
) -> None:
    keep(home / "credentials.json", "moonshot", KEY, "https://api.moonshot.cn")
    by_hand(home, "services:\n  moonshot:\n    base_url: https://api.moonshot.ai/v1\n")
    person = Person("moonshot", "kimi-test", "", keys=(OTHER,))

    set_up(home, person)

    assert (
        "   4  moonshot     Moonshot AI (Kimi)         a key is stored for another address"
    ) in person.told.splitlines()
    assert person.shown[person.shown.index(NOT_SHOWN) - 1] == (
        "The key that is stored for moonshot is for https://api.moonshot.cn, and it is not "
        "sent to this address."
    )
    assert person.asked_unseen == ["A key for this address, or Enter to store none: "]
    assert keys(home) == {"moonshot": OTHER}
    assert places(home) == {"moonshot": "https://api.moonshot.ai"}


def test_a_key_for_another_address_does_not_count_as_a_key_of_the_service(home: Path) -> None:
    keep(home / "credentials.json", "moonshot", KEY, "https://api.moonshot.cn")
    by_hand(home, "services:\n  moonshot:\n    base_url: https://api.moonshot.ai/v1\n")
    person = Person("moonshot", "kimi-test", "")

    set_up(home, person)

    assert person.shown[-1] == (
        "No key is stored for moonshot. Set the variable MOONSHOT_API_KEY before the program "
        "is started, or run this again."
    )
    assert places(home) == {"moonshot": "https://api.moonshot.cn"}


@pytest.mark.parametrize(
    ("elsewhere", "told"),
    [
        ("http://other-host.example.test/v1", True),
        ("https://api.deepseek.com:8443", True),
        ("https://api.deepseek.com/beta", False),
        ("", False),
    ],
    ids=["another machine", "another port", "the same place", "set to nothing"],
)
def test_a_variable_that_sends_the_requests_elsewhere_is_named(
    home: Path, elsewhere: str, told: bool
) -> None:
    person = Person("", "", keys=(KEY,))

    set_up(home, person, {"DEEPSEEK_BASE_URL": elsewhere})

    note = (
        "The variable DEEPSEEK_BASE_URL is set, and it sends the requests for deepseek to "
        "another address. A stored key is not sent there."
    )
    assert (person.shown[-1] == note) is told
    assert (note in person.shown) is told
    # The key is stored for the service as it is known here, whatever the variable says.
    assert places(home) == {"deepseek": "https://api.deepseek.com"}


def test_a_service_without_such_a_variable_has_none_that_is_named(home: Path) -> None:
    person = Person("openai", "gpt-test", "", keys=(KEY,))

    set_up(home, person, {"DEEPSEEK_BASE_URL": "http://other-host.example.test/v1", "": "x"})

    assert not any("to another address" in told for told in person.shown)


def test_asking_stores_nothing_and_storing_asks_nothing(home: Path) -> None:
    person = Person("openai", "gpt-test", "400000", keys=(KEY,))

    answers = setup.asked(home, {}, person.ask, person.ask_unseen, person.show)

    assert person.through
    assert not home.exists()
    assert (answers.service.name, answers.model, answers.key) == ("openai", "gpt-test", KEY)
    assert answers.chosen == Chosen(
        use="openai", services={"openai": Own(model="gpt-test", context_window=400000)}
    )
    assert not answers.without_a_key
    # Whatever shows the answers shows no key.
    assert KEY not in repr(answers)
    assert KEY not in str(answers)
    shown_while_asking = list(person.shown)

    setup.store(home, {}, answers, person.show)

    assert keys(home) == {"openai": KEY}
    assert written(home) == {
        "use": "openai",
        "services": {"openai": {"model": "gpt-test", "context_window": 400000}},
    }
    assert person.shown[len(shown_while_asking) :] == [
        "",
        "Stored. Columbina talks to openai now, with the model gpt-test.",
        f"What was chosen is in {home / 'provider.yml'}.",
        in_the_file(home),
    ]


@pytest.mark.parametrize("in_a_keyring", [False, True], ids=["in the file", "in a keyring"])
def test_whoever_is_told_that_the_choice_could_not_be_written_is_told_of_the_key_that_was(
    home: Path, monkeypatch: pytest.MonkeyPatch, in_a_keyring: bool
) -> None:
    if in_a_keyring:
        Memory().install()
    where = IN_A_KEYRING if in_a_keyring else home / "credentials.json"
    replace = Path.replace

    def only_the_keys(self: Path, target: Path) -> Path:
        if Path(target).name == "provider.yml":
            raise PermissionError(13, "Permission denied")
        return replace(self, target)

    monkeypatch.setattr(Path, "replace", only_the_keys)

    with pytest.raises(SetupError) as refused:
        set_up(home, Person("", "", keys=(KEY,)))

    assert str(refused.value) == (
        f"{home / 'provider.yml'} cannot be written: Permission denied. The key was stored all "
        f"the same, in {where}"
    )
    assert keys(home) == {"deepseek": None if in_a_keyring else KEY}
    assert not (home / "provider.yml").exists()
