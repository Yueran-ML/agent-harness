"""Semantic versions (https://semver.org), as the release scripts use them."""

import re
from collections.abc import Iterable
from dataclasses import dataclass

_PATTERN = re.compile(
    r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)


@dataclass(frozen=True)
class Version:
    """A version: three numbers, the identifiers of a pre-release, and build metadata."""

    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...] = ()
    build: tuple[str, ...] = ()

    def __str__(self) -> str:
        text = f"{self.major}.{self.minor}.{self.patch}"
        if self.prerelease:
            text += "-" + ".".join(self.prerelease)
        if self.build:
            text += "+" + ".".join(self.build)
        return text


def parse(text: str) -> Version:
    """Read a version such as "1.4.2", "2.0.0-rc.1" or "1.0.0+build.7"."""
    match = _PATTERN.fullmatch(text.strip())
    if match is None:
        raise ValueError(f"not a semantic version: {text!r}")
    major, minor, patch, prerelease, build = match.groups()
    return Version(
        int(major),
        int(minor),
        int(patch),
        tuple(prerelease.split(".")) if prerelease else (),
        tuple(build.split(".")) if build else (),
    )


def precedence_key(version: Version) -> tuple:
    """Return a key that orders versions by their precedence, lowest first.

    These are the rules of semver.org. Major, minor and patch are compared as
    numbers. A pre-release is lower than the release itself, so 1.0.0-rc.1 <
    1.0.0. Two pre-releases of the same version are compared identifier by
    identifier, from left to right: identifiers made of digits only are
    compared as numbers, other identifiers in ASCII order, and an identifier
    of digits is always lower than one that is not. When all the identifiers
    of one are equal to the first ones of the other, the shorter pre-release
    is lower, so 1.0.0-alpha < 1.0.0-alpha.1. Build metadata does not count.
    """
    if not version.prerelease:
        return (version.major, version.minor, version.patch, 1, ())
    identifiers = tuple(
        (0, int(identifier), "") if identifier.isdigit() else (1, 0, identifier)
        for identifier in version.prerelease
    )
    return (version.major, version.minor, version.patch, 0, identifiers)


def compare(a: str, b: str) -> int:
    """Return -1, 0 or 1 as version `a` is lower than, equal to, or higher than version `b`."""
    key_a = precedence_key(parse(a))
    key_b = precedence_key(parse(b))
    return (key_a > key_b) - (key_a < key_b)


def sort_versions(versions: Iterable[str]) -> list[str]:
    """Return the versions from the lowest to the highest."""
    return sorted(versions, key=lambda text: precedence_key(parse(text)))


def latest(versions: Iterable[str], *, include_prereleases: bool = False) -> str:
    """Return the highest version; pre-releases count only if `include_prereleases` is true."""
    candidates = [text for text in versions if include_prereleases or not parse(text).prerelease]
    if not candidates:
        raise ValueError("there is no version to choose from")
    return max(candidates, key=lambda text: precedence_key(parse(text)))
