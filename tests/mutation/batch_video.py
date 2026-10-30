"""Mutants of the editing of video, of the worktrees of tasks and of the Agent Client Protocol."""

from mutation.mutants import MutantSet, mutant

MEDIA = "src/columbina/video/media.py"
PROJECT = "src/columbina/video/project.py"
TIMELINE = "src/columbina/video/legacy.py"
TRANSCRIBE = "src/columbina/video/transcribe.py"
STORY = "src/columbina/video/story.py"
EDITS = "src/columbina/video/edits.py"
TEXTCUT = "src/columbina/video/textcut.py"
MIGRATE = "src/columbina/video/migrate.py"
WORDS = "src/columbina/video/words.py"
EDITING = "src/columbina/video/editing.py"
VIDEO_PLUGIN = "src/columbina/video/__init__.py"
ENGINE = "src/columbina/video/engine.py"
VIDEO_COMMAND = "src/columbina/apps/video_command.py"
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
        "the end of a large source is omitted from its identity",
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
        "of a word said twice the earlier one is kept",
        STORY,
        "        if able[i + 1][j]:\n            continue\n        if not line[j][1]:",
        "        if False:\n            continue\n        if not line[j][1]:",
    ),
    mutant(
        "a struck word is kept",
        STORY,
        "        if not line[j][1]:\n            kept.add(place)",
        "        if True:\n            kept.add(place)",
    ),
    mutant(
        "a word half struck through is taken",
        STORY,
        "                and alike[j] >= j + len(said)",
        "                and True",
    ),
    mutant(
        "a word without letters at the start of a line is cut from an unchanged story",
        STORY,
        "            if places[max(0, before - 1)] in kept:",
        "            if before and places[before - 1] in kept:",
    ),
    mutant(
        "a line of words without letters alone is cut though it is kept",
        STORY,
        "        return set() if _struck_out(text) else {place for place, _ in words}",
        "        return set()",
    ),
    mutant(
        "a line of words without letters alone that is struck through is kept",
        STORY,
        "        elif not struck and piece.strip():",
        "        elif piece.strip():",
    ),
    mutant(
        "the clip that plays the middle of a word does not tell it",
        STORY,
        "            owner = middles.get(index)\n",
        "            owner = None\n",
    ),
    mutant(
        "a word whose head a clip trims is not found near the clip",
        STORY,
        "    first = bisect_left(reach.ends, begins)",
        "    first = bisect_left(reach.starts, begins)",
    ),
    mutant(
        "word timing changes do not stale the story",
        STORY,
        "            [(word.start_us, word.end_us, word.text) for word in transcripts[one].words],",
        "            [],",
    ),
    mutant(
        "words of Chinese are written apart",
        STORY,
        '        gap = " " if previous and (_latin(previous[-1]) or _latin(text[:1])) else ""',
        '        gap = " " if previous else ""',
    ),
)


_EDITING = (
    mutant(
        "a dry text cut commits", EDITING, "        if not dry and ranges:", "        if ranges:"
    ),
    mutant(
        "a concurrent desktop edit is not checked before a text cut",
        EDITING,
        '                        textcut.edits_of(target["id"], ranges), revision=revision',
        '                        textcut.edits_of(target["id"], ranges)',
    ),
    mutant(
        "cached transcription is redone without asking",
        EDITING,
        "        reused = known is not None and not again",
        "        reused = False",
    ),
    mutant(
        "render cancellation leaves the export running",
        EDITING,
        "                await job.cancel()",
        "                pass",
    ),
    mutant(
        "a text cut read before the engine was reached again is applied",
        EDITING,
        "ranges), revision=revision, session=session",
        "ranges), revision=revision",
    ),
    mutant(
        "subtitles of words timed to overlap overlap",
        EDITING,
        "    return edits.untangled(items)",
        "    return sorted(items, key=lambda one: one.start)",
    ),
    mutant(
        "the sound taken out for the transcriber is kept",
        EDITING,
        "                    await asyncio.to_thread(sound.unlink, missing_ok=True)",
        "                    await asyncio.sleep(0)",
    ),
    mutant(
        "a story that is not there is told of as a file that is not found",
        EDITING,
        "            except FileNotFoundError:\n                raise story.StoryError(",
        "            except IsADirectoryError:\n                raise story.StoryError(",
    ),
)
_ENGINE = (
    mutant(
        "a revision read from another engine is taken",
        ENGINE,
        "        if session is not None and session != self._session:",
        "        if False:",
    ),
    mutant(
        "a render given up on while its start is answered goes on in the engine",
        ENGINE,
        "                await self._stopped(asked.answer)",
        "                pass",
    ),
    mutant(
        "the answer to a render's start goes with a caller that gives up",
        ENGINE,
        "await asyncio.shield(asked.answer)",
        "await asked.answer",
    ),
    mutant(
        "video editing is taken to run on every platform",
        ENGINE,
        "    return None if sys.platform in STUDIO_PLATFORMS else ONLY_WINDOWS",
        "    return None",
    ),
)
_EDITS = (
    mutant(
        "ripple delete is a lift",
        EDITS,
        '{"edit": "ripple_delete", "timeline": timeline, "clips": list(clips), "linked": linked}',
        '{"edit": "lift", "timeline": timeline, "clips": list(clips), "linked": linked}',
    ),
    mutant(
        "explicit null media marks are omitted",
        EDITS,
        "        if value is LEFT_OUT:",
        "        if value is LEFT_OUT or value is None:",
    ),
)
_TEXTCUT = (
    mutant(
        "text cuts run from first to last and shift later cuts",
        TEXTCUT,
        "for one in reversed(merged(ranges))",
        "for one in merged(ranges)",
    ),
    mutant("a locked dialogue track is cut", TEXTCUT, "    if told.locked:", "    if False:"),
    mutant(
        "adjacent cut ranges remain separate",
        TEXTCUT,
        "        if joined and one.start <= joined[-1].end:",
        "        if joined and one.start < joined[-1].end:",
    ),
    mutant(
        "a clip cuts as if it played the words of every clip",
        TEXTCUT,
        "            owned = sorted(of_clip.get(clip.id, []))",
        "            owned = sorted({index for _, index in told.owners})",
    ),
)
_MIGRATE = (
    mutant(
        "a marked legacy project is migrated again",
        MIGRATE,
        '        or (not marked and (directory / "migrated.json").exists())',
        "        or False",
    ),
    mutant(
        "legacy cuts use a changed source",
        MIGRATE,
        '        if media_by_asset[asset.id]["identity"]["digest"] != asset.identity.digest:',
        "        if False:",
    ),
    mutant(
        "migration ignores an existing source in the pool",
        MIGRATE,
        "        known = existing.get(path)",
        "        known = None",
    ),
    mutant(
        "old still facts are left unchanged",
        MIGRATE,
        '            if picture.get("still") and not known_picture.get("still"):',
        "            if False:",
    ),
    mutant(
        "one old project that cannot be migrated stops the others",
        MIGRATE,
        '                report = {"source": str(directory), "error": _why(directory, error)}',
        "                raise",
    ),
    mutant(
        "a migration given up after the engine took it is made again",
        MIGRATE,
        "    found = await _found(directory, engine)\n",
        "    found = None\n",
    ),
    mutant(
        "a migration that was undone is not made again",
        MIGRATE,
        "        for directory in sorted({*found, *undone}):",
        "        for directory in sorted(found):",
    ),
    mutant(
        "a refused migration leaves a note that a later timeline is taken for",
        MIGRATE,
        "                if error.code != UNREACHABLE:\n                    pending.unlink(",
        "                if False:\n                    pending.unlink(",
    ),
    mutant(
        "a transition that could be shorter is taken away",
        MIGRATE,
        '            if longest:\n                operation["length"] = longest',
        '            if False:\n                operation["length"] = longest',
    ),
    mutant(
        "old relative sources are looked for where this runs",
        MIGRATE,
        "    return (beside if beside.exists() else workspace / named).resolve()",
        "    return named.resolve()",
    ),
    mutant(
        "silences not found in time stop the migration",
        MIGRATE,
        "            except (media.MediaUnavailableError, media.UnreadableMediaError) as error:",
        "            except ZeroDivisionError as error:",
    ),
    mutant(
        "a directory that cannot be read stops the search for old projects",
        MIGRATE,
        "        except OSError:\n            continue\n    return sorted(found)",
        "        except ZeroDivisionError:\n            continue\n    return sorted(found)",
    ),
)
_WORDS = (
    mutant(
        "a word occurring twice is corrected without a choice",
        WORDS,
        "    if len(found) != 1:",
        "    if not found:",
    ),
    mutant(
        "words of other media are accepted",
        WORDS,
        "    if words.digest[:_DIGITS] != _digits(digest):",
        "    if False:",
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
        '[videos.checked(run, tool, path, "read") for path in args.paths]',
        "[run.cwd / path for path in args.paths]",
    ),
    mutant(
        "a name of nothing is a model in the working directory",
        VIDEO_PLUGIN,
        '        return None if value == "" else value',
        "        return value",
    ),
    mutant(
        "media that an edit adds is read without asking the file seam",
        VIDEO_PLUGIN,
        '            where = videos.checked(run, tool, path, "read")',
        "            where = Path(path)",
    ),
    mutant(
        "media that an edit adds is given to the engine as the path was written",
        VIDEO_PLUGIN,
        '"path": str(where)}',
        '"path": path}',
    ),
    mutant(
        "subtitles beside an export are written without asking the file seam",
        VIDEO_PLUGIN,
        "                    for sidecar in sidecars(target, path, chosen):",
        "                    for sidecar in []:",
    ),
    mutant(
        "every call migrates again",
        VIDEO_PLUGIN,
        "                self._migrated.add(directory)\n",
        "",
    ),
    mutant(
        "a migration that fails as a whole fails the call",
        VIDEO_PLUGIN,
        "        except Exception as error:  # A migration that fails never stops the call.",
        "        except ZeroDivisionError as error:",
    ),
    mutant(
        "a call waits for a migration however long it takes",
        VIDEO_PLUGIN,
        "            async with asyncio.timeout(_MIGRATION_SECONDS):",
        "            async with asyncio.timeout(None):",
    ),
    mutant(
        "the tools are offered where the engine does not run",
        VIDEO_PLUGIN,
        "    if missing is not None:\n        # The engine is built",
        "    if False:\n        # The engine is built",
    ),
    mutant(
        "the model is not told why there are no video tools",
        VIDEO_PLUGIN,
        "        ctx.on(RUNTIME_FACTS, facts)\n        return",
        "        return",
    ),
)
_VIDEO_COMMAND = (
    mutant(
        "a command looks for an engine where none runs",
        VIDEO_COMMAND,
        "    if missing is not None:\n        # Before any engine",
        "    if False:\n        # Before any engine",
    ),
    mutant(
        "a command that answers in JSON says it needs Windows in words",
        VIDEO_COMMAND,
        'json.dumps({"error": missing})\n            if getattr(args, "json", False)',
        'json.dumps({"error": missing})\n            if False',
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
            *_EDITS,
            *_TEXTCUT,
            *_MIGRATE,
            *_WORDS,
            *_EDITING,
            *_ENGINE,
            *_VIDEO_PLUGIN,
            *_VIDEO_COMMAND,
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
