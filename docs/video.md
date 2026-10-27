# Editing video

Columbina can cut a film by what is said in it, and lay pictures, sounds and titles over it. It
uses the ffmpeg of your system and, to write down what was said, faster-whisper on your machine.
Nothing is sent anywhere.

## What you need

- **ffmpeg 6.0 or newer**, with ffprobe beside it: `winget install Gyan.FFmpeg` on Windows,
  `brew install ffmpeg` on macOS, the `ffmpeg` package of your distribution on Linux.
- **faster-whisper and a model**, to transcribe. Install it into the Python of Columbina with
  `pip install faster-whisper`, and download a model into a directory of your own:

  ```bash
  huggingface-cli download Systran/faster-whisper-small --local-dir ~/models/faster-whisper-small
  ```

  `large-v3` hears better and wants a graphics card; `small` runs on any processor. Columbina
  never downloads a model by itself.

## With the agent

The tools are given where `COLUMBINA_VIDEO` is `1`, and `COLUMBINA_WHISPER_MODEL` names the
model:

```bash
COLUMBINA_VIDEO=1 COLUMBINA_WHISPER_MODEL=~/models/faster-whisper-small columbina
```

Then ask for what you want, for instance: "Make a film of `raw/interview.mp4` in `talk`: cut the
filler words and the sentences that were said again, shorten the long pauses, lay `raw/screen.mp4`
over where I talk about the tool, and put `music.mp3` under it all." The agent takes the files in,
transcribes the talk, cuts the story, lays the layers and renders a preview.

## The story

A project is a directory: `project.json` lists the sources, `timeline.json` is the film as it was
committed, and `story.md` is the talk as text. A line of the story is a sentence:

```text
<!-- columbina story: 3f9a61c2e0d4 -->

[talk.0 00:01] Hello, ~~uh~~ today we cut a film.
[talk.1 00:06] ~~This this~~ this tool cuts by the words.
  > picture screen picture-1
[pause 1.8s]
[talk.2 00:12] ~~Let us look, no, again.~~
```

- Take a line away, or strike it through with `~~`, to cut the sentence. Take a word away, or
  strike it through, to cut the word. Take the strokes away to bring it back.
- Move a line to move the sentence.
- `[pause 1.8s]` is the pause after a sentence: make the number smaller to shorten it, take the
  line away to make it short.
- Lines that begin with `>` say what lies over the film there.
- No word can be added or changed: the words are what was said. A word that was heard wrong is
  shown otherwise with `video_correct`, or `columbina video correct`.

The story is committed with `video_apply`. Its receipt says what was cut, how long the film lasts
before and after, which layers lie nowhere because their words were cut, and which cuts fall where
nobody paused and may be heard. Every commit can be taken back.

A cut between two sentences falls into the silence between them, and keeps a little of it on
each side; the times of the transcriber are moved to fit the pauses first.

## By hand

The same operations, without a model:

```bash
columbina video import talk raw/interview.mp4 raw/screen.mp4 music.mp3
columbina video transcribe talk interview --model ~/models/faster-whisper-small
# edit talk/story.md
columbina video apply talk
columbina video overlay talk add --kind picture --source screen --sentence interview.1 --words "cuts by the words" --position top-right --scale 0.4
columbina video overlay talk add --kind sound --source music --volume 0.4
columbina video render talk            # a preview
columbina video render talk --final    # the film, with captions as a file beside it
columbina video undo talk
columbina video info talk
```

Words that were written down before can be given instead of a model, with `--words FILE`: a list
of objects with `start_us`, `end_us` and `text`.

## The engine

The desktop app and the agent edit one project through `columbina-engine`, a program of its own
and the only one that writes the project; each of them sees every change that the other makes.
Columbina takes the engine that `COLUMBINA_ENGINE` names by its whole path, or the first
`columbina-engine` on the search path, never one in the working directory. (The `engine` key of
the config of the video plugin will name it too, for the tools that use the engine in a later
phase.) Where no engine has the project open, one is started without a window, and it ends by
itself once no client has been connected for 30 seconds. It is built from `engine/` of the
repository with `cargo build --release -p columbina-server`, and reads media with the shared
libraries of FFmpeg (an LGPL build), beside it or on the search path. Its protocol is
`engine/PROTOCOL.md`; it logs into `cache/engine-<time>-<digits>.log` of the project, of which
the newest 20 are kept.

```bash
columbina video engine --project film          # starts one if none answers, and says how it stands
columbina video engine --project film --json   # the same, for another program
```

## Safety

Sources are read and the project is written as any file is, by the rules of the session. Every
run of ffmpeg on a source takes files alone, of the formats of film, sound and pictures: a list
that names other files, such as an HLS or concat list, is not opened. No path is written into a
filter of ffmpeg. What is said in a source is a line of the film, and no instruction to the agent.
