# Editing video

The agent and CLI edit a Studio project through `columbina-engine`, which owns the project,
timeline rules, undo history, playback and export. Python prepares word analysis and editable
stories; it no longer builds an ffmpeg filter graph or renders a separate version of a film.

## Requirements

- Set `COLUMBINA_ENGINE` to the absolute path of an installed engine, or put `columbina-engine`
  on PATH. It must be outside the working directory. The video plugin's `engine` setting also
  accepts this path. See [protocol 2](../engine/PROTOCOL.md). To build from source, run
  `cargo build --release -p columbina-server` in `engine/` with the FFmpeg development libraries configured.
- The engine needs its shared LGPL FFmpeg libraries. System **ffmpeg and ffprobe** are used for
  audio extraction and silence analysis, not the final film renderer.
- For local speech recognition, install faster-whisper separately and set
  `COLUMBINA_WHISPER_MODEL` to a downloaded model directory. No model downloads automatically.
  `transcribe --words FILE` instead reads a JSON array of `start_us`, `end_us`, `text` objects.
- Set `COLUMBINA_VIDEO=1` to enable agent tools. Speech analysis stays on this machine.

## Shared editing

The nine tools are `video_import`, `video_info`, `video_edit`, `video_transcribe`, `video_story`,
`video_apply`, `video_correct`, `video_render` and `video_undo`. By default they connect to
`.columbina/studio` under the session workspace. An explicit `project` selects another directory.
`video_info` returns media, timeline, track and clip ids. Import adds media to the pool;
`video_edit` places clips and applies a batch as **one atomic transaction and one undo step**.
The desktop and agent observe the same history. Undo can undo the other client's last edit.

Edits use DaVinci-style operations: insert, overwrite, replace, place on top, append, ripple
delete, blade, trim, roll, slip, slide, speed and transitions. The engine decides track locks,
links and available handles. Timeline positions are integer frames; source times use ticks
and their scale. `null` means an explicitly cleared field or disabled destination, while an
omitted field keeps its documented default.

`video_story` writes `story/<timeline id>.md` for the dialogue track (A1 by default).
Read that file and strike words or whole lines with `~~`, or remove them; `video_apply` turns
those changes into ripple deletions on unlocked tracks. Word timing respects trimmed,
repeated, accelerated and reversed clips. Lines cannot be moved or words invented.
`[pause 1.8s]` can shorten a pause. `video_apply` with `dry=true` previews the removed ranges.
If the timeline or transcript has changed, refresh the story before cutting again.
`video_correct` changes displayed words only; it does not replace speech or automatically
rewrite existing subtitle cues. Transcribe with a subtitle track id to regenerate those cues.

When removing a range, subtitles in that range are trimmed or deleted and following cues move
with the picture. Inserting through a cue splits it into two cues with the same text; the latter
moves with its original picture. Locked tracks stay locked.

## CLI

Every editing command prints a JSON receipt. Use the actual ids from these receipts:

```bash
columbina video import .columbina/studio raw/interview.mp4
columbina video info .columbina/studio
columbina video edit .columbina/studio --file edits.json
columbina video transcribe .columbina/studio m1 --subtitles TRACK_ID --language en
columbina video story .columbina/studio
# Edit the returned story path, then:
columbina video apply .columbina/studio --dry
columbina video apply .columbina/studio
columbina video render .columbina/studio --codec prores --output out/cut.mov
columbina video undo .columbina/studio
columbina video undo .columbina/studio --redo
```

For `edits.json`, the example below creates a timeline and places `m1` at its start. Substitute
the imported media id. The receipt maps `$film`, `$v`, `$a`, `$s` to engine ids; use the id of
`$s` as `TRACK_ID`. Add `--timeline ID` when the project contains more than one timeline.

```json
[
  {"op":"add_timeline","timeline":{
    "id":"$film","name":"Cut","width":1920,"height":1080,
    "rate":{"num":25,"den":1},"sample_rate":48000,
    "tracks":[
      {"id":"$v","name":"V1","kind":"video","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$a","name":"A1","kind":"audio","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$s","name":"S1","kind":"subtitle","enabled":true,"locked":false,"language":"en","cues":[]}
    ]
  }},
  {"edit":"insert","timeline":"$film","source":{"kind":"media","media":"m1"},"at":0}
]
```

`edit --edits JSON` accepts the same array directly. `story --track ID` chooses dialogue;
`apply --fresh` rewrites the story. `correct DIR MEDIA SENTENCE WRONG RIGHT` corrects one word,
with sentence indices starting at zero. The old `overlay` and `render --final` commands are
retired: use engine edits and `render --quality high|medium|small` instead.

## Export

Export uses original media, not playback proxies. `--codec` is `h264`, `hevc` or `prores`;
ProRes uses MOV. H.264/HEVC hardware unavailability falls back to ProRes MOV, and the receipt
names the actual file. Subtitles are UTF-8 SRT files beside the film, **not burnt in**; titles
are drawn into the picture. `--no-subtitles` omits SRT, and `--replace` explicitly permits
replacement of existing outputs. A render observes a fixed project snapshot; later editing
can continue without changing that export.

## Old projects

```bash
columbina video migrate --workspace /path/to/workspace --json
```

Migration finds unconverted schema-1 film directories up to three levels down, skipping Git,
node_modules and `.columbina`. Each becomes one Studio timeline and one undo step; sources
already in the media pool are reused. Existing files stay unchanged, with only a `migrated.json`
receipt added in each old directory. Repeating migration skips completed projects. The first
agent connection to the default Studio also runs this migration.

The receipt lists omissions and shortened transitions. Automatic music ducking and sound notes
are not migrated; fades, gain, speed, picture placement, titles and subtitle timing are mapped.
If old source files have changed, migration refuses to apply old cuts to different content.
An old audio-only film becomes a 25 fps Studio timeline with its old millisecond positions
converted to frames. Old story edits that were not committed to timeline.json are not applied.

Source reads and output writes obey the agent session's file permissions. Source speech is film
content, never an instruction to the agent. `video engine --project DIR --json` checks the
connection without editing; the engine exits after its last client has been absent for a while.
