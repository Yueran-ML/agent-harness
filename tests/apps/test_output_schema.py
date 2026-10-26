"""``columbina run --output-schema``: an answer for a program, which has to fit a JSON Schema."""

import json
from pathlib import Path

import pytest

import columbina.apps.headless
from columbina.apps import FRONTEND
from columbina.apps.launcher import main
from columbina.llm import UserMessage
from columbina.llm.adapters.scripted import Reply
from support.loop import MODEL, Harness

SCHEMA = {
    "type": "object",
    "properties": {
        "files": {"type": "array", "items": {"type": "string"}},
        "count": {"type": "integer"},
    },
    "required": ["files", "count"],
    "additionalProperties": False,
}


def schema_file(tmp_path: Path, schema: object = SCHEMA, text: str | None = None) -> Path:
    path = tmp_path / "schema.json"
    path.write_text(json.dumps(schema) if text is None else text, encoding="utf-8")
    return path


async def frontend(harness: Harness, schema: Path, **config: object) -> columbina.apps.Frontend:
    await harness.ctx.plugin(
        columbina.apps.headless,
        {
            "model": MODEL.model_dump(),
            "cwd": harness.cwd,
            "task": "List the files.",
            "output_schema": str(schema),
            **config,
        },
    )
    return harness.ctx.use(FRONTEND)


def asked(harness: Harness) -> list[str]:
    """Return what the user said in each request, the last message of it."""
    said: list[str] = []
    for request in harness.model.requests:
        last = request.messages[-1]
        assert isinstance(last, UserMessage)
        said.append(last.content[0].text)
    return said


async def test_an_answer_that_fits_is_printed_as_one_line_of_json(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text='```json\n{"files": ["a.py"],\n "count": 1}\n```'))
    runner = await frontend(harness, schema_file(tmp_path))

    code = await runner.run()

    assert code == 0
    assert capsys.readouterr().out == '{"files": ["a.py"], "count": 1}\n'
    (task,) = asked(harness)
    assert task.startswith("List the files.\n\nWhen you are done, answer with JSON alone")
    assert task.endswith("```json\n" + json.dumps(SCHEMA, indent=2) + "\n```")


async def test_an_answer_that_does_not_fit_is_sent_back_with_what_is_wrong(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(
        Reply(text="Here they are: a.py"),
        Reply(text='{"files": ["a.py"], "count": "one"}'),
        Reply(text='{"files": ["a.py"], "count": 1}'),
    )
    runner = await frontend(harness, schema_file(tmp_path))

    code = await runner.run()

    assert code == 0
    assert capsys.readouterr().out == '{"files": ["a.py"], "count": 1}\n'
    told = asked(harness)
    assert told[1] == (
        "Your answer does not fit the schema: the answer is no JSON: Expecting value at "
        "character 0. Answer again with the JSON alone, which fits the schema."
    )
    assert told[2] == (
        'Your answer does not fit the schema: /count is "one", and has to be integer. Answer '
        "again with the JSON alone, which fits the schema."
    )


async def test_an_answer_that_never_fits_fails_the_run(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text="{}"), Reply(text="[]"))
    runner = await frontend(harness, schema_file(tmp_path), schema_attempts=1)

    code = await runner.run()

    captured = capsys.readouterr()
    assert code == 1
    assert captured.out == "[]\n"
    assert captured.err == (
        "columbina: the answer does not fit the schema: the answer is [], and has to be object\n"
    )


async def test_many_problems_are_said_ten_at_a_time(harness: Harness, tmp_path: Path) -> None:
    many = {"type": "array", "items": {"type": "string"}}
    harness.model.add(Reply(text=json.dumps(list(range(12)))), Reply(text='["x"]'))
    runner = await frontend(harness, schema_file(tmp_path, many))

    assert await runner.run() == 0
    assert asked(harness)[1].endswith(
        "/9 is 9, and has to be string; and 2 more. Answer again with the JSON alone, which fits "
        "the schema."
    )


async def test_the_lines_of_json_end_with_the_output(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text='{"files": [], "count": 0}'))
    runner = await frontend(harness, schema_file(tmp_path), json_lines=True)

    assert await runner.run() == 0

    final = json.loads(capsys.readouterr().out.splitlines()[-1])
    assert final["output"] == {"files": [], "count": 0}
    assert "output_problems" not in final


async def test_the_lines_of_json_say_what_did_not_fit(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text='{"files": []}'))
    runner = await frontend(harness, schema_file(tmp_path), json_lines=True, schema_attempts=0)

    assert await runner.run() == 1

    final = json.loads(capsys.readouterr().out.splitlines()[-1])
    assert final["output"] is None
    assert final["output_problems"] == ["the answer has no 'count'"]


async def test_a_turn_that_did_not_end_well_is_not_read(
    harness: Harness, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    harness.model.add(Reply(text='{"files"', finish="max-tokens"))
    runner = await frontend(harness, schema_file(tmp_path), json_lines=True)

    assert await runner.run() == 1

    final = json.loads(capsys.readouterr().out.splitlines()[-1])
    assert "output" not in final


@pytest.mark.parametrize(
    ("text", "said"),
    [
        (None, "cannot be read"),
        ("{not json", "is no JSON"),
        ('{"type": "object", "patternProperties": {}}', "cannot be checked: the schema has"),
    ],
)
async def test_a_schema_that_cannot_be_used_ends_the_run_before_it_begins(
    harness: Harness,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    text: str | None,
    said: str,
) -> None:
    path = tmp_path / "schema.json" if text is None else schema_file(tmp_path, text=text)
    runner = await frontend(harness, path)

    assert await runner.run() == 2

    assert said in capsys.readouterr().err
    assert harness.model.requests == []


def test_the_command_line_names_the_schema(
    home: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = main(
        ["run", "Go.", "--output-schema", str(tmp_path / "none.json"), "--cwd", str(tmp_path)]
    )

    assert code == 2
    assert "none.json cannot be read" in capsys.readouterr().err
