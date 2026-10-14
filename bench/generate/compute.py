"""The tasks of many steps of reckoning, made from a seed.

A problem is made the way the second bank of ReliQuant makes its problems
(the author's own study of quantized models): a plant, a store or a delivery
hub handles some units an hour for some hours; then three to six steps follow,
each of them once at most: the day is repeated for some days, a share is
rejected, the rest is split into equal shipments of which one is kept, a rush
order adds some, some are damaged, and each unit left may bring in a price.
Every division and every share comes out whole.

What changes here is that no number is written in the text. Each stands
there as a name in brackets, and a model looks it up with the tool
``lookup``. So a model that answers has asked for every number, and the
tool calls show how it went about the problem.
"""

import json
import random
from dataclasses import dataclass
from pathlib import Path

from bench.generate import files

__all__ = ["HIGHEST", "LOWEST", "Problem", "Step", "made", "run", "text", "written"]

DOMAINS = {
    "factory": {"actor": ("the plant", "the workshop"), "unit": "parts", "unit_s": "part"},
    "commerce": {"actor": ("the store", "the outlet"), "unit": "boxes", "unit_s": "box"},
    "logistics": {
        "actor": ("the delivery hub", "the warehouse"),
        "unit": "packages",
        "unit_s": "package",
    },
}
OPS = ("mul_days", "pct_loss", "div_group", "add_batch", "sub_loss")
STEPS = (3, 4, 5, 6)
LOWEST, HIGHEST = 10, 100_000

NAMES = {
    "r": "hourly_rate",
    "h": "hours_per_day",
    "d": "days",
    "pct": "reject_percent",
    "g": "shipments",
    "b": "rush_order",
    "s": "damaged",
    "p": "price",
}
"""The name in brackets that each number of a problem goes by."""

START_DAILY = "Each day, {actor} handles {r} {unit} per hour for {h} hours."
START_ONCE = "On one day, {actor} handles {r} {unit} per hour for {h} hours."
# A share and a split of each day are of the units of that day alone. Of the units that are
# left from the days before, the answer would come out another way, and a model was seen to
# read it so where the text said no more than "remaining at that point".
DAILY = {
    "add_batch": "Each day, an extra rush order adds {b} more {unit}.",
    "sub_loss": "Each day, {s} {unit} are damaged in handling and written off.",
    "pct_loss": (
        "Each day, quality checks reject {pct} percent of the day's {unit} remaining at that point."
    ),
    "div_group": (
        "Each day, the day's {unit} remaining at that point are split evenly into {g} equal "
        "shipments, and only one shipment is kept."
    ),
}
MUL_DAYS_MORE = (
    "This daily routine repeats for {d} days in a row, and all subsequent operations apply "
    "to the combined total."
)
MUL_DAYS_LAST = "This daily routine repeats for {d} days in a row."
ONCE = {
    "add_batch": "An extra rush order then adds {b} more {unit}.",
    "sub_loss": "Then {s} {unit} are damaged in handling and written off.",
    "pct_loss": "Quality checks then reject {pct} percent of the {unit} remaining at that point.",
    "div_group": (
        "The {unit} remaining at that point are then split evenly into {g} equal shipments, "
        "and only one shipment is kept."
    ),
    "mul_value": "Each remaining {unit_s} brings in {p} dollars when sold.",
}
QUESTION_TOTAL = "How many {unit} are left in total at the end?"
QUESTION = "How many {unit} are left at the end?"
QUESTION_MONEY = "How many dollars does this bring in total?"

Step = tuple[str, dict[str, int]]


def run(steps: list[Step]) -> list[int] | None:
    """Return the value after each step, or ``None`` where a share or a division is not whole."""
    value = steps[0][1]["r"] * steps[0][1]["h"]
    values = [value]
    for op, numbers in steps[1:]:
        if op == "mul_days":
            value *= numbers["d"]
        elif op == "pct_loss":
            if value * numbers["pct"] % 100:
                return None
            value -= value * numbers["pct"] // 100
        elif op == "div_group":
            if value % numbers["g"]:
                return None
            value //= numbers["g"]
        elif op == "add_batch":
            value += numbers["b"]
        elif op == "sub_loss":
            value -= numbers["s"]
        else:
            value *= numbers["p"]
        values.append(value)
    return values


def _drawn(rng: random.Random, count: int) -> list[Step] | None:
    """Draw a program of ``count`` steps after the start, and an optional price."""
    pool: list[str] = list(OPS)
    rng.shuffle(pool)
    ops = pool[: count - 1]
    if rng.random() < 0.5:
        ops.append("mul_value")
    rate, hours = rng.randint(6, 14), rng.randint(4, 9)
    value = rate * hours
    steps: list[Step] = [("start", {"r": rate, "h": hours})]
    for op in ops:
        if op == "mul_days":
            numbers = {"d": rng.randint(3, 7)}
            value *= numbers["d"]
        elif op == "pct_loss":
            whole = [pct for pct in (10, 20, 25, 50) if value * pct % 100 == 0]
            if not whole:
                return None
            numbers = {"pct": rng.choice(whole)}
            value -= value * numbers["pct"] // 100
        elif op == "div_group":
            divisors = [g for g in range(2, 13) if value % g == 0]
            if not divisors:
                return None
            numbers = {"g": rng.choice(divisors)}
            value //= numbers["g"]
        elif op == "add_batch":
            numbers = {"b": rng.randint(15, 60)}
            value += numbers["b"]
        elif op == "sub_loss":
            if value <= 25:
                return None
            numbers = {"s": rng.randint(5, min(40, value - 5))}
            value -= numbers["s"]
        else:
            numbers = {"p": rng.randint(3, 9)}
            value *= numbers["p"]
        steps.append((op, numbers))
    return steps


@dataclass(frozen=True)
class Problem:
    """A problem and what answers it.

    Attributes:
        id: Its name.
        text: The problem, with every number as a name in brackets.
        values: Each name and its number.
        answer: The answer.
    """

    id: str
    text: str
    values: dict[str, int]
    answer: int


def text(steps: list[Step], domain: str, actor: str) -> str:
    """Write a program as a problem, each number as its name in brackets."""
    words = DOMAINS[domain]
    named = {key: f"[{name}]" for key, name in NAMES.items()}
    context = {"actor": actor, "unit": words["unit"], "unit_s": words["unit_s"], **named}
    ops = [op for op, _ in steps]
    days = ops.index("mul_days") if "mul_days" in ops else None
    sentences = [(START_DAILY if days is not None else START_ONCE).format(**context)]
    for at, op in enumerate(ops[1:], start=1):
        if op == "mul_days":
            template = MUL_DAYS_MORE if at < len(ops) - 1 else MUL_DAYS_LAST
        elif days is not None and at < days:
            template = DAILY[op]
        else:
            template = ONCE[op]
        sentences.append(template.format(**context))
    if ops[-1] == "mul_value":
        sentences.append(QUESTION_MONEY.format(**context))
    else:
        sentences.append((QUESTION_TOTAL if days is not None else QUESTION).format(**context))
    return " ".join(sentences)


PROMPT = (
    "{text}\n\n"
    "The numbers in brackets are not given here. Look each of them up with the tool lookup, by "
    "the name in the brackets. Work the answer out, and submit it, a whole number, with the "
    "tool submit_answer.\n"
)

README = (
    "# This task\n\n"
    "Nothing in this directory is needed. The numbers of the task are looked up with the tool "
    "lookup, and the answer is submitted with the tool submit_answer.\n"
)


def written(problems: list[Problem], root: Path) -> None:
    """Write a task for each problem, in a directory of its own below ``root``."""
    for problem in problems:
        directory = root / problem.id
        task = {
            "id": problem.id,
            "category": "compute",
            "prompt": PROMPT.format(text=problem.text),
            "timeout_seconds": 300,
            "max_steps": 25,
        }
        files.task_yml(directory, task)
        files.text(directory / "workspace" / "README.md", README)
        files.text(
            directory / "private" / "values.json", json.dumps(problem.values, indent=2) + "\n"
        )
        answer = {"value": problem.answer, "lookups": list(problem.values)}
        files.text(directory / "reference" / "answer.json", json.dumps(answer, indent=2) + "\n")


def made(seed: int, count: int) -> list[Problem]:
    """Make problems from a seed: the same seed makes the same problems, byte for byte."""
    rng = random.Random(seed)
    problems: list[Problem] = []
    cells = [(domain, steps) for steps in STEPS for domain in DOMAINS]
    while len(problems) < count:
        domain, length = cells[len(problems) % len(cells)]
        steps = _drawn(rng, length)
        if steps is None:
            continue
        values = run(steps)
        if values is None or not all(LOWEST <= value <= HIGHEST for value in values):
            continue
        actor = rng.choice(DOMAINS[domain]["actor"])
        numbers = {NAMES[key]: number for _, step in steps for key, number in step.items()}
        problems.append(
            Problem(
                id=f"compute-{len(problems) + 1:02d}-{domain}",
                text=text(steps, domain, actor),
                values=numbers,
                answer=values[-1],
            )
        )
    return problems
