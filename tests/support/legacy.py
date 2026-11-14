"""Install captured old projects without keeping the old editor in production."""

import json
from pathlib import Path

from columbina.video.legacy import Entry
from columbina.video.project import identity_of
from columbina.video.transcript import Transcript


def whole(transcript: Transcript) -> tuple[Entry, ...]:
    """An old main line with all words, for reader-only unit tests."""
    return tuple(
        Entry(asset=transcript.asset, sentence=i, kept=tuple(range(first, last)))
        for i, (first, last) in enumerate(transcript.sentences)
    )


def installed(workspace: Path, name: str) -> Path:
    """Substitute the generated source files in a snapshot made by the retired editor."""
    fixture = Path(__file__).parents[1] / "fixtures/video/legacy"
    files: dict[str, str] = json.loads(
        (fixture / (name.lower().replace(" ", "-") + ".json")).read_text(encoding="utf-8")
    )
    manifest = json.loads(files["project.json"])
    directory = workspace / name
    for asset in manifest["assets"]:
        path = workspace / "raw" / asset["path"]
        before = asset["identity"]["digest"]
        identity = identity_of(path)
        asset["path"] = str(path)
        asset["identity"] = identity.model_dump()
        transcript = f"transcripts/{asset['id']}.json"
        if transcript in files:
            files[transcript] = files[transcript].replace(before, identity.digest)
        for cached in list(files):
            if cached.startswith(f"cache/{asset['id']}-{before[:12]}-"):
                files[cached.replace(before[:12], identity.digest[:12])] = files.pop(cached)
    files["project.json"] = json.dumps(manifest, indent=2) + "\n"
    for name, text in files.items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return directory
