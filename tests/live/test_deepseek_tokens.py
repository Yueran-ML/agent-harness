"""The estimate of tokens against what the real DeepSeek service counts.

Skipped unless DEEPSEEK_API_KEY is set. The texts are written or generated in this
file, so they stay the same from run to run. They are eight of the 28 texts that the
table of rates was chosen on, which makes this a guard and no proof: it fails when the
table and the service drift apart. Each text is sent alone as one message with a limit
of one token for the reply, so the whole file costs about ten thousand tokens of input.
Run it with::

    uv run pytest tests/live -m live
"""

import hashlib
import json
import os
import random

import httpx
import pytest

from columbina.token_meter import estimate

pytestmark = [
    pytest.mark.live,
    pytest.mark.timeout(180),
    pytest.mark.skipif(
        not os.environ.get("DEEPSEEK_API_KEY"), reason="DEEPSEEK_API_KEY is not set"
    ),
]

PROSE = (
    "The harbour was quiet when the first boats came in, and nobody on the quay had expected "
    "them before noon. A woman with a ledger walked from one mooring to the next and wrote "
    "down what each crew had brought: herring, mostly, and a few crates of something that "
    "nobody would name. By the time the fog lifted, the price had been settled twice and "
    "unsettled once, and the harbour master had gone home for his breakfast. "
)
CODE = '''
def settle(prices: dict[str, float], offers: list[tuple[str, float]]) -> dict[str, float]:
    """Return the prices after the offers, each offer counting for a tenth of a price."""
    settled = dict(prices)
    for name, offer in offers:
        if name not in settled:
            raise KeyError(f"nobody sells {name!r} here")
        settled[name] = round(0.9 * settled[name] + 0.1 * offer, 2)
    return settled
'''
CHINESE = (
    "清晨的码头很安静，第一批渔船靠岸的时候，岸上的人都没有想到它们会回来得这么早。"  # noqa: RUF001 - a text in Chinese has the commas of Chinese
    "一位拿着账本的女人从一个泊位走到下一个泊位，记下每条船带回来的东西。"  # noqa: RUF001 - a text in Chinese has the commas of Chinese
    "雾散开之前，价钱已经谈定了两次，又推翻了一次，港务长早就回家吃早饭去了。"  # noqa: RUF001 - a text in Chinese has the commas of Chinese
)
WORDS = ("harbour", "ledger", "mooring", "herring", "crate", "quay", "fog", "price", "boat")


def lines_of_json() -> str:
    randomness = random.Random(11)
    rows = [
        {
            "id": index,
            "name": randomness.choice(WORDS),
            "weight": round(randomness.uniform(0, 500), 2),
            "sold": randomness.random() < 0.5,
            "tags": randomness.sample(WORDS, 3),
        }
        for index in range(40)
    ]
    return "\n".join(json.dumps(row) for row in rows)


def digests() -> str:
    return "\n".join(
        f"{hashlib.sha256(str(index).encode()).hexdigest()}  catch-{index}.tar"
        for index in range(40)
    )


def numbers() -> str:
    randomness = random.Random(13)
    return "\n".join(
        "  ".join(f"{randomness.uniform(-90, 90):9.4f}" for _ in range(6)) for _ in range(60)
    )


def paths() -> str:
    randomness = random.Random(17)
    return "\n".join(
        "src/" + "/".join(randomness.sample(WORDS, 3)) + f"/{randomness.choice(WORDS)}_{index}.py"
        for index in range(80)
    )


TEXTS = {
    "english prose": PROSE * 6,
    "python code": CODE * 8,
    "chinese prose": CHINESE * 8,
    "chinese with code": (CHINESE + "\n\n```python" + CODE + "```\n\n") * 4,
    "lines of json": lines_of_json(),
    "digests": digests(),
    "a table of numbers": numbers(),
    "paths": paths(),
}


@pytest.fixture(scope="module")
def counted() -> dict[str, int]:
    """What the service counts for each text, without what an empty message costs."""
    url = os.environ.get("DEEPSEEK_BASE_URL", "https://api.deepseek.com").rstrip("/")
    model = os.environ.get("COLUMBINA_MODEL", "deepseek-flash")
    headers = {"Authorization": f"Bearer {os.environ['DEEPSEEK_API_KEY']}"}

    def prompt_tokens(client: httpx.Client, text: str) -> int:
        body = {"model": model, "messages": [{"role": "user", "content": text}], "max_tokens": 1}
        reply = client.post(f"{url}/chat/completions", json=body, headers=headers)
        reply.raise_for_status()
        return int(reply.json()["usage"]["prompt_tokens"])

    with httpx.Client(timeout=120) as client:
        beside = prompt_tokens(client, "x") - 1
        return {name: prompt_tokens(client, text) - beside for name, text in TEXTS.items()}


@pytest.mark.parametrize("name", list(TEXTS))
def test_the_estimate_of_a_text_is_within_a_quarter_of_what_the_service_counts(
    counted: dict[str, int], name: str
) -> None:
    estimated = estimate(TEXTS[name])

    assert estimated == pytest.approx(counted[name], rel=0.25), (
        f"{name}: estimated {estimated}, counted {counted[name]}"
    )


def test_on_average_the_estimate_is_off_by_less_than_a_seventh(counted: dict[str, int]) -> None:
    off = [abs(estimate(TEXTS[name]) / counted[name] - 1) for name in TEXTS]

    assert sum(off) / len(off) < 1 / 7, dict(
        zip(TEXTS, (round(one, 3) for one in off), strict=True)
    )
