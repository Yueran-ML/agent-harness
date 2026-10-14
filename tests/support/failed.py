"""Tells which tests failed, and what each said first, where a run of the CI shows it.

The log of a run of the CI can be read only when one is signed in. Its summary
and its annotations can be seen by anyone who can see the repository, and the
annotations can also be read through the API without a key. When a run fails,
the CI calls this twice with the report that pytest wrote::

    python tests/support/failed.py junit.xml >> "$GITHUB_STEP_SUMMARY"
    python tests/support/failed.py --annotations junit.xml

Each test is named as pytest takes it on its command line, and what it said is
cut to its first line. A module that could not be collected is named with the
line that says why.
"""

import sys
from pathlib import Path
from xml.etree import ElementTree

__all__ = ["annotations", "main", "summary"]

_HEADING = "### What failed\n\n"
_SHOWN = 50
_ANNOTATED = 10  # what GitHub keeps of the errors that one step reports
_LONGEST = 200


def summary(report: Path) -> str:
    """The Markdown that tells which tests in ``report`` failed.

    The names and what they said are written as code, so that nothing in them
    is read as Markdown or HTML.
    """
    try:
        failed = _failed(report)
    except FileNotFoundError:
        return (
            _HEADING
            + "There is no report of the tests. An earlier step failed, or the tests did not end.\n"
        )
    except ElementTree.ParseError:
        return _HEADING + "The report of the tests cannot be read.\n"
    if not failed:
        return _HEADING + "No test failed. Another step did.\n"
    lines = [
        f"- {_code(test)}: {_code(said)}\n" if said else f"- {_code(test)}\n"
        for test, said in failed[:_SHOWN]
    ]
    if len(failed) > _SHOWN:
        lines.append(f"\nAnd {len(failed) - _SHOWN} more.\n")
    return _HEADING + "".join(lines)


def annotations(report: Path) -> str:
    """A workflow command for each of the first tests in ``report`` that failed.

    The runner makes each an annotation of the run. Where the report is missing
    or cannot be read, the summary says so, and there is nothing to annotate.
    """
    try:
        failed = _failed(report)
    except (FileNotFoundError, ElementTree.ParseError):
        return ""
    return "".join(
        f"::error title={_escaped(test, ':,')}::{_escaped(said or 'failed', '')}\n"
        for test, said in failed[:_ANNOTATED]
    )


def main(argv: list[str]) -> int:
    """Write the summary or the annotations, in UTF-8 whatever the console is."""
    match argv:
        case ["--annotations", report]:
            text = annotations(Path(report))
        case [report] if report != "--annotations":
            text = summary(Path(report))
        case _:
            print("usage: failed.py [--annotations] REPORT", file=sys.stderr)
            return 2
    sys.stdout.flush()
    sys.stdout.buffer.write(text.encode("utf-8"))
    sys.stdout.buffer.flush()
    return 0


def _failed(report: Path) -> list[tuple[str, str]]:
    """Each test that failed or broke in ``report``, with what it said."""
    root = ElementTree.parse(report).getroot()
    return [
        (_test(case), _said(outcome))
        for case in root.iter("testcase")
        for outcome in case
        if outcome.tag in ("failure", "error")
    ]


def _test(case: ElementTree.Element) -> str:
    """The id of the test as pytest takes it, from the dotted names in the report.

    The report joins the path of the module and the classes with dots. Classes
    are the trailing names that begin with ``Test``. A module that could not be
    collected is reported with no class and its dotted path as the name.
    """
    dotted = case.get("classname", "")
    name = case.get("name", "")
    if not dotted:
        dotted, name = name, ""
    module = dotted.split(".")
    classes: list[str] = []
    while len(module) > 1 and module[-1].startswith("Test"):
        classes.insert(0, module.pop())
    return "::".join(["/".join(module) + ".py", *classes, *([name] if name else [])])


def _said(outcome: ElementTree.Element) -> str:
    """The first line of what a test said, or why a module could not be collected.

    pytest gives every module it could not collect the same message, and the
    traceback below it ends in the lines of the error, each begun with ``E``.
    """
    message = (outcome.get("message") or "").strip()
    text = outcome.text or ""
    said = message or text.strip()
    if message == "collection failure":
        why = [line[1:].strip() for line in text.splitlines() if line.startswith("E ")]
        said = why[-1] if why else said
    first = said.splitlines()[0].strip() if said else ""
    return first if len(first) <= _LONGEST else first[: _LONGEST - 1] + "…"


def _code(text: str) -> str:
    return "`" + text.replace("`", "'") + "`"


def _escaped(text: str, also: str) -> str:
    """``text`` as a workflow command takes it: ``%`` and line ends, and ``also``, as codes."""
    for char in "%\r\n" + also:
        text = text.replace(char, f"%{ord(char):02X}")
    return text


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
