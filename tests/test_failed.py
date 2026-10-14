"""What a run of the CI writes into its summary where tests failed: ``tests/support/failed.py``."""

from pathlib import Path

import pytest

from support.failed import annotations, main, summary

REPORT = """<?xml version="1.0" encoding="utf-8"?>
<testsuites>
  <testsuite name="pytest" errors="1" failures="2" skipped="1" tests="5">
    <testcase classname="tests.mcp_client.test_connection" name="test_it_waits" time="0.1">
      <failure message="assert [0.5, 1.0, 2.0] == [0.5, 1.0, 0.5]&#10;  At index 2 diff">
        long traceback
      </failure>
    </testcase>
    <testcase classname="tests.test_text" name="test_a_line" time="0.1"/>
    <testcase classname="tests.test_text" name="test_skipped" time="0.0">
      <skipped message="not here"/>
    </testcase>
    <testcase classname="tests.apps.test_cli.TestPrompt" name="test_a_key[`y`]" time="0.1">
      <failure message="AssertionError: `y` was not taken"/>
    </testcase>
    <testcase classname="tests.sandbox.test_docker_real" name="test_a_mount" time="0.1">
      <error message="failed on setup with &quot;COLUMBINA_REQUIRE_DOCKER is set&quot;"/>
    </testcase>
  </testsuite>
</testsuites>
"""


def test_each_test_that_failed_is_named_with_the_first_line_of_what_it_said(
    tmp_path: Path,
) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(REPORT, encoding="utf-8")

    assert summary(report) == (
        "### What failed\n"
        "\n"
        "- `tests/mcp_client/test_connection.py::test_it_waits`: "
        "`assert [0.5, 1.0, 2.0] == [0.5, 1.0, 0.5]`\n"
        "- `tests/apps/test_cli.py::TestPrompt::test_a_key['y']`: "
        "`AssertionError: 'y' was not taken`\n"
        "- `tests/sandbox/test_docker_real.py::test_a_mount`: "
        '`failed on setup with "COLUMBINA_REQUIRE_DOCKER is set"`\n'
    )


def test_what_a_failure_said_is_taken_from_its_text_where_it_has_no_message(
    tmp_path: Path,
) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuites><testsuite name="pytest">'
        '<testcase classname="tests.t" name="test_a">'
        "<failure>\n  first\nsecond</failure></testcase>"
        '<testcase classname="tests.t" name="test_b"><error/></testcase>'
        '<testcase classname="" name="tests.test_odd"><error message="collection failure"/>'
        "</testcase></testsuite></testsuites>",
        encoding="utf-8",
    )

    assert summary(report) == (
        "### What failed\n"
        "\n"
        "- `tests/t.py::test_a`: `first`\n"
        "- `tests/t.py::test_b`\n"
        "- `tests/test_odd.py`: `collection failure`\n"
    )


def test_a_module_that_could_not_be_collected_is_named_with_the_line_that_says_why(
    tmp_path: Path,
) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuites><testsuite name="pytest">'
        '<testcase classname="" name="tests.sandbox.test_broken">'
        '<error message="collection failure">'
        "ImportError while importing test module 'x'.\nTraceback:\n"
        "tests\\sandbox\\test_broken.py:1: in &lt;module&gt;\n    import fcntl\n"
        "E   ModuleNotFoundError: No module named 'fcntl'"
        "</error></testcase></testsuite></testsuites>",
        encoding="utf-8",
    )

    assert summary(report) == (
        "### What failed\n"
        "\n"
        "- `tests/sandbox/test_broken.py`: `ModuleNotFoundError: No module named 'fcntl'`\n"
    )


def test_a_report_in_which_nothing_failed_says_so(tmp_path: Path) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuites><testsuite name="pytest"><testcase classname="tests.t" name="test_x"/>'
        "</testsuite></testsuites>",
        encoding="utf-8",
    )

    assert summary(report) == "### What failed\n\nNo test failed. Another step did.\n"


def test_without_a_report_the_tests_did_not_get_as_far_as_writing_one(tmp_path: Path) -> None:
    assert summary(tmp_path / "junit.xml") == (
        "### What failed\n\n"
        "There is no report of the tests. An earlier step failed, or the tests did not end.\n"
    )


def test_a_report_that_cannot_be_read_is_said_to_be_such(tmp_path: Path) -> None:
    report = tmp_path / "junit.xml"
    report.write_text("<testsuites><testsuite", encoding="utf-8")

    assert summary(report) == "### What failed\n\nThe report of the tests cannot be read.\n"


def test_a_long_word_of_a_failure_is_cut_and_many_failures_are_counted(tmp_path: Path) -> None:
    cases = "".join(
        f'<testcase classname="tests.t" name="test_{n}"><failure message="{"x" * 400}"/></testcase>'
        for n in range(60)
    )
    report = tmp_path / "junit.xml"
    report.write_text(
        f'<testsuites><testsuite name="pytest">{cases}</testsuite></testsuites>', encoding="utf-8"
    )

    lines = summary(report).splitlines()

    assert lines[2] == f"- `tests/t.py::test_0`: `{'x' * 199}…`"
    assert len(lines) == 2 + 50 + 2
    assert lines[-2:] == ["", "And 10 more."]


def test_each_test_that_failed_is_an_annotation_of_the_run(tmp_path: Path) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(REPORT, encoding="utf-8")

    assert annotations(report) == (
        "::error title=tests/mcp_client/test_connection.py%3A%3Atest_it_waits"
        "::assert [0.5, 1.0, 2.0] == [0.5, 1.0, 0.5]\n"
        "::error title=tests/apps/test_cli.py%3A%3ATestPrompt%3A%3Atest_a_key[`y`]"
        "::AssertionError: `y` was not taken\n"
        "::error title=tests/sandbox/test_docker_real.py%3A%3Atest_a_mount"
        '::failed on setup with "COLUMBINA_REQUIRE_DOCKER is set"\n'
    )


def test_what_would_be_read_as_part_of_an_annotation_is_escaped(tmp_path: Path) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuites><testsuite name="pytest">'
        '<testcase classname="tests.t" name="test_a[1,2%]"><failure message="100% sure"/>'
        '</testcase><testcase classname="tests.t" name="test_b"><error/></testcase>'
        "</testsuite></testsuites>",
        encoding="utf-8",
    )

    assert annotations(report) == (
        "::error title=tests/t.py%3A%3Atest_a[1%2C2%25]::100%25 sure\n"
        "::error title=tests/t.py%3A%3Atest_b::failed\n"
    )


def test_no_more_annotations_are_made_than_a_step_may_have(tmp_path: Path) -> None:
    cases = "".join(
        f'<testcase classname="tests.t" name="test_{n}"><failure message="no"/></testcase>'
        for n in range(12)
    )
    report = tmp_path / "junit.xml"
    report.write_text(
        f'<testsuites><testsuite name="pytest">{cases}</testsuite></testsuites>', encoding="utf-8"
    )

    made = annotations(report).splitlines()

    assert len(made) == 10
    assert made[-1] == "::error title=tests/t.py%3A%3Atest_9::no"


def test_a_report_that_is_missing_or_cannot_be_read_makes_no_annotation(tmp_path: Path) -> None:
    report = tmp_path / "junit.xml"
    assert annotations(report) == ""
    report.write_text("<testsuites", encoding="utf-8")
    assert annotations(report) == ""


def test_the_summary_or_the_annotations_are_printed(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    report = tmp_path / "junit.xml"
    report.write_text(REPORT, encoding="utf-8")

    assert main([str(report)]) == 0
    assert capsys.readouterr().out == summary(report)
    assert main(["--annotations", str(report)]) == 0
    assert capsys.readouterr().out == annotations(report)


@pytest.mark.parametrize("argv", [[], ["--annotations"], ["a.xml", "b.xml"]])
def test_without_the_name_of_one_report_it_says_how_it_is_called(
    argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(argv) == 2
    assert capsys.readouterr().err == "usage: failed.py [--annotations] REPORT\n"
