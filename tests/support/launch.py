"""Running ``columbina run`` in a test: a profile with a scripted model, and what a run leaves."""

import json
from importlib import resources
from pathlib import Path
from typing import Any

import yaml

from columbina.agent import InboxSplicedEvent
from columbina.approval import ApprovalAskedEvent, ApprovalDecidedEvent, ApprovalPolicyEvent
from columbina.kernel import Layer, parse_layer
from columbina.retry import RetryEvent
from columbina.sandbox import SandboxModeEvent
from columbina.session import CORE_EVENTS, EventData, StoredSession, read_session
from columbina.session.store import workspace_key

__all__ = ["KINDS", "bundle", "lines", "profile", "stored"]

KINDS: dict[str, type[EventData]] = {
    kind.event_type: kind
    for kind in (
        *CORE_EVENTS,
        InboxSplicedEvent,
        RetryEvent,
        SandboxModeEvent,
        ApprovalPolicyEvent,
        ApprovalAskedEvent,
        ApprovalDecidedEvent,
    )
}
"""The kinds of events that a test reads from a stored session. The others are passed over."""


def bundle(name: str) -> Layer:
    """Return the bundle that comes with the package under a name."""
    file = resources.files("columbina.bundles").joinpath(f"{name}.yml")
    try:
        return parse_layer(file.read_text(encoding="utf-8"), source=f"bundle {name}")
    except FileNotFoundError:
        raise KeyError(name) from None


def profile(
    tmp_path: Path,
    *replies: dict[str, Any],
    extra: list[dict[str, Any]] | None = None,
    extends: str = "headless",
) -> str:
    """Write a profile that is a bundle with a scripted model, and return its path.

    Args:
        tmp_path: Where the file is written.
        replies: What the model says, reply by reply.
        extra: More operations for the profile, which come after the model.
        extends: The bundle that the profile builds on.
    """
    scripted = {
        "id": "model",
        "plugin": "columbina.llm.adapters.scripted",
        "config": {"provider": "deepseek", "replies": list(replies)},
    }
    layer = {"extends": extends, "patch": [scripted, *(extra or [])]}
    path = tmp_path / "profile.yml"
    path.write_text(yaml.safe_dump(layer, sort_keys=False), encoding="utf-8")
    return str(path)


def lines(text: str) -> list[dict[str, Any]]:
    """Read what a run reported as lines of JSON."""
    return [json.loads(line) for line in text.splitlines()]


def stored(home: Path, work: Path, session_id: str = "s-1") -> StoredSession:
    """Read the session that a run left behind."""
    return read_session(home / "sessions" / workspace_key(work) / session_id, KINDS)
