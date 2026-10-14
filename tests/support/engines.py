"""A real Docker engine for the tests that need one.

Such tests start containers. They are passed over where no engine answers or
an image is missing, unless ``COLUMBINA_REQUIRE_DOCKER`` is ``1``: then that
is a failure, so that a machine which is meant to run them cannot pass
without them. They do not fetch images themselves.
"""

import os
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import NoReturn

import pytest

from columbina import programs

__all__ = ["BARE", "IMAGE", "asked", "containers", "docker_program", "left_behind", "real"]

IMAGE = "debian:bookworm-slim"
"""A small image with bash."""

BARE = "busybox"
"""A small image without it."""


def docker_program() -> str | None:
    """Find the docker program for these tests."""
    named = os.environ.get("COLUMBINA_TEST_DOCKER")
    if named:
        return named
    found = programs.found("docker", os.environ.get("PATH", ""))
    usual = Path(os.environ.get("PROGRAMFILES", ""), "Docker/Docker/resources/bin/docker.exe")
    if found is None and sys.platform == "win32" and usual.is_file():
        # A shell that was opened before Docker Desktop was installed does not have it yet.
        return str(usual)
    return found


def asked(program: str, *arguments: str) -> tuple[int, str]:
    """Ask the engine something, and return how that ended and what it said."""
    try:
        done = subprocess.run(
            [program, *arguments], capture_output=True, text=True, timeout=120, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return 1, str(error)
    return done.returncode, done.stdout.strip()


def real(images: Sequence[str] = (IMAGE, BARE)) -> str:
    """Return the docker program, if a real engine with some images answers it.

    Otherwise the test that asks is passed over, or fails where an engine is required.
    """

    def without(why: str) -> NoReturn:
        if os.environ.get("COLUMBINA_REQUIRE_DOCKER") == "1":
            pytest.fail(f"COLUMBINA_REQUIRE_DOCKER is set, and {why}")
        pytest.skip(why)

    found = docker_program()
    if found is None:
        without("no docker program was found")
    code, system = asked(found, "info", "--format", "{{.OSType}}")
    if code != 0:
        without("no Docker engine answers")
    if system != "linux":
        without("the Docker engine runs no Linux containers")
    for image in images:
        if asked(found, "image", "inspect", "--format", "{{.Id}}", image)[0] != 0:
            without(f"the image {image} is not on this machine")
    return found


def containers(program: str) -> set[str]:
    """The names of the containers of this program that the engine has, running or not."""
    code, said = asked(
        program, "ps", "--all", "--filter", "label=columbina=1", "--format", "{{.Names}}"
    )
    assert code == 0, said
    return set(said.split())


def left_behind(program: str, before: set[str], directory: Path) -> set[str]:
    """Return the containers that are there and were not before, and remove those of a test.

    A test that fails leaves nothing on the machine. Which of the containers are its own is
    told by what they have mounted: something under the directory that the test alone has.
    Another one may belong to a program that somebody is using, and is left where it is.
    """
    left = containers(program) - before
    own = os.path.normcase(os.path.realpath(directory))
    for name in left:
        listing = "{{range .Mounts}}{{.Source}}{{println}}{{end}}"
        code, mounted = asked(program, "inspect", "--format", listing, name)
        if code == 0 and any(os.path.normcase(at).startswith(own) for at in mounted.splitlines()):
            asked(program, "rm", "--force", name)
    return left
