"""Compact static mixer settings, without evaluating automation or reading media."""

from typing import Any


def _pan(value: dict[str, Any]) -> dict[str, Any]:
    # model/audio.rs Pan::default, including the surround controls.
    return {
        "on": True,
        "x": 0.0,
        "y": 0.0,
        "rotate": 0.0,
        "spread": 99.0,
        "divergence": 0.0,
        "lfe_db": -100.0,
        "lfe_on": False,
        "lfe_pre": False,
        **value,
    }


def _strip(owner: dict[str, Any], buses: list[dict[str, Any]], *, track: bool) -> dict[str, Any]:
    strip = owner.get("strip", {})
    # RawTrack + place_outputs: an omitted track strip goes to B1 only if it exists.
    # An explicit empty strip, and an omitted bus strip, output to no bus.
    outputs = strip.get("outputs", [])
    if track and "strip" not in owner and any(bus["id"] == "B1" for bus in buses):
        outputs = [{"bus": "B1", "on": True}]
    fx = strip.get("track_fx")
    return {
        "id": owner["id"],
        "name": owner.get("name", ""),
        "format": owner.get("format", "stereo" if track else None),
        **({"enabled": owner.get("enabled", True)} if track else {}),
        "mute": owner.get("mute", False),
        "solo": owner.get("solo", False),
        "solo_safe": owner.get("solo_safe", False),
        "fader_db": strip.get("fader_db", 0.0),
        "pan": _pan(strip.get("pan", {})),
        "track_fx": None if fx is None else {"name": "ducker", "on": fx.get("on", False)},
        "order": strip.get("order", ["fx", "dynamics", "eq"]),
        "inserts_on": strip.get("inserts_on", True),
        "inserts": [
            {
                "id": insert.get("id", ""),
                "name": insert["effect"]["kind"],
                "on": insert.get("on", True),
            }
            for insert in strip.get("inserts", [])
        ],
        "dynamics": {"on": strip.get("dynamics", {}).get("on", False)},
        "eq": {"on": strip.get("eq", {}).get("on", False)},
        "outputs": [{"on": True, **output} for output in outputs],
        "sends": [
            {"on": True, "pre": False, "level_db": 0.0, **send, "pan": _pan(send.get("pan", {}))}
            for send in strip.get("sends", [])
        ],
        "vcas": strip.get("vcas", []),
        "lanes": list(owner.get("lanes", {})),
    }


def mixer(timeline: dict[str, Any]) -> dict[str, Any]:
    """List each audio strip, routing destination and automation lane by its real id."""
    buses = timeline.get("buses", [])
    return {
        "id": timeline["id"],
        "name": timeline.get("name", ""),
        "rate": timeline["rate"],
        "tracks": [
            _strip(track, buses, track=True)
            for track in timeline["tracks"]
            if track["kind"] == "audio"
        ],
        "buses": [_strip(bus, buses, track=False) for bus in buses],
        "vcas": [
            {
                "id": vca["id"],
                "name": vca["name"],
                "fader_db": vca.get("fader_db", 0.0),
                "mute": vca.get("mute", False),
                "solo": vca.get("solo", False),
                "vcas": vca.get("vcas", []),
                "lanes": list(vca.get("lanes", {})),
            }
            for vca in timeline.get("vcas", [])
        ],
        "groups": [
            {
                "id": group["id"],
                "name": group["name"],
                "members": group["members"],
                "active": group.get("active", True),
                "linked": group.get(
                    "linked", ["editing", "fader", "solo", "mute", "sends", "panning", "automation"]
                ),
            }
            for group in timeline.get("track_groups", [])
        ],
    }
