"""Mutants of the editing of video, of the worktrees of tasks and of the Agent Client Protocol."""

from mutation.mutants import MutantSet, mutant

MEDIA = "src/columbina/video/media.py"
PROJECT = "src/columbina/video/project.py"
TIMELINE = "src/columbina/video/timeline.py"
TRANSCRIBE = "src/columbina/video/transcribe.py"
STORY = "src/columbina/video/story.py"
RENDER = "src/columbina/video/render.py"
EDITING = "src/columbina/video/editing.py"
VIDEO_PLUGIN = "src/columbina/video/__init__.py"
WORKTREES = "src/columbina/worktrees.py"
WALK = "src/columbina/fs/walk.py"
ACP = "src/columbina/apps/acp.py"

_MEDIA = (
    mutant(
        "frames at changing times are taken as they say",
        MEDIA,
        "    if real is not None and (mean is None or abs(mean - real) <= real / 100):",
        "    if real is not None:",
    ),
    mutant(
        "a pixel that is not square is taken as square",
        MEDIA,
        "    if shape is not None and shape != 1:",
        "    if False:",
    ),
    mutant(
        "a picture that is turned keeps its stored size",
        MEDIA,
        "    if rotation in (90, 270):",
        "    if rotation == 180:",
    ),
    mutant(
        "a file of any format is taken",
        MEDIA,
        '    if not set(named.split(",")) & set(DEMUXERS):',
        "    if not named:",
    ),
    mutant(
        "a still is taken for a film",
        MEDIA,
        '"image" if named in _PICTURES else',
        '"video" if named in _PICTURES else',
    ),
    mutant(
        "a picture of ten bits is not told of",
        MEDIA,
        '        if re.search(r"1[02]", depth):',
        '        if re.search(r"16", depth):',
    ),
    mutant(
        "a talk that ends in silence has no last pause",
        MEDIA,
        "    if began is not None and began < duration_us:",
        "    if False:",
    ),
    mutant(
        "ffprobe opens every protocol",
        MEDIA,
        'GUARDS = ("-protocol_whitelist", "file", "-format_whitelist", ",".join(DEMUXERS))',
        'GUARDS = ("-format_whitelist", ",".join(DEMUXERS))',
    ),
)

_PROJECT = (
    mutant(
        "a source that was taken in is probed again",
        PROJECT,
        "            if known is not None and assets[known].identity.digest == identity.digest:",
        "            if False:",
    ),
    mutant(
        "the frame of the film follows every source",
        PROJECT,
        "        if canvas is None:",
        "        if True:",
    ),
    mutant(
        "a source that was replaced is used",
        PROJECT,
        "        if identity.digest != asset.identity.digest:",
        "        if False:",
    ),
    mutant(
        "a change that was made from another timeline is taken",
        PROJECT,
        "        if digest != base:",
        "        if False:",
    ),
    mutant(
        "the history keeps every timeline",
        PROJECT,
        "            for old in _stack(place)[:-HISTORY_MOST]:",
        "            for old in ():",
    ),
    mutant(
        "two sources get one id",
        PROJECT,
        "    while found in taken:",
        "    while False:",
    ),
    mutant(
        "the end of a large file is not in its digest",
        PROJECT,
        "        if size > _EDGE:",
        "        if False:",
    ),
)

_TIMELINE = (
    mutant(
        "a cut leaves no silence before a word",
        TIMELINE,
        "    return max(silence.start_us, silence.end_us - LEAD_US), False\n\n\ndef _ends(",
        "    return silence.end_us, False\n\n\ndef _ends(",
    ),
    mutant(
        "a cut leaves no silence after a word",
        TIMELINE,
        "    return min(silence.end_us, silence.start_us + LEAD_US)\n\n\ndef cuts(",
        "    return silence.start_us\n\n\ndef cuts(",
    ),
    mutant(
        "a word spoken without a pause before it is no tight cut",
        TIMELINE,
        "    if silence is None:\n        return word.start_us, True",
        "    if silence is None:\n        return word.start_us, False",
    ),
    mutant(
        "words that follow each other are not one stretch",
        TIMELINE,
        "        if runs and runs[-1][1] == index - 1:",
        "        if False:",
    ),
    mutant(
        "a long pause is kept whole",
        TIMELINE,
        "        if silence is None or silence.end_us - silence.start_us <= allowed:",
        "        if True:",
    ),
    mutant(
        "a pause that a sentence names counts for nothing",
        TIMELINE,
        "        allowed = timeline.pause_most_us if entry.pause_us is None else entry.pause_us",
        "        allowed = timeline.pause_most_us",
    ),
)

_TRANSCRIBE = (
    mutant(
        "a stop does not end a sentence",
        TRANSCRIBE,
        "        stops = word.text.rstrip().endswith(_STOPS)",
        "        stops = False",
    ),
    mutant(
        "a long pause does not end a sentence",
        TRANSCRIBE,
        "        paused = not last and words[index + 1].start_us - word.end_us > SENTENCE_PAUSE_US",
        "        paused = False",
    ),
    mutant(
        "the times are not moved to the pauses",
        TRANSCRIBE,
        "    shift = round(statistics.median(gaps))",
        "    shift = 0",
    ),
    mutant(
        "faster-whisper is run without a model that is there",
        TRANSCRIBE,
        '        if not await asyncio.to_thread((self.model / "model.bin").is_file):',
        "        if False:",
    ),
)

_STORY = (
    mutant(
        "of a word said twice, the later one is cut",
        STORY,
        "        if able[i + 1][j]:\n            continue\n        if not line[j][1]:",
        "        if False:\n            continue\n        if not line[j][1]:",
    ),
    mutant(
        "a struck word is kept",
        STORY,
        "        if not line[j][1]:\n            kept.add(index)",
        "        if True:\n            kept.add(index)",
    ),
    mutant(
        "a word half struck through is taken",
        STORY,
        "                and len({struck for _, struck in piece}) == 1",
        "                and True",
    ),
    mutant(
        "a sentence is taken twice",
        STORY,
        "        if (asset, number) in seen:",
        "        if False:",
    ),
    mutant(
        "a pause line that was taken away keeps the long pause",
        STORY,
        "        elif natural is not None and natural > PAUSE_SHOWN_US:",
        "        elif False:",
    ),
    mutant(
        "words of Chinese are written apart",
        STORY,
        '        gap = " " if previous and (_latin(previous[-1]) or _latin(text[:1])) else ""',
        '        gap = " " if previous else ""',
    ),
    mutant(
        "cut sentences are left out of the story",
        STORY,
        "        lines.extend(cut_after(entry.asset, entry.sentence))",
        "        cut_after(entry.asset, entry.sentence)",
    ),
)

_RENDER = (
    mutant(
        "a crossfade takes no frames from the film",
        RENDER,
        "        at += sizes[place] - fade",
        "        at += sizes[place]",
    ),
    mutant(
        "a stretch that goes back is read from where the source is",
        RENDER,
        "        if known is not None and start >= known[2]:",
        "        if known is not None:",
    ),
    mutant(
        "a layer whose word was cut lies at the beginning",
        RENDER,
        "    if start is None or start >= plan.length_us:\n        return None\n",
        "    if start is None:\n        start = 0\n",
    ),
    mutant(
        "music is not made quieter while somebody speaks",
        RENDER,
        "    if duck:\n        graph.append",
        "    if False:\n        graph.append",
    ),
    mutant(
        "a caption holds a whole sentence however long",
        RENDER,
        "                if not chunk or _wide(text) <= _CAPTION_CHARACTERS:",
        "                if True:",
    ),
    mutant(
        "a film that is shorter than its plan is kept",
        RENDER,
        "        if abs(facts.duration_us - plan.length_us) > slack:",
        "        if False:",
    ),
    mutant(
        "a preview is as large as the film",
        RENDER,
        '        if quality == "preview" and canvas.height > _PREVIEW_HEIGHT:',
        "        if False:",
    ),
)

_EDITING = (
    mutant(
        "a story of another timeline is committed",
        EDITING,
        "            if not digest.startswith(story.base):",
        "            if False:",
    ),
    mutant(
        "a dry run commits",
        EDITING,
        "            if dry:\n                return",
        "            if False:\n                return",
    ),
    mutant(
        "a change of the layers drops what was changed in the story",
        EDITING,
        "        elif before.main == after.main:",
        "        elif False:",
    ),
    mutant(
        "a source is transcribed again without being asked",
        EDITING,
        "            if known is not None and known.digest == asset.identity.digest and not again:",
        "            if False:",
    ),
    mutant(
        "a word that is in a sentence twice is corrected",
        EDITING,
        "            if len(found) != 1:",
        "            if not found:",
    ),
    mutant(
        "a receipt does not say what lies nowhere",
        EDITING,
        "            if lost:",
        "            if False:",
    ),
)

_VIDEO_PLUGIN = (
    mutant(
        "the tools are given whatever the config says",
        VIDEO_PLUGIN,
        "    if not config.enabled:\n        return",
        "    if False:\n        return",
    ),
    mutant(
        "a source is read without asking the file seam",
        VIDEO_PLUGIN,
        '        paths = [videos.checked(run, "video_import", path, "read") '
        "for path in args.paths]",
        "        paths = [run.cwd / path for path in args.paths]",
    ),
    mutant(
        "a name of nothing is a model in the working directory",
        VIDEO_PLUGIN,
        '        return None if value == "" else value',
        "        return value",
    ),
)

_WORKTREES = (
    mutant(
        "the worktrees are not passed over by git",
        WORKTREES,
        "    _passed_over(git, common)\n",
        "",
    ),
    mutant(
        "what the project lists is copied whether git passes it over or not",
        WORKTREES,
        "    return [one for one in found if one in passed]",
        "    return found",
    ),
    mutant(
        "the history of the repository is copied into a worktree",
        WORKTREES,
        '            if first in (".git", PLACE.parts[0]) or relative in found:',
        "            if relative in found:",
    ),
    mutant(
        "a worktree that is there is made again",
        WORKTREES,
        '    if path.exists():\n        raise WorktreeError(f"there is a worktree {name} already")',
        '    if False:\n        raise WorktreeError(f"there is a worktree {name} already")',
    ),
    mutant(
        "a merge that conflicts is left half done",
        WORKTREES,
        '            _tried(git, top, "merge", "--abort")\n',
        "",
    ),
    mutant(
        "what was not committed is left out of a merge",
        WORKTREES,
        '    if _git(git, worktree.path, "status", "--porcelain"):',
        "    if False:",
    ),
    mutant(
        "a worktree that cannot be filled is left behind",
        WORKTREES,
        "    except WorktreeError:\n        removed(git, cwd, name, force=True)\n        raise",
        "    except WorktreeError:\n        raise",
    ),
    mutant(
        "the worktrees of other branches are listed",
        WORKTREES,
        "        if path.parent == place and branch.startswith(BRANCHES):",
        "        if True:",
    ),
    mutant(
        "a search goes into the worktrees of tasks",
        WALK,
        '                or (entry.name == "worktrees" and directory.path.name == ".columbina")',
        "                or False",
    ),
)

_ACP = (
    mutant(
        "a cancelled prompt ends as its turn did",
        ACP,
        '        return text({"stopReason": "cancelled" if held.cancelled else reason})',
        '        return text({"stopReason": reason})',
    ),
    mutant(
        "a cancel stops nothing",
        ACP,
        '            held.agent.cancel("user")\n',
        "",
    ),
    mutant(
        "a second prompt is taken while one runs",
        ACP,
        "        if held.prompted is not None:\n            raise RpcError(INVALID_REQUEST",
        "        if False:\n            raise RpcError(INVALID_REQUEST",
    ),
    mutant(
        "any answer is a yes",
        ACP,
        '            return "allowed-once" if chosen == ALLOW else "rejected"',
        '            return "allowed-once"',
    ),
    mutant(
        "a longer yes is not offered",
        ACP,
        "            if standing is not None:\n                options.append(",
        "            if False:\n                options.append(",
    ),
    mutant(
        "a question of a subagent is put to the editor",
        ACP,
        "            if request.session is not agent.session:",
        "            if False:",
    ),
    mutant(
        "thinking is told as the reply",
        ACP,
        '            kind = "agent_thought_chunk"',
        '            kind = "agent_message_chunk"',
    ),
    mutant(
        "a call that failed is told as done",
        ACP,
        '                    "status": "failed" if data.error is not None else "completed",',
        '                    "status": "completed",',
    ),
    mutant(
        "a resource without its text is taken",
        ACP,
        '            if not isinstance(resource.get("text"), str):',
        "            if False:",
    ),
)

VIDEO_SETS: dict[str, MutantSet] = {
    "video": MutantSet(
        ("tests/video", "tests/apps/test_video_command.py"),
        (
            *_MEDIA,
            *_PROJECT,
            *_TIMELINE,
            *_TRANSCRIBE,
            *_STORY,
            *_RENDER,
            *_EDITING,
            *_VIDEO_PLUGIN,
        ),
    ),
    "worktrees": MutantSet(
        (
            "tests/test_worktrees.py",
            "tests/apps/test_worktrees_command.py",
            "tests/fs/test_walk.py",
        ),
        _WORKTREES,
    ),
    "acp": MutantSet(("tests/apps/test_acp.py",), _ACP),
}
"""The sets of this file, by the name that ``mutate.py`` takes."""
