"""Choosing the release to deploy among the tags of a repository."""

from versions import latest, parse


def versions_from_tags(tags: list[str]) -> list[str]:
    """Return the versions named by tags such as "v1.4.0", skipping the tags that name none."""
    found = []
    for tag in tags:
        if not tag.startswith("v"):
            continue
        try:
            parse(tag[1:])
        except ValueError:
            continue
        found.append(tag[1:])
    return found


def release_to_deploy(tags: list[str], *, allow_prereleases: bool = False) -> str:
    """Return the tag of the highest version among the tags."""
    return "v" + latest(versions_from_tags(tags), include_prereleases=allow_prereleases)
