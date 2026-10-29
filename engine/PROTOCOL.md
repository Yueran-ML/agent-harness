# The engine protocol (version 2)

`columbina-engine` is the only program that writes a project. The desktop app and the agent
connect to the same running engine, send it changes, and are told of every change in one order.
This is what a client is written against.

## Starting and finding an engine

```
columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]
```

- The engine reads media with libav (FFmpeg's libraries, an LGPL shared build), which it finds
  beside itself or on `PATH`: on Windows `avcodec-62.dll`, `avformat-62.dll`, `avutil-60.dll`,
  `swscale-9.dll` (and `swresample-6.dll`, which they may use). Without them it cannot start
  (on Windows it ends at once with `0xc0000135`).
- To make proxies (below), an engine that is not `--headless` runs the `ffmpeg` of the same LGPL
  build, the first of: the file that `COLUMBINA_PROXY_FFMPEG` names, if it is an **absolute**
  path (one that is not is not taken, and is logged); `ffmpeg.exe` (`ffmpeg` elsewhere) **beside
  the engine**; and, in a debug build only, the `bin` of the `FFMPEG_DIR` it was built with. It
  never runs an `ffmpeg` found on `PATH`, which may be of the GPL. Without one, the engine works
  all the same, and each proxy that it would make fails. The `ffmpeg` found is logged
  (`proxy_ffmpeg`).
- The directory is made if it is not there. The engine takes the lock of the project, opens
  `project.json` (a new, empty project if there is none), and listens.
- Its first line on **stdout** is one JSON object, written once it listens:
  `{"listening": "<address>", "pid": 1234, "protocol": 2}`. Nothing else is ever written to
  stdout.
- **stderr** carries logs, one JSON object a line (`{"event": "kicked", ...}`), written by a
  thread of their own: the engine never waits for stderr. Its owner may read it, leave it
  unread, or close it; lines that cannot be written are dropped and counted
  (`{"event": "log_dropped", "lines": n}`).
- **Address.** A client never makes the address itself: it takes it from the first line on
  stdout or from `engine.lock` (below), and opens it as it is given: Node
  `net.connect(address)`, Python `asyncio.open_unix_connection(address)` or, on Windows, the
  proactor loop's `create_pipe_connection` (wrap it in a timeout: it retries a busy pipe for
  ever). For the record, the address is made from `<hash>`, the first 16 lowercase hex digits of
  the SHA-256 of the project directory as a full path (as `std::fs::canonicalize` gives it; on
  Windows `\\?\C:\…` is written `C:\…` and `\\?\UNC\server\…` is written `\\server\…`):
  - Windows: the named pipe `\\.\pipe\columbina-engine-<hash>`.
  - Unix: the socket `<hash>.sock` in `$XDG_RUNTIME_DIR/columbina/`, or, without
    `XDG_RUNTIME_DIR`, in `${TMPDIR:-/tmp}/columbina-<uid>/`. The directory is made with mode
    0700, and the engine refuses to start (exit code 2) if it is not a directory of this user
    that only this user can open. The socket is never in the project directory, whose path may
    be too long for a socket or be on a file system without sockets.
- **Ending.** A `--headless` engine ignores stdin and ends once no connection has been open for
  `--idle-seconds` (30 unless given), counted from its start and from each time the last
  connection closes; a connection made in that time keeps it. An engine that is not headless
  belongs to whoever started it (the desktop) and ends when its **stdin is closed** (the owner
  closes it, or dies); it does not end for lack of connections. So an engine that is not
  headless and is started with stdin closed or empty (`NUL`, `/dev/null`) ends at once: agents
  and command-line tools use `--headless`. Every engine also ends on Ctrl+C, on Ctrl+Break and
  when its console is closed, the user logs off or the system shuts down (Windows), and on
  SIGTERM and SIGHUP (Unix). As it ends, the engine stops listening, closes every connection,
  saves the project, and removes `engine.lock`.
- **Exit codes.**

  | code | meaning | stderr |
  |---|---|---|
  | 0 | ended as asked (idle, stdin closed, a signal, `engine.yield`) | |
  | 1 | stopped by an error after it listened (taking connections failed 20 times in a row, or the last save failed) | `{"event": ...}` lines |
  | 2 | could not start: bad arguments, a `project.json` that cannot be read, no directory for the socket, or the address held by another program for more than about 2 s | `{"error": "..."}` |
  | 3 | the project is open in another engine: connect to it instead | `{"held": {"pid": 1234, "address": "..."}}`, or `{"contended": true}` when that engine has not yet said where it listens (try again shortly) |

- **`engine.lock`** in the project directory is `{"pid": ..., "address": "...", "started":
  "<RFC 3339>"}`, written by the engine that holds the lock. Clients only ever read it. It may
  name an engine that has just died (the lock of the system was let go, but the file stays until
  the next engine writes over it): **a client that cannot connect to the address it names reads
  `engine.lock` again after 50–100 ms and tries what it says then, or starts an engine of its
  own.** The engine writes `engine.lock` before it listens, so a fresh one can fail to connect
  for a moment too.

## Framing

- One JSON-RPC 2.0 message per line, UTF-8, ending in `\n` (`\r\n` is read too). Empty lines are
  ignored.
- A request line may be at most **16 MiB** (without its newline). A longer one is answered with
  `-32600` `"request line exceeds 16 MiB"` (id `null`), skipped up to its newline, and the
  connection goes on. Lines from the engine have no limit (`project.get` of a large project).
- A request carries `"jsonrpc": "2.0"`, a `method`, an `id`, and, where the method takes them,
  `params`. The **id** is a string or a whole number, at most 256 bytes as written (quotes
  included); any other id (`null`, a fraction, an object, a longer one) is refused with
  `-32600` and id `null`. **Params** are an object; a method that takes none takes them left
  out, `null` or `{}`.
- A message without an `id` is a notification; the engine defines none from clients and ignores
  them. Batches (arrays) are refused (`-32600`).
- Requests on one connection are answered in the order they were sent; a client may send many
  before reading the answers.

## Order of messages

Each connection has one outgoing queue. Every change to the project makes a new **revision**
(0 when the engine opened the project, then +1 per transaction, undo and redo). A change and
everything sent about it happen under one lock:

1. the change is applied;
2. `project.changed` is queued on **every connection that said hello**, the one that asked
   included;
3. the answer is queued on the one that asked.

So every client sees every change in revision order, with no gaps, and a client sees its own
change as a notification **before** the answer to its request. `hello` and `project.get` are
answered in the same order: a client sees exactly the changes with a revision above the one in
those answers.

**A client that does not read** fills its queue (4096 messages, or 128 MiB, whichever comes
first; a single message is always queued if the queue is empty). The engine then closes its
connection rather than drop a single message, and logs `{"event": "kicked", ...}`. A client
whose connection was closed reconnects, says hello and calls `project.get`.

## Methods

Before `hello`, every other request is refused with `-32600`.

| method | params | result |
|---|---|---|
| `hello` | `{"client": "desktop" \| "agent" \| "cli", "protocol": 2}` | `{"engine": "0.1.0", "protocol": 2, "revision": n, "project": "<dir>", "pid": 1234, "headless": false}` |
| `engine.yield` | none | `{}` (desktop only, to a `--headless` engine; see below) |
| `project.get` | none | `{"revision": n, "next_id": k, "project": {...}}` |
| `timeline.duration` | `{"timeline": "t1"}` | `{"frames": n}` |
| `transaction.apply` | `{"ops": [op_or_edit, ...], "revision": n}` (`revision` optional) | `{"revision": n, "done": [op, ...], "next_id": k, "names": {"$a": "c7", ...}, "clamped": [i, ...]}` |
| `history.undo` | none | `{"revision": n, "done": [op, ...], "next_id": k}`, or `{"revision": n, "done": null}` if there is nothing to undo |
| `history.redo` | none | as `history.undo` |
| `media.import` | `{"paths": ["<file>", ...], "bin": "<bin id>"}` (`bin` optional) | `{"revision": n, "added": ["m3", ...], "next_id": k}` |
| `media.inspect` | `{"paths": ["<file>", ...]}` | `{"media": [media, ...]}` (empty ids; no project change) |
| `media.make_proxy` | `{"media": "m1"}` | `{}` (see Proxies) |
| `media.thumbnail` | `{"media": "m1", "frame": 50, "height": 90}` (`frame` optional) | `{"width": 160, "height": 90, "png": "<base64>"}` |
| `media.waveform` | `{"media": "m1"}` | `{"rate": 100, "channels": 2, "peaks": "<base64>"}`, or `{"pending": true}` |
| `view.attach` | `{"monitor":"source" \| "timeline", "parent": "<handle>", "x": x, "y": y, "width": w, "height": h}` | `{"adapter": "<name>", "backend": "Dx12"}` |
| `view.place` | `{"monitor":"source" \| "timeline", "x": x, "y": y, "width": w, "height": h}` | `{}` |
| `view.show` | `{"monitor":"source" \| "timeline", "shown": true \| false}` | `{}` |
| `view.detach` | `{"monitor":"source" \| "timeline"}` | `{"detached": true \| false}` |
| `transport.*` | see The transport | |

- `hello` says once per connection what the client is; a second `hello` is refused (`-32600`).
  From then on the connection is sent every notification. `protocol` other than 2 is refused
  (`-32600`, `data: {"reason": "protocol", "supported": [2]}`). A refused hello does not
  register the connection: it may retry with version 2. `project` is the directory as the engine writes it (see
  Address); `pid` is the engine's; `headless` whether it was started `--headless` (an agent's or a
  command's engine, with no view, no transport and no proxies) or is the desktop's own.
- `engine.yield` hands the project over from a headless engine to the desktop. When the desktop
  opens a project and finds it held by an engine whose `hello` says `"headless": true` (an
  agent's), it calls `engine.yield` on it, waits for that engine's process (`pid` of `hello`, or
  of `engine.lock`) to end, and starts its own engine on the project. The headless engine tells
  every connection that said hello `engine.yielding` `{"to": "desktop"}` (below), answers `{}`,
  and, once these are written (or after 2 s at most), ends as it does when it is asked to: it
  takes no more connections, closes every one it has (the agent's too), saves the project, lets
  the lock go and ends with 0 (logged `stopping`, with the reason). **A client told
  `engine.yielding`** does not start an engine of its own at once: it waits, up to about 5 s, for
  `engine.lock` to name a new engine (the desktop's, which it then connects to), and only then
  starts one itself; so that it does not take the project in the moment before the desktop's
  engine writes `engine.lock`. Only the desktop may call it (`-32600` from another client); an engine that is
  not headless (the desktop's own) answers `-32601`: it ends when its stdin is closed.
- `timeline.duration` reads the greatest end frame of any clip or subtitle, including disabled
  and locked tracks (0 when empty). It changes no revision or history. A missing timeline is
  `-32602`, `data: {"reason": "missing", "timeline": "..."}`. Unknown params are refused.
- `transaction.apply` applies basic operations and edits in order, all or none; an empty list or
  a batch that does nothing is refused. Edits are expanded against the project left by earlier
  entries. The whole batch makes one undo step. `done` contains only basic operations, as they
  were done (see below), never edits. `clamped` contains the zero-based input indexes of edits
  that went less far than requested, including repaired transitions that had to be shortened.
  `names` maps each `$name` of the
  transaction (see below) to the id the engine gave it; `{}` if there were none.
  An optional `revision` binds the request to the snapshot used to prepare its ranges. The
  engine checks it under the same lock as the change; a mismatch refuses the whole batch
  with `-32001`, `data: {"reason":"stale_revision","expected":n,"actual":k}` and no history
  change. Omit it for commands meant to operate on the current state.
- `media.inspect` reads identities and facts under the same path and read limits as
  `media.import`, returning media in input order with empty ids. It does not change the
  project, revision or undo history, and starts no proxies. Clients may name these media and
  add them alongside a timeline in one `transaction.apply`, so a whole migration is one undo.
- `media.import` reads each file (its identity, and what it holds, as below) and adds it to the
  optional `bin` (otherwise the root) in the same transaction; an unknown bin refuses the
  whole import, without changing the project or its history. It adds the media to the
  end of the media as **one transaction** of `add_media` operations, one for each path in order,
  each given an id by the engine; `added` lists them in the same order. Every path must be
  **absolute** and UTF-8; a relative one is refused (`-32602`, `data: {"index": i, "path":
  "...", "reason": "relative"}`) before any file is read. If any file cannot be read, **nothing**
  is added, no revision is made, and the error (`-32002`) names the first path in the list that
  could not be read; once one cannot be read, no other is begun. If the media that was read is
  refused as an operation (`-32001`, as `transaction.apply` is refused: `too_large` for a file
  whose size or duration is beyond 2^53 − 1), nothing is added either. At most 8 files are read
  at once, of all the calls together. Other connections are answered while the files are read
  (requests on the same connection are answered in order, after it). A file that is taken in
  twice is two media, as in an editor. A path is stored as the system writes it
  (`C:\media\a.mp4` on Windows). Once they are added, the engine makes a **proxy** of each that
  needs one (see Proxies).
  - `identity`: `size`, `modified_us` (microseconds since 1970, UTC) and `digest`: the SHA-256 of
    the size in decimal, the first 64 KiB, and the last 64 KiB from no earlier than byte 65536,
    as the Python editing makes it, in 64 lowercase hex digits (anything else is refused as
    `out_of_range` `digest`).
  - A file is read as one of the formats MP4/MOV, Matroska/WebM, AVI, MPEG-TS (and M2TS),
    MPEG-PS, WAV, MP3, FLAC, Ogg, AAC, PNG, JPEG and WebP, and as a file only: a playlist (HLS)
    or a list of `concat` is not media.
  - `facts.duration` is in the time base of the picture if there is one, else of the sound: a
    time base of `num/den` seconds is the `scale` `den`, and the duration counts its ticks times
    `num`, from the start of that stream (its first frame, which need not be at 0 in the file)
    to the end of its last frame. Where the file does not say how long the stream lasts (as
    Matroska does not), its end is read from its last packets. A `source_in` of a clip of the
    media is in this scale, from the same start.
  - `facts.video`: `width` and `height` as stored (before rotation); `rate` the rate of the
    timestamps (`r_frame_rate`), or, where the mean rate is more than 1% from it, the standard
    rate nearest to the mean; `codec` as ffprobe names it (`h264`, `hevc`, `prores`); `bits` of
    luma; `rotation` how far the picture is turned **clockwise** to be shown upright (0, 90,
    180, 270; a file made with `-display_rotation 90` has 270, where ffprobe, and the Python
    editing, say 90 counterclockwise: a project of it is moved over as `(360 − r) % 360`). A
    mirrored picture keeps its turn, and its mirror is not told. No `video` for sound only, no
    `audio` for a picture only; a file with neither cannot be read, and neither can one whose
    picture libav cannot decode.
  - `facts.video.still` is `true` for the PNG/JPEG/WebP image demuxers; it is optional and
    defaults to `false` (omitted when false). A still clip holds image frame zero for its
    chosen timeline length; the single-packet duration does not limit trims or transitions.
    Insertion without a length or mark out uses five seconds, rounded down to timeline frames.
    Normal video, including a one-frame movie, keeps its source-range limits.
- `history.undo` takes back the latest step **whoever made it** (desktop and agent share one
  line of undo); `history.redo` does again the step undone last, with the same ids. A new
  transaction clears what could be redone. 1000 steps are kept.
- `view.*`: the viewer, below; `transport.*`: playing a source, below.

## Thumbnails and waveforms

Both methods work in headless engines and change no project revision or undo history. They
read a snapshot of the media without keeping the project locked. Unknown params are refused;
a missing media id is `-32602`, `data: {"reason":"missing", "media":"..."}`. A file that
cannot be opened or decoded is `-32002`, with `path`, `reason` and `message` as for import.

- **Thumbnails.** `height` is an integer from 1 through 270, in display pixels. Width follows
  the display aspect ratio (rounded to the nearest pixel, at least 1); the PNG is upright RGBA,
  with media rotation applied. The first-phase limited BT.709 conversion matches the preview.
  `frame` is a nonnegative frame number at the media's rate. If omitted it is
  `floor(ceil(duration * rate) / 2)`. A number outside the duration is `-32602`; a source without
  video is `-32002`, reason `no_video`. A usable proxy is preferred, with fallback to the
  original if the proxy cannot open or decode. At most two thumbnails are decoded at once.
- **Waveforms.** Bin `n` describes media time `[n/100, (n+1)/100)` seconds. The last partial
  bin is included. For each bin, channel 0's minimum and maximum come first, then channel 1's,
  and so on, in native channel order. Every native sample contributes: the service does not
  resample to 100 Hz. Each extremum is clamped to `[-1,1]`, multiplied by 127, rounded to the
  nearest integer (half away from zero), and stored as a signed i8 byte in two's complement.
  Missing sound is zero. For video media time zero is the first video frame, so audio pre-roll
  is omitted; for audio-only media it is the audio stream start. A source without audio is
  `-32002`, reason `no_audio`. Audio is always read from the original, not the picture proxy.
- **Background work.** An uncached waveform returns `{"pending":true}` and runs on a separate
  single-worker queue, leaving the thumbnail queue available. Repeated requests for the same
  pending source share the job. Completion sends `media.waveform` with `{"media":"m1"}` to
  all clients that said hello, provided the same source is still in the project. Query again
  after this notification to receive the data or a decode error; the notification does not
  assert success. An error is consumed by the next query, and a further query can retry (for
  example after an offline file is restored). Shutdown cancels queued jobs and stops decoding
  at the next audio block.
- **Caches.** PNG files live in `cache/thumbs/<media>-<digest12>-<frame>-<height>.png`;
  waveforms in `cache/waveforms/<media>-<digest12>.bin`. The digest is the existing imported
  identity, not a newly computed hash. Punctuation in media ids is percent-encoded in filenames.
  Files are replaced atomically. Malformed caches are rebuilt; a cache that cannot be written
  does not prevent a computed answer. Valid caches can be read while the source is offline,
  including after restart. A redirected cache directory or a file symlink is not read through.
  Waveform files start with `CWAV1`, a little-endian u16 channel count, then the peak bytes.
- **Size limits.** Thumbnail width must not exceed 16384 pixels (`-32602`). Encoded answers
  must fit the 16 MiB protocol line limit, leaving 1024 bytes for JSON; an oversized waveform
  or PNG returns `-32002`, reason `too_large`. Waveform size is checked before allocation;
  PNG size is checked before base64 response encoding.

## Proxies

Media whose picture is slow to seek in gets a **proxy**: a copy of its picture whose every frame
is a key frame, so that any frame is decoded alone, in a few milliseconds. Scrubbing it shows the
frame itself at once, and playing it backward shows every frame. The engine makes proxies by
itself; a client only sees them, and may turn their use off (`transport.proxies`).

- **Which media.** A picture of long groups of frames (a key frame, then frames decoded from it)
  that also has more than 1080 rows, more than 8 bits, HEVC or AV1, or B-frames: what cameras and
  phones record. Not media whose every frame is a key frame (ProRes, DNxHR, MJPEG, H.264 or HEVC
  of intra frames only), and not H.264 of 8 bits up to 1080p without B-frames. The engine tells
  from the first 64 packets of the picture.
- **When.** After any transaction that adds media (including `media.import` and redo),
  for the media it added, and when the engine starts, for
  every media item of the project that has no proxy, has one made from another file
  (`digest_of_source` is not the digest of its `identity`), or has one whose file is gone (so a
  proxy that a client took away with `set_proxy` comes back at the next start). One is made at a
  time, at a low priority of the system. Only an engine that is not `--headless` makes them (it
  has the transport that reads them): media taken in by a headless engine gets its proxy when
  the desktop's engine opens the project. Media of the same file as other media (the same
  digest, and the same facts: the digest reads only the size and the first and last 64 KiB) is
  given that media's proxy, which is not made twice.
- **What.** HEVC of intra frames only, made by NVENC (on an NVIDIA card; else ProRes 422 Proxy),
  at most 1920 pixels on its long edge, its shape kept; every frame of the media, with its
  timestamp, so that frame `n` of the proxy is frame `n` of the media (the frames of the proxy
  are numbered at the rate of the media, whatever its container says); no sound (the sound is
  always read from the media). Before it is set, it is checked: its first and last 8 frames are
  numbered as those of the media, and its frames are key frames. It is written to
  `<project>/cache/proxies/<media id>-<12 hex digits of the digest>.mov`, as `….mov.part` until
  it is whole. Making a proxy takes a fraction of the time the media lasts (some 2 s for 10 s of
  4K HEVC on an RTX 4080).
- **Set on the media** as `proxy` (`project.json`, below) by a change of the **engine** itself: a
  `project.changed` with `by` and `made_by` `"engine"`, and `done` one `set_proxy` (the file of a
  proxy that it takes the place of, beside it in `cache/proxies` and had by no other media, is
  deleted). It is **no step of the history**: an undo never takes it back, and it never lets go of
  what could be redone. A client may set or take away a proxy too, with `set_proxy` in a
  transaction, which is undone as any other.
- **What it is at** is told to every connection that said hello in `media.proxy` (below):
  `queued` once the media is found to need one, `running` as it is made (with `progress` from 0 to
  1, at most once a second), then `done`, or `failed` with a `message`. The proxy of media that is
  **removed** is not made: the one being made is stopped, its `.part` deleted, and it is told
  `failed` with the message that the media was removed; and so is the one being made when the
  engine ends (without a notification), to be made again by the next engine. An `ffmpeg` that
  writes no frame for 60 s is stopped, and its proxy `failed`. On Windows the `ffmpeg` of an
  engine that is killed ends with it (a job object of the system); elsewhere it ends once it is
  done.
- **What could not be made** by any encoder (each failed, or made a proxy that is not numbered as
  the media) is remembered by the digest of the file in `cache/proxies/failed.json`, and not tried
  again, at an import or a start (it is told `failed`, with the message that it failed before),
  until a client asks for it with **`media.make_proxy`** `{"media": "m1"}`: that forgets the
  failure and makes the proxy again if the media needs one (answered `{}` at once; what comes of
  it is told in `media.proxy`). Media not in the project is refused (`-32602`, `reason:
  "missing"`); an engine that makes no proxies answers `-32601`. Deleting `failed.json` forgets
  every failure, at the next start. A failure that is not the file's (the engine ended, the media
  was removed, no `ffmpeg`, a file that cannot be read, an `ffmpeg` that wrote nothing for 60 s,
  or one whose `ffmpeg` said that the disk is full or that a file may not be written: "no space
  left", "not enough space", "disk full", "disk quota", "permission denied", "access is denied",
  "read-only file system", "error opening output", "could not open file") is not remembered.
- **The cache.** When the engine starts, the files of `cache/proxies` that no media of the project
  names (the proxies of media removed since; the history is not kept from one engine to the next,
  so nothing could bring them back) are deleted, and so are `.part` files that an engine that was
  killed left. A media item that is removed keeps its proxy file until then, so that an undo has
  it back.
- **The file of the media.** A proxy is made only from a file whose identity is still that of
  the media; one that was changed since it was taken in fails ("the file of the media is not the
  one that was taken in").
- **A proxy that cannot be read**: one whose file is gone, or that cannot be opened, is let go for
  the media itself when it is loaded; one whose frames cannot be decoded (a damaged file) is let
  go for the media itself at once, from the frame at the playhead, playing on if it played, and
  its file is deleted. Either is made again, and read as soon as it is made (see The transport).
  A proxy that could not be read is not read again until it is made again.

## The project as it is written

`project.json` is the project as `project.get` gives it (`crates/model/src/project.rs`). A media
item may have `proxy`, which is left out when it has none:

```json
"proxy": {"path": "C:\\p\\cache\\proxies\\m1-0eb91568ee7d.mov", "width": 1920, "height": 1080,
          "codec": "hevc", "digest_of_source": "0eb91568ee7df39f…"}
```

`path` is a file of the engine's own (see Security): `<id>-<12 hex digits>.mov` directly in
`<project>/cache/proxies`; `width` and `height` as stored; `codec` as ffprobe names it
(`hevc`, `prores`); `digest_of_source` the `identity.digest` of the media it was made from. A
project written before proxies has none, and is read as it is. A proxy whose `path` is not of
the engine's own is taken away as the project is read (logged `proxy_dropped`), and the rest of
the project is read as it is.

### Schema 2

The top level is `{schema: 2, next_id, bins: [...], media: [...], timelines: [...]}`. A schema 1
project is upgraded on open; its original bytes are kept once in `project.schema1.json`.
Unknown fields are refused. `project.get` and `project.changed` are the source of truth for
clients; a client must not write `project.json` itself.

- A bin is `{id, name, parent}` (`parent: null` for the root). Bins cannot contain themselves,
  and a nonempty bin cannot be removed. Media adds `name`, nullable `bin`, `mark_in` and
  `mark_out`; marks use the media duration's time scale.
- A timeline is `{id, name, rate: {num, den}, width, height, sample_rate, tracks}`. Width and
  height are even integers, 16–16384; sample rate is 44100, 48000 or 96000. Frames are integers
  at the timeline rate. Video tracks stack in list order, the first at the bottom.
- Every track has `id`, `kind`, `name`, `enabled`, `locked`. Video/audio tracks additionally
  have `clips` and `transitions`; subtitle tracks instead have `language` and `cues`. Fields of
  another track kind are refused, even empty arrays. Track ids are unique within a timeline.
- A clip is `{id, start, length, source, enabled, speed, reverse, link, transform, gain_db, pan,
  fade_in, fade_out}`. `start >= 0`, `length > 0`; same-track clips cannot overlap.
  `speed: {num, den}` is a positive rational in [1/8, 8]; direction is the separate `reverse`
  flag. `link` is a group id or `null`. Time conversion uses integer ticks, rounded down, so a
  cut need not be an exact sample boundary for arbitrary source time bases.
- A stored `source` is `{kind:"media", media, in:{ticks,scale}}`,
  `{kind:"color", color:"#rrggbbaa"}`, or `{kind:"title", text, style}`.
  Title style is `{font, size, color, x, y, align}`: size 1–4096 in timeline pixels; x/y in
  [0,1]; align `left|center|right`. Text is at most 1000 characters.
- Colors accept exactly eight hexadecimal digits after `#`, with either letter case; returned
  operations and saved projects normalize them to lowercase `#rrggbbaa` (alpha last).
- `transform` is `{x,y,scale,rotation,opacity}`: offsets as shares of timeline width/height,
  scale in (0,16], clockwise degrees, opacity in [0,1]. Gain is −96…24 dB; pan is −1…1.
  Fades count timeline frames from the respective ends. A fade and transition at the same end
  are refused; clear the fade explicitly to replace it with a transition.
- A transition is `{id,kind,left,right,length,align}`. Kind is `dissolve|dip_black|dip_white`
  on video or `crossfade` on audio; alignment is `start|center|end`. Its clips must be adjacent
  on one track, have enough source handles and enough length beside their other transitions.
- A subtitle cue is `{id,start,length,text}`, with a nonnegative start and positive length.
  Cues cannot overlap on the same track. `language` is a short BCP 47 tag or an empty string.

Integers written into the project and edit frame/tick inputs stay within ±(2^53−1), so clients
using JavaScript preserve them exactly. See `crates/model/src/project.rs` for the complete
serialized types and `crates/model/src/ops.rs` for validation.

## The view

The engine draws the picture of the viewer itself, with the graphics card, into a native child
window that it makes inside the desktop's window, over the rectangle that the page keeps for
the viewer (`crates/view`). No frame crosses between the processes; the page never sees the
picture.

- **Monitors.** Every `view.*` request must name `monitor: "source"` or `"timeline"`.
  There is one independent native window for each monitor. Missing, null or other monitor
  values are refused (`-32602`); protocol 2 does not default to the source monitor.
- **Who sends them.** Only the desktop (`hello` with `"client": "desktop"`) may call `view.*`;
  another client is refused with `-32600`. Within the desktop, only the **Electron main
  process** sends them: it owns the window and its handle. The page (the renderer) never gets
  the handle and never calls the engine for the view: it tells the main process where the
  viewer is (over IPC), and the main process sends `view.place`. A `--headless` engine draws in
  no window: every `view.*` is answered `-32601`.
- **`parent`** is the native handle of the window, written in **decimal**, as a string: on
  Windows the `HWND` of `BrowserWindow.getNativeWindowHandle()` (a Buffer of 8 bytes: read it
  with `readBigUInt64LE()` and send its `toString()`; a signed reading is taken too).
- **Rectangles** are in **device pixels** (physical pixels of the screen, not CSS pixels),
  relative to the top left corner of the parent's **client area** (the window without its frame
  and title bar). From the page: `getBoundingClientRect()` of the viewer's element, times
  `devicePixelRatio`, rounded (the left and top rounded, then the right and bottom rounded, and
  the width and height taken between them, so that neighbours share their edges). The engine
  is aware of the DPI of each monitor (`PER_MONITOR_AWARE_V2`), as Electron is, so that these
  are its pixels too. **The window must have no menu bar**: a menu takes the top of the client
  area, and the page's rectangles would be off by its height. The app calls
  `setMenuBarVisibility(false)`; it must also never show the menu (no `autoHideMenuBar` that
  Alt brings back; `Menu.setApplicationMenu(null)` or `removeMenu()` is surest). Width and
  height are 0 to 16384; a view of no pixels draws nothing.
- **`view.attach`** makes the view in `parent` at the rectangle, shown and **black**, and makes
  the graphics card ready to draw in it; it is answered once both are done (about 100–300 ms;
  other connections are answered meanwhile). `adapter` and `backend` say what draws. There is
  **one view per monitor**: another `view.attach` for that monitor replaces only its view,
  in the same or another parent. Both share the engine's GPU. The view is the engine's, not the connection's: it stays when the
  connection that made it closes, until `view.detach`, another `view.attach`, or the end of the
  engine. A desktop that reconnects attaches again.
- **A connection of its own.** `view.attach` takes about 100–300 ms, and up to some 20 s when
  the graphics card is slow to answer (a driver that is resetting); requests on the same
  connection are answered in order, so they wait behind it. The Electron main process should
  send `view.*` on a **connection of its own**, so that nothing else of the desktop waits for
  the view.
- **`view.place`** moves and sizes the view, as the page's layout changes (the window resized,
  a panel dragged). It is answered at once; the window follows a moment later (it is moved by
  a thread of the engine that serves it, and drawn again at its new size). Send it whenever the
  viewer's rectangle changes, and after the parent's DPI changes.
- **`view.show`** hides the view, and shows it again. A native window covers whatever of the
  page lies over its rectangle ("airspace"): an HTML menu, dialog or tooltip that opens over the
  viewer is hidden under it. So the page hides each affected monitor (`{"monitor":"source",
  "shown":false}` and likewise for `timeline`) as such an overlay opens and shows it again
  as the overlay closes; meanwhile the page beneath shows (it may draw
  the last frame there, or nothing). A view that is hidden draws nothing. The view is put on
  top of the parent's other child windows each time it is moved, shown or hidden (`HWND_TOP`),
  so a native window of the app's own over the viewer (another view of Electron, a
  `WebContentsView` laid over it) is covered too: the app hides the view while such a window
  lies over the viewer, as for an overlay of the page.
- **Input** goes to the page: the view is a disabled window that never takes the focus, so the
  clicks, wheel and keys of the user over the viewer go to the page's own element there, as if
  the view were not there.
- **What is drawn.** Black, until a frame is given by a transport, not by a JSON view method.
  A source picture is fitted into the view, its
  shape kept and centred, with black bars, turned as its media says, its colours taken as
  BT.709. The engine draws only when there is something new to draw (a picture, a new size, the
  window shown or uncovered), never on a clock, and with v-sync. A timeline frame already
  composed as RGBA on the same GPU is sampled directly, fitted with black bars; it is neither
  read back to CPU memory nor converted through YUV again. On device replacement old composed
  textures are cleared and the producer must recompose on the new GPU. CPU source pictures
  can be uploaded again. Window lifecycle logs include `monitor`.
- **`view.detach`** destroys only that monitor's view; `detached` says whether there was one.
- **Failures.** A view whose window was destroyed with its parent (the window closed) is let
  go: `view.place` and `view.show` are then answered `-32003` with `reason: "gone"`, and the
  desktop attaches again to its new window. If drawing a frame takes more than 1 s (the card
  hangs, a driver resets), the view hides itself, so that the page is seen rather than a frozen
  picture, and shows itself again once the frame returns. The engine has one graphics card,
  made when the first view is attached (`gpu_ready`: its adapter, its API, and whether it is the
  system's in software), which the view draws with; a device that is lost (`gpu_lost`) is made
  again, a second apart until it can be (`gpu_remaking`), and the view draws again on it. For
  10 s after a card was lost only a card is taken, since a driver that resets brings it back; then
  the software adapter is taken if there is no card, and a card is looked for every 30 s while the
  engine draws in software (`gpu_upgraded` once one is taken). A surface that is lost is made
  again. These are logged on stderr (`view_ready`, `view_stalled`, `view_recovered`,
  `view_device_lost`, `view_surface_lost`, `view_error`, `gpu_error`), with
  `view_attached` and `view_detached`. `view_ready` says the engine's `dpi_awareness`:
  `per_monitor_v2`, or why not, in which case the rectangles will be off.
- **Not tested yet** (phase 0): the border of the window dragged by the user (Windows' modal
  loop of sizing), the window moved to a monitor of another DPI, fullscreen, sleep and resume,
  and a device lost for real (it is lost on purpose in a test).
- **On macOS the view is not available yet**: `view.attach` is answered at once with `-32003`
  and `reason: "unsupported"`, and the desktop must show the preview another way there. A view
  made by the engine cannot be put in Electron's window as on Windows: the handle Electron gives
  is an `NSView *` of Electron's process, and AppKit would have to be served by the engine's
  main thread. Another way (a layer of the engine shown in Electron's window with
  `CALayerHost`, or the pictures handed over as `IOSurface`s) is to be settled before the
  desktop is built for macOS.

| `-32003` `reason` | when |
|---|---|
| `no_view` | `view.place` or `view.show` with no view attached |
| `gone` | the view's window was destroyed with its parent; the view is let go |
| `window` | the window of the view could not be made |
| `gpu` | no graphics card can draw in the window |
| `unsupported` | views are not made on this system (macOS, for now) |
| `failed` | the engine failed to make the view |

A `parent` that is not a window, or not a handle in decimal, is refused with `-32602` and
`data: {"reason": "parent"}`; a rectangle wider or higher than 16384 with `{"reason": "rect"}`.

## The transport

There are two independent transports: the **source monitor** loads one media item, and the
**timeline monitor** loads a timeline and composes its enabled video and subtitle tracks.
Each has its own playhead, loaded item, speed and proxy preference. Every request and state
notification names `monitor: "source" | "timeline"`; protocol 2 requires it. Each draws in its
own view (above), when attached. Starting either monitor waits until the other has stopped
playing, including stopping its sound, before starting the requested monitor.

- **Who sends them.** Any client (`desktop`, `agent`, `cli`); all clients see and move the same
  pair of transports. A `--headless` engine has nothing to show and nowhere to play sound:
  every `transport.*` is answered `-32601`.
- **Answers and what is told.** `transport.load` is answered once the media is open and its frame
  0 is shown (about 100–300 ms; other connections are answered meanwhile), with the state, which
  is also told to everyone **before** the answer. Every other method is answered at once with `{}`
  (or the state, for `transport.state`): the answer is queued before the transport is given the
  command, so what it did is always told **after** the answer, in a `transport.state`
  notification to every connection that said hello. So a page that sends seeks as the playhead
  is dragged is never held up by them.

| method | params | result |
|---|---|---|
| `transport.load` | `{"monitor":"source", "media":"m1"}` or `{"monitor":"timeline", "timeline":"t1"}` | the state |
| `transport.play` | `{"monitor":m, "speed":s}`: a whole number from −8 to 8, not 0; 1 if left out | `{}` |
| `transport.pause` | `{"monitor":m}` | `{}` |
| `transport.seek` | `{"monitor":m, "frame":n, "scrubbing":false}` (`scrubbing` false if left out) | `{}` |
| `transport.step` | `{"monitor":m, "frames":n}` | `{}` |
| `transport.state` | `{"monitor":m}` | the state |
| `transport.proxies` | `{"monitor":m, "on":true \| false}` | `{}` |

Here `m` is `"source"` or `"timeline"`. Missing, null or unknown monitors, unknown parameters,
and fields for the wrong monitor are refused with `-32602`.

**The state** (the result of `transport.load` and `transport.state`, and the params of the
notification `transport.state`):

```json
{"monitor":"source", "media": "m1", "frame": 37, "playing": false, "speed": 1, "exact": true,
 "shown": 37, "last": 99, "sound": "off", "source": "proxy", "proxies": true}
```

- `media`: the media loaded, or `null` (then `frame`, `shown`, `last` and `source` are `null`
  too).
- `frame`: the **playhead**: the frame shown while playing, the frame sought or stepped to
  otherwise. Frames are numbered at the rate of the media (`facts.video.rate`), from 0, its
  first frame, as `media.import` describes; for sound alone, see below.
- `playing`, and `speed`: the speed it plays at, or played at last (1 at first).
- `exact`: whether the picture shown is the frame at the playhead. False while scrubbing, when
  a key frame stands in for it (below), and when the frame sought could not be decoded (the
  picture shown before stays; the engine logs `transport_error`).
- `shown`: the frame whose picture is shown (the key frame standing in, while scrubbing);
  `null` for sound alone.
- `last`: the last frame: from the duration of the media (`round(duration × rate) − 1`), until
  the engine decodes to the end and finds where it truly is.
- `sound`: `"none"` (the media has no sound), `"on"` (played, and the clock of the picture),
  `"off"` (not played now: paused, or a speed other than 1), or `"unavailable"` (there is no
  output device, or it failed: the picture plays alone, by the clock of the system; the engine
  logs `sound_unavailable` or `sound_failed`). A device fails when it says so (unplugged, turned
  off), when it asks for no sound within 750 ms of being started (a Bluetooth headset takes some
  300 ms to wake), when it stops asking for 500 ms once its sound has started to be heard, or
  when it goes on asking and lets no sound be heard within 2 s of being started: playing goes on
  at once from where it is, by the clock of the system, with `sound: "unavailable"` (logged
  `transport_sound_lost`). The device is opened anew at the next `transport.load` or
  `transport.play` that plays sound, and so is the default device of the system when it is
  another than the one opened (a headset plugged in, or chosen in the settings of the system),
  so that sound is `"on"` again as soon as it can be.
- `source`: where the picture is read from: `"proxy"`, the proxy of the media (see Proxies), or
  `"original"`, the media itself (as is sound alone).
- `proxies`: whether proxies are used (`transport.proxies`; `true` at first).

**The timeline state** has `monitor:"timeline"` and `timeline` in place of `media`, plus `late`:

```json
{"monitor":"timeline", "timeline":"t1", "frame":37, "playing":false, "speed":1,
 "exact":true, "shown":37, "last":249, "late":0, "sound":"off",
 "source":"mixed", "proxies":true}
```

- Timeline positions use the timeline's frame rate; `last` is `max(duration - 1, 0)`.
  An empty timeline shows black at frame 0. Removing the loaded timeline unloads it and clears
  its view. A missing timeline is refused with `-32602 {reason:"missing", timeline:"..."}`.
- `shown` identifies the last composed timeline frame; individual layers may stand in with a
  key frame while scrubbing, or their track's previous picture when decoding is late. `exact`
  is true only when every media layer is exact. `late` counts playback frames for which at
  least one layer was unavailable, once per frame, and resets when playback starts from pause.
- `source` is `"original"`, `"proxy"`, or `"mixed"` when the composition contains both. A frame
  of only titles/colours or black uses `"original"`. Proxy switches and repairs invalidate
  read-ahead and take effect on the next composition, immediately when paused.
- Edits and undo/redo redraw the paused frame. During playback the next frame uses the new
  project without stopping; a shortened duration clamps the playhead. Old decode results
  cannot be published into a new project generation.
- Each enabled video track reads ahead independently, including upcoming cut/transition
  handles. The shared decoder pool allows two decoders per media/source kind and twelve total.
  Only unheld composition targets are reused, so a view never samples a target being rewritten.
- At playback speed 1, enabled audio tracks are mixed at the timeline sample rate in stereo,
  then converted to the device's rate and channel layout. Disabled tracks/clips are not decoded.
  Each clip applies its gain, linear fades and equal-power crossfades sample by sample; centred
  stereo pan preserves level, endpoints silence the opposite channel, and combined tracks are
  limited to [-1, 1]. Clip speed resampling changes pitch, with a low-pass filter for speed-up;
  reverse clips read backward. Pitch preservation belongs to a later audio phase.
- The device's played samples (including its latency) drive the picture clock. Pausing and
  monitor handover stop that output. Direct seek preserves playback and starts the mix at the
  new position; edits replace the queued old mix at the current playhead. A failed device falls
  back to the monotonic clock from the last heard position and is retried on the next play.
  Shuttle playback speeds other than 1 are silent, as in the source monitor. No enabled audio
  clips reports `"none"`; paused/shuttling audio reports `"off"`.

**Test hook.** Only with `--test-hooks`, `transport.frame_digest {"monitor":m}` reads the current
rendered frame and returns `{monitor, frame, width, height, hash, pixels}`. `hash` is SHA-256 of
the entire RGBA frame, for regression comparisons; `pixels` contains the four top-left RGBA
pixels. It is not a playback operation or project identity. Without the flag, or headless,
the method returns `-32601`; before a picture is loaded it returns `-32004` `not_loaded`.
While timeline sound is playing, the hook also returns `audio_samples` (device-rate samples
heard since that output started) and `audio_frame` (the corresponding timeline clock frame).
These fields are absent after pause and are only for audio/picture regression checks.

The media-specific details below describe the source monitor. Seek, pause, step, speed bounds,
scrub settling and end-of-playback rules also apply to the timeline monitor.

**When it is told.** The notification `transport.state` is sent to every connection that said
hello: at once after each method that does something (even one that changes nothing, so that its
sender hears back), when playing stops (at an end, or paused), when a scrub settles, and when the
media is unloaded; while playing, as the playhead moves, **at most 30 times a second**.

- **`transport.load`** loads the media in place of the one loaded (which stops playing) and shows
  its frame 0, paused. The picture is decoded on the graphics card where it can, else on the
  processor. Media that is not in the project is refused (`-32602`, `data: {"reason": "missing",
  "media": "m9"}`); a file that cannot be opened or decoded is refused as `media.import` refuses
  it (`-32002`, `data: {"path", "reason", "message"}`), and then nothing is loaded (the state says
  `media: null`). When the media loaded, or being loaded, is **removed from the project** (by a
  transaction, an undo or a redo), it is unloaded (once its load is done), and the state is told
  with `media: null`; the view is cleared. While a load is being done, the other methods are
  taken, and done after it (they are not refused as `not_loaded`).
- **Sound alone** (music, a recording) plays as media with a picture does, with no picture: the
  view is black, and `shown` is `null`. Its frames are on a grid of the rate of the **first
  timeline of the project** when it is loaded, or 25 a second if the project has none, from
  frame 0 at the start of its sound to `last` (from its duration). At speed 1 its sound is the
  clock; at the other speeds and backward the playhead moves in silence.
- **`transport.play`** plays from the playhead, or, if it plays already, changes speed from where
  it is (the playhead does not jump). Negative speeds play backward; at speed 1 the sound is
  played, at every other speed (2, 4, 8, and backward) none. It stops at the last frame playing
  forward and at frame 0 playing backward, and the state is told (`playing: false`); playing
  forward from the last frame (or backward from frame 0) stops at once. At 2×, 4× and 8× frames
  are decoded as fast as they can be and those that come too late are passed over (on media of
  long groups of frames, the transport goes on from a key frame when it falls far behind).
  Backward, each frame is sought: on a proxy, and on media whose every frame is a key frame,
  every frame is shown (60 a second of a proxy of 4K at 60); on media of long groups of frames
  read without a proxy, only some are decoded in time, and those are shown.
- **The clock.** At speed 1 with sound, the sound that the device has played (its latency
  counted) is the clock, so that the picture follows what is heard; otherwise the monotonic
  clock of the system. The frame shown at a time is the frame that the clock is at, rounded
  down: `floor(t × rate × speed)` frames from where playing started (playing forward from frame
  `n` starts at the start of `n`, backward at its end, so that `n` is shown for a whole frame
  either way). A frame is never shown before its time; one that comes too late is not shown.
- **`transport.pause`** stops where it is: the playhead is the frame shown, and the sound stops at
  once.
- **`transport.seek`** shows `frame`, clamped to the media (0 to `last`). While playing, it plays
  on from there (unless `scrubbing`). Seeks that come while the engine decodes are taken
  together: only the last is decoded.
- **Scrubbing** (`"scrubbing": true`, while the playhead is dragged; it pauses playing). Send a
  seek at most every 1/30 s while dragging. The engine shows at once the **key frame** at or
  before the frame (`exact: false`, `shown` the key frame; a key frame already shown is not
  decoded again), and once no newer seek has come for **150 ms**, the frame itself (`exact:
  true`). A frame of media whose key frames are not known (MPEG-TS and MPEG-PS where it has not
  been decoded yet) is decoded at once, and so is every frame of a **proxy**, every one of which
  is a key frame: then the first state told of each seek is `exact: true` (some 3–15 ms after the
  seek). Showing a frame that is not a key frame takes up to a few hundred milliseconds on long
  groups of frames (4K HEVC from a camera read without a proxy).
- **`transport.step`** pauses, then moves exactly `frames` frames (negative is back), clamped to
  the media, and shows that frame. Steps sent faster than they are done are added up.
- **Proxies.** Where the media has a proxy (see Proxies) and proxies are used, its picture is read
  from the proxy (`source: "proxy"`), and its sound from the media; the frames, `last` and the
  clock are the media's. **`transport.proxies`** turns their use off (`{"on": false}`: the media
  itself is read, as DaVinci's "Use proxy media" unticked) and on again, for every client, and is
  told in `proxies`. It is taken whether media is loaded or not. A proxy whose file is gone, or
  cannot be opened or decoded, is let go for the media itself (logged `transport_proxy_failed`,
  with `reason: "decode"` for one that could not be decoded), and made again. When the source changes while media is loaded (proxies turned on or off, a proxy made
  or made again while its media is loaded, or one set or taken away by a transaction), the
  picture is read from the new source **at once while it does not play**, and while it plays as
  soon as it stops or is sought: the frame shown stays, and what is decoded next is decoded from
  the new source (told with `source`).
- **The view.** The picture is drawn in the view if one is attached, turned as the media says;
  a view attached later is drawn the picture shown. Without a view the transport plays all the
  same (the state is told as ever).
- **Logs** (stderr): `transport_loaded` (`decoder`: `cuda`, `d3d11va`, `videotoolbox` or
  `software`, and the devices that failed before it, or `null` and the `grid` for sound alone),
  `transport_played` (each time playing stops: how many frames were shown, passed over or
  decoded too late, how late they were shown against the clock of the transport, how far the
  clock of the sound went from the system's, and the latency of the device and the length of
  its runs), `transport_unloaded`, `transport_error`, `transport_device_failed`,
  `transport_sound_lost`, `transport_failed`, `transport_source`, `transport_proxy_failed`,
  `sound_ready`, `sound_unavailable`, `sound_failed`. Of proxies: `proxy_ffmpeg` (the `ffmpeg`
  found, or `null`), `proxy_queued`, `proxy_started`, `proxy_encoding` (the `pid` of each
  `ffmpeg`), `proxy_made` (its `encoder`, size, frames and seconds; `encoder: null` for the proxy
  of other media of the same file), `proxy_recipe_failed` (an encoder that failed, before the next
  is tried), `proxy_failed`, `proxy_skipped` (media that could not be read to tell whether it
  needs one, or whose proxy failed before), `proxy_deleted` (a file of the cache that no media
  names, as the engine starts), `proxy_dropped` (a proxy that is not the engine's own, taken
  away as the project is read), `proxy_not_deleted` (a file that was not deleted, not being one
  of the proxies of the project).

| `-32004` `reason` | when |
|---|---|
| `not_loaded` | `transport.play`, `.pause`, `.seek` or `.step` with no media loaded, and none being loaded |
| `failed` | the transport of the engine has stopped (a bug of the engine): it told the state with `media: null` as it stopped, and logged `transport_failed`; from then on every `transport.*` is refused so, but `transport.state`, which still answers with that state |

## Operations

An operation is an object that names itself in `op` (unknown fields are refused, a missing
`index` means "at the end"); the project and its parts are as in `project.json`
(`crates/model/src/project.rs`):

| op | fields |
|---|---|
| `add_media` | `media`, `index?` |
| `remove_media` | `id` |
| `add_timeline` | `timeline` (with its tracks and clips), `index?` |
| `remove_timeline` | `id` |
| `add_track` | `timeline`, `track` (with its clips), `index?` |
| `remove_track` | `timeline`, `id` |
| `insert_clip` | `timeline`, `track`, `clip` |
| `remove_clip` | `timeline`, `id` |
| `move_clip` | `timeline`, `id`, `track`, `start` |
| `trim_clip` | `timeline`, `id`, `source_in?` (leave source in unchanged if absent), `start`, `length` |
| `shift_clips` | `timeline`, `track`, `clips: [id, ...]`, `by` |
| `shift_cues` | `timeline`, `track`, `cues: [id, ...]`, `by` |
| `set_proxy` | `media`, `proxy` (a proxy, or `null` to take it away; its inverse sets the one before). A proxy must be a file of the engine's own (see Security), else it is refused as `not_a_proxy_path`; so must one on media that `add_media` adds |
| `set_timeline` | `id`, `name?`, `rate?`, `width?`, `height?`, `sample_rate?` |
| `set_track` | `timeline`, `id`, `name?`, `enabled?`, `locked?`, `language?` |
| `set_clip` | `timeline`, `id`, `enabled?`, `speed?`, `reverse?`, `link?`, `transform?`, `gain_db?`, `pan?`, `fade_in?`, `fade_out?`, `source?` |
| `add_bin` | `bin`, `index?` |
| `remove_bin` | `id` |
| `set_bin` | `id`, `name?`, `parent?` |
| `set_media` | `id`, `name?`, `bin?`, `mark_in?`, `mark_out?`, `still?` |
| `add_transition` | `timeline`, `track`, `transition` |
| `remove_transition` | `timeline`, `id` |
| `set_transition` | `timeline`, `id`, `kind?`, `length?`, `align?` |
| `add_cue` | `timeline`, `track`, `cue` |
| `remove_cue` | `timeline`, `id` |
| `set_cue` | `timeline`, `id`, `start?`, `length?`, `text?` |

Optional settings are unchanged when omitted. Nullable settings (`parent`, `bin`, marks and
`link`) use explicit `null` to clear them. A `set_clip.source` may change title text/style or
color, but not its source kind or media; use the `replace` edit for media replacement.
`set_media.still` refreshes an existing import's image classification from `media.inspect`.
It requires picture without audio for `true`; changing it revalidates existing clips and
transitions before committing. Its inverse restores the old classification. This lets old
media-pool images gain unlimited still duration without new ids, together with migration.
The settings of `set_media` are checked in a fixed order: `bin`, the marks, then `still`
(`no_stream` without picture, `out_of_range` `still` with sound, then the clips of the media).
A `set_media` that sets every given setting to the value it has already changes nothing: alone
in a transaction it is refused as `empty` and makes no step.

`shift_clips` moves each distinct selected clip by `by` frames on one track, preserving its
source, length and settings, and every transition whose two ends move together. A transition
with only one end selected must be removed first. The operation refuses invalid positions,
missing ids or overlap atomically. Its inverse moves the same ids by `-by`. `shift_cues` does
the same for subtitles of one subtitle track, refusing `cue_overlap`, `out_of_range` `start`
(before frame 0) or `wrong_track` for a track of picture or sound; the edits move every later
subtitle of a track with one `shift_cues`, however many there are. Both refuse an empty list
(`out_of_range` `clips` / `cues`) and `by: 0` (`out_of_range` `by`), which would make a step that
changes nothing.

Basic operations can change locked tracks, as undo/redo must. Interactive clients should send
edits for clip manipulation; those implement locks, links, clamping and ripple rules.

A thing added with an empty `id` is given one by the engine (`m`/`t`/`c`/`b`/`x`/`q`/`g` + a number from
`next_id`, never reused; tracks `V<n>`/`A<n>`/`S<n>` within their timeline). An id that starts with `$` is never
kept (see names below). The rules are in `crates/model/src/ops.rs`.

**`done`** lists the operations as they were applied, each with every id the engine gave and the
place each thing was put at (the `index` of an added thing is filled in): applied in order to a
copy of the project at revision `n - 1` whose `next_id` was first set to the `next_id` sent with
them, they give the project at revision `n`. Without that `next_id`, a copy would refuse the
engine's new ids as `reserved`.

## Edits

An edit uses `edit` instead of `op`; sending both or an unknown field is refused. Every command
below also requires `timeline`. `?` marks optional fields. Where present, `linked` defaults to
true and includes unlocked members of the selected clips' link groups. Naming a locked track
or clip directly is refused; implicitly linked members on locked tracks are left untouched.

A placement `source` is `{kind:"media", media, from?, to?, length?}` (from/to are media ticks),
`{kind:"color", color, length}`, or `{kind:"title", text, style, length}`. Media defaults to its
marks or full duration; `length` overrides the span's timeline duration. `video_track` and
`audio_track` omitted select the first track of that kind, explicit `null` suppresses it, and an
id targets that track. Both picture and sound placed together get a new link group.

| edit | fields besides timeline | behavior |
|---|---|---|
| `insert` | `source`, `at`, `video_track?`, `audio_track?` | Cut every unlocked track at the insertion point and ripple later content by the source length. |
| `overwrite` | same | Replace the covered range on target tracks, without rippling other tracks. |
| `replace` | same | Replace the source of the clips at `at`, keeping their positions, lengths and settings; explicit source `from` aligns to `at`. |
| `place_on_top` | `source`, `at` | Use an unlocked track above occupied tracks, creating one when needed. |
| `append` | `source`, `video_track?`, `audio_track?` | Place after the latest clip end on the target tracks. |
| `smart_insert` | `source`, `at`, `video_track?`, `audio_track?` | Insert at the nearest unlocked audio/video cut; ties choose the earlier cut. |
| `ripple_overwrite` | same | Replace the clip range at `at` and ripple by the difference in lengths. |
| `lift` | `clips`, `linked?` | Remove selected clips and leave their gaps. |
| `ripple_delete` | `clips`, `linked?` | Remove selected clips and close their occupied ranges on unlocked tracks. |
| `ripple_delete_range` | `from`, `to`, `tracks?` | Cut out `[from,to)` and close it, on all unlocked tracks or the given tracks. |
| `close_gap` | `track`, `at` | Close the gap at `at` and ripple unlocked tracks. |
| `blade` | `at`, `tracks?`, `linked?` | Split crossing clips; original ids stay with first halves, second halves get new ids and link groups. |
| `move` | `clips`, `delta`, `to_track?`, `overwrite?`, `linked?` | Move clips; default refuses occupied destinations, `overwrite:true` clears them. |
| `insert_move` | `clips`, `at`, `linked?` | Lift and close original ranges, then insert at the original timeline coordinate. Noncontiguous selections pack their occupied interval union, not the gaps between it. Subtitles on unlocked tracks within a moved range go with it, keeping their ids and their place on the clips; one across an end of a range is cut there, and only its part inside goes along. |
| `trim` | `clip`, `edge:in|out`, `delta`, `ripple?`, `linked?` | Move one edge; ripple defaults false. |
| `roll` | `left`, `right`, `delta`, `linked?` | Move a shared cut, keeping outer endpoints. |
| `slip` | `clip`, `delta`, `linked?` | Change source in/out while keeping timeline position and length. |
| `slide` | `clip`, `delta`, `linked?` | Move the clip while adjusting neighbors, keeping its source and length. A final clip may slide and change total duration. |
| `add_transition` | `left`, `right`, `length`, `kind?`, `align?` | Default dissolve on video or crossfade on audio, centered. |
| `remove_transition` | `id` | Remove the transition. |
| `set_transition` | `id`, `kind?`, `length?`, `align?` | Change given transition settings. |
| `set_speed` | `clip`, `speed`, `reverse?`, `ripple?`, `linked?` | Preserve source span; optional ripple uses the linked group's change in maximum end, including J/L cuts. |
| `add_cue` | `track`, `start`, `length`, `text` | Add one subtitle. |
| `set_cue` | `id`, `start?`, `length?`, `text?` | Change given subtitle settings. |
| `remove_cue` | `id` | Remove one subtitle. |
| `cues_from` | `track`, `items:[{start,length,text},...]` | Replace all subtitles on that track. |
| `add_track` | `kind`, `index?`, `name?`, `language?` | Add an empty track; index is among tracks of its kind; name is its display label. |
| `remove_track` | `track` | Remove the track and its contents, all restored by undo. |

Ripples preserve later content's relative timing on unlocked tracks. An unselected clip inside
a range to be closed causes `ripple_blocked`; it is not silently deleted. Subtitles inside a
closed range are trimmed or removed, and later subtitles follow the picture. A ripple that
moves later content later (an insertion, a ripple overwrite with a longer source, a ripple trim
or a change of speed that lengthens) splits a subtitle across its point into two cues with the
same text, with the new frames between them; the first keeps the id. Locked subtitle tracks
remain unchanged. A ripple trim or `set_speed` ripple of a split edit (J/L cut) is refused as
`ripple_blocked` when a clip starts between the end of one of the group's clips and the end of
the whole group on that clip's track: it would stay while what follows moves, parting the
picture of the next group from its sound.

Trims, rolls, slips and slides clamp to available handles and space, reporting their input
index in `clamped`; zero delta or an impossible movement is refused. Editing through a
transition keeps it where its clips still meet, shortens it to available handles/coverage, or
removes it if none fits. A repair shortened to zero removes the transition and is also reported as clamped. Transitions and same-end
fades are not silently substituted. All changes are still one reversible transaction.

## Names of a transaction (`$name`)

Within one `transaction.apply`, a thing that an operation adds may be given an id of the form
`$name`: `$` and 1–32 of `A–Z a–z 0–9 _`. The thing is added as if its id were empty, and given
one by the engine. Every **later** operation of the **same** transaction that names `$name` where
it names a thing (its `timeline`, `track` or `id`, the `media` of a clip, and the `media` of
`set_proxy`) is made to name that
id before it is applied. Edit references use the same rules. The ids are answered in `names`; `done` and `project.changed` carry only
real ids. For example:

```json
{"ops": [
  {"op": "add_timeline", "timeline": {"id": "$t", "name": "Cut",
    "rate": {"num": 25, "den": 1}, "width": 1920, "height": 1080, "sample_rate": 48000, "tracks": []}},
  {"op": "add_track", "timeline": "$t", "track": {"id": "$v", "kind": "video",
    "name": "Picture", "enabled": true, "locked": false, "clips": [], "transitions": []}},
  {"edit": "insert", "timeline": "$t", "at": 0, "video_track": "$v",
    "source": {"kind": "color", "color": "#000000ff", "length": 50}, "name": "$c"},
  {"edit": "trim", "timeline": "$t", "clip": "$c", "edge": "out", "delta": -10}
]}
```
→ in a new project, `"names": {"$c": "c2", "$t": "t1", "$v": "V1"}`.

- Names may bind media, bins, timelines, tracks, clips, transitions and cues, including those
  inside an added timeline or track. A name can be referenced only by a **later input entry**,
  not elsewhere inside the same added object.
- A clip's `link` (including `set_clip.link`) may declare a new `$group` on its first use. Later
  input entries use it to join that group. An empty link also creates a group but does not name
  it; `null` unlinks. Media bin references, transition endpoints and all edit id fields support
  names; display names, title text and subtitle text remain literal, even if they start with `$`.
- Creating edits `insert`, `overwrite`, `append`, `smart_insert`, `ripple_overwrite`,
  `place_on_top`, `add_transition` and `add_cue` accept `name:"$result"`. It binds the intended
  new clip (picture first, otherwise sound), transition or cue; never a fragment split from
  existing content or a restored transition.
- To address several outputs, use `name:{"video":"$v","audio":"$a","link":"$g"}`.
  Selectors are `primary`, `clip`, `video`, `audio`, `link`, `transition`, `cue`; every requested
  output must exist. A missing output refuses the **whole** transaction with `-32001`,
  `data:{reason:"missing",what:"named output",id:"$a",index:i,message:...}`, preserving the
  project, revision, id counter and undo/redo. Empty name objects are refused.
- `replace` retains ids; batch splits (`blade`, `cues_from`) expose their resulting basic
  operations in `done` rather than a single named output. `add_track.name` remains its display
  label; to name a new track within a transaction use basic `add_track` with `track.id:"$v"`.
  Names live only for the current transaction and are never written into the project.
- The id of a track is unique only within its timeline, so **a track's name is named only
  together with its timeline**, written as the operation that gave the name wrote it (the same
  `$name` or the same id). A track named in a timeline added without an id cannot be named
  later in the transaction.
- An id that starts with `$` is always a name. These are refused, as `-32602` with
  `data: {"index": i, "name": "$x", "reason": "..."}`, and nothing is applied:

  | reason | meaning |
  |---|---|
  | `invalid_name` | not of the form `$` + 1–32 of `A–Z a–z 0–9 _` |
  | `unknown_name` | named before (or without) an operation that gives it |
  | `duplicate_name` | given twice |
  | `name_elsewhere` | a track's name used with another timeline than the one it was given in |

## Notifications

| method | params |
|---|---|
| `project.changed` | `{"revision": n, "by": "<kind>", "made_by": "<kind>", "done": [op, ...], "next_id": k}` |
| `engine.error` | `{"message": "...", "kind": "save"}` |
| `transport.state` | the state of the transport (see The transport) |
| `engine.yielding` | `{"to": "desktop"}`: the engine is about to end, to yield the project to the desktop's engine (see `engine.yield`) |
| `media.proxy` | `{"media": "m1", "state": "queued" \| "running" \| "done" \| "failed", "progress": 0.5, "message"?: "..."}` (see Proxies) |
| `media.waveform` | `{"media": "m1"}` (background analysis finished; query again for data or error) |

- `by` is the kind of the client that asked (for an undo or a redo, the one that asked for
  that); `made_by` is the kind of the client that made the transaction (for an undo or a redo,
  the one that made the step taken back or done again). Both are `"engine"` for a change that
  the engine made itself (a proxy set on its media), which is no step of the history.
- **Keeping a copy.** Call `project.get` after `hello`; then for each `project.changed`: ignore
  it if `revision` is not above yours; if it is yours + 1, set your `next_id` to its `next_id`
  and apply `done` in order; if it skips a revision (which the engine never does on one
  connection), call `project.get` again.
- `engine.error` with `kind: "save"`: the project could not be saved; the engine keeps it and
  tries again, more slowly while it keeps failing (at least every 32 s).

## Security

`project.json` lives in the user's repository (`.columbina/studio`), and may come from anyone who
commits to it; and any client may send transactions. So:

- **Proxies are the engine's own.** The engine reads the file of a proxy, and deletes it when it
  cannot be decoded, when it makes another in its place, and when no media names it as the
  engine starts. A proxy is therefore only ever a file `<id>-<12 hex digits>.mov` (`<id>` of
  `A–Z a–z 0–9 - _`, the id of the media it was made for, or of other media of the same file)
  directly in `<project>/cache/proxies`, compared as the system finds the paths: no `..`, and
  neither `cache` nor `cache/proxies` a link or a junction to elsewhere. `set_proxy` (or
  `add_media`) with another path is refused (`not_a_proxy_path`); a `project.json` that names one
  is read without it (logged `proxy_dropped`). Before each deletion the same is checked again,
  and only a file (not a link) is deleted; anything else is left as it is (logged
  `proxy_not_deleted`).
- **Media paths are the user's sources**, and may be anywhere: `add_media` takes any absolute
  path, and the engine reads (never writes or deletes) the file there when it is loaded,
  scrubbed or made a proxy of. A `project.json` from elsewhere may name a path on another
  computer (`\\server\share\…` on Windows), which the system may sign in to as the media is
  read. This is left to the desktop (which does not let its page send `add_media`) and to a
  later decision.

## Errors

| code | when | `data` |
|---|---|---|
| `-32700` | the line is not JSON (id `null`) | |
| `-32600` | not a request object, `jsonrpc` is not `"2.0"`, no `method`, a bad `id` (answered with id `null`), a request before `hello`, a second `hello`, a line over 16 MiB, `view.*` or `engine.yield` from a client that is not the desktop | |
| `-32601` | no such method; `view.*` and `transport.*` in a `--headless` engine; `media.make_proxy` in an engine that makes no proxies; `engine.yield` in an engine that is not headless | |
| `-32602` | params are not what the method takes (or not an object); an operation that cannot be read; a bad `$name`; a speed of `transport.play` out of −8…8 or 0; `transport.load` of media not in the project | `{"index": i, "reason": "invalid_op"}` for an operation that cannot be read; see names above; `{"reason": "speed"}`; `{"reason": "missing", "media": "..."}` |
| `-32001` | a transaction, undo or redo was refused; the project is exactly as it was | the refusal, below |
| `-32002` | `media.import`: a file cannot be read; nothing was added. `transport.load`: the file of the media cannot be opened or decoded | `{"path": "<as sent>", "reason": "...", "message": "..."}`, below |
| `-32003` | `view.*`: the view cannot be made, there is none, or its window is gone | `{"reason": "..."}`, see The view |
| `-32004` | `transport.*`: no media is loaded | `{"reason": "..."}`, see The transport |

The connection stays usable after every error.

**Files that cannot be read (`-32002`).** `reason` is one of `not_found` (no such file),
`unreadable` (it may not be read, or it is a directory), `not_utf8` (its path is not UTF-8 or
holds a NUL), `not_media` (libav cannot open it as media of the formats above), `no_streams`
(neither picture nor sound), `unsupported` (what it holds cannot be used: a picture that libav
cannot decode, no duration, a picture with no size or rate), `read` (it cannot be read on, once
opened), or `failed` (the engine failed to read it). `transport.load` gives these too, and
`decode` (its first frame cannot be decoded) and `seek`.

**Refusals (`-32001`).** `data` is `{"reason": "...", "message": "...", ...fields}`, with
`"index": i` (the place of the refused operation in `ops`) when an operation was refused, and
`{"reason": "empty", "message": ...}` for a transaction with no operations. An undo or a redo is
refused only through a bug of the engine. Each `reason` and its fields:

| reason | fields | meaning |
|---|---|---|
| `missing` | `what` (`media`, `bin`, `timeline`, `track`, `clip`, `transition`, `cue`, `named output`), `id` | the thing named is not there |
| `taken` | `what`, `id` | the id of an added thing is held by another of its kind |
| `reserved` | `id` | an id of the engine's own form (`m`/`t`/`c`/`b`/`x`/`q`/`g` + number) not made yet |
| `dollar` | `id` | an id that starts with `$` (only names of a transaction do; the engine replaces or refuses them before this) |
| `out_of_list` | `at`, `len` | an `index` past the end of its list |
| `no_stream` | `media`, `kind` (`video`, `audio`) | the media has no stream for the track |
| `wrong_kind` | `track`, `kind` | a clip moved to a track of the other kind |
| `out_of_source` | `media`, `why` (`length`, `before_start`, `past_end`) | the clip lasts no frame, or shows what is not in its media |
| `scale` | `scale`, `media_scale` | a time of scale 0, or not of the scale of its media |
| `overlap` | `with` | the clip covers frames that clip `with` covers |
| `out_of_timeline` | `start` | the clip starts before frame 0 |
| `in_use` | `id`, `by` | media still used by clip `by` |
| `too_large` | `what` | an integer further than 2^53 − 1 from 0 |
| `zero_rate` | | a rate with a 0 in it |
| `negative_duration` | `ticks` | media lasting less than no time |
| `ids_exhausted` | | the engine has made every id it can |
| `no_id` | | (only for a project that is read, never from an operation) |
| `not_absolute` | `path` | the `path` of a proxy is not absolute (`C:\…`, `\\server\…` or `/…`) |
| `not_a_proxy_path` | `path` | the `path` of a proxy is not a file of the engine's own: `<id>-<12 hex digits>.mov` directly in `<project>/cache/proxies` (see Security) |
| `empty` | | the transaction has no operations |

### Additional schema 2 and edit refusals

| reason | fields | meaning |
|---|---|---|
| `locked` | `track` | an edit directly names content on a locked track |
| `ripple_blocked` | `track`, `clip` | an unselected clip occupies frames a ripple would close |
| `no_room` | `track` | there is no usable space for the edit |
| `no_handles` | `clip`, `need`, `have`, `longest?` | insufficient source/coverage; `longest` is the maximum possible transition length |
| `empty_range` | | zero-length range/delta or an edit that cannot move |
| `nothing_at` | `track` (nullable), `frame` | no target clip at the requested frame |
| `no_track` | `kind` | no track for the requested source |
| `wrong_track` | `what`, `kind` | content or settings on an incompatible track kind |
| `cue_overlap` | `with` | subtitles overlap on one track |
| `out_of_range` | `what` | invalid setting such as speed, gain, transform or text length; `digest` for media whose `identity.digest` is not 64 lowercase hex digits; `by`, `clips` or `cues` for a shift that would change nothing |
| `has_transition` | `transition` | a basic operation would move just one transition endpoint |
| `not_adjacent` | `left`, `right` | transition endpoints do not meet |
| `cut_taken` | `with` | a transition is already at this cut |
| `too_long` | `transition`, `clip` | transitions cover more than the available clip length |
| `source` | `clip` | basic set_clip cannot change this source kind/media |
| `cycle` | `bin` | cyclic bin parenting |
| `fade_at_transition` | `clip`, `edge` | fade and transition at the same clip end |
| `not_empty` | `bin`, `holds` | removing a bin that still holds content |

As with basic operations, edit refusal indexes refer to the original mixed `ops` array, not
the expanded operation list. Refusal emits no `project.changed` and adds no history step.

## Rendering a timeline

`render.start` is available to every greeted client, including a headless engine:

```json
{"timeline":"t1","path":"C:/Exports/film.mp4","codec":"h264","quality":"high","range":{"from":0,"to":250},"subtitles":true,"replace":false}
```

`codec` is `h264`, `hevc`, or `prores`; `quality` is `high`, `medium`, or `small`.
`range` is optional and half-open; when omitted, it covers the full timeline. It must be
nonempty and inside the timeline. The first exported frame becomes frame zero in the file.
`subtitles` and `replace` default to false. Unknown fields are refused.

The path must be absolute, its parent directory must exist, and its extension must be `.mp4`
(H.264/HEVC) or `.mov` (any codec). Export cannot write inside the project's cache, including
through a directory link, or replace a directory or symbolic link. An existing destination or
SRT with `replace: false` is refused with `-32001` and `{reason:"exists",path}`. Invalid path,
codec, quality, timeline or range is `-32602`. One export runs at a time; a second start is
refused with `-32001` and `{reason:"busy"}`.

The answer is `{job:"render-1"}` and is queued before that job's first `render.progress`:

```json
{"job":"render-1","state":"running","frame":36,"frames":250,"fps":48.2,"path":"C:/Exports/film.mp4","encoder":"nvenc"}
```

`frame` counts frames completed, not absolute timeline frames; `frames` is the total. Running
notifications are sent at most once per 250 ms. The terminal notification is always sent,
with `state` equal to `done`, `failed`, or `cancelled`. `encoder` is present once opened;
`message` explains failure or a codec fallback. `path` reports the actual destination.
The project revision and undo history are not changed by an export.

The export uses a snapshot taken at start, independent original-media decoders, the same
scene compositor as preview, and the same stereo timeline mixer. It never uses proxies.
Edits, undo, or deletion of that timeline after start do not change the export. Titles are
composited into the picture. Subtitle tracks are **not burned into the picture**.
The GPU is shared with preview: a waiting preview composition takes the next turn; export
submits and reads back one frame at a time. Audio is AAC, 192 kb/s, 48 kHz stereo. H.264 uses
8-bit picture; HEVC uses 10 bits when the project contains 10-bit media. If no hardware
encoder can open H.264/HEVC, the engine tries ProRes in a `.mov`, reports that change, and
checks the new destination against the same overwrite rules.

With `subtitles: true`, each enabled subtitle track gets an SRT: one track uses `film.srt`;
multiple tracks use `film.<language-or-track-name>.srt`. Unsafe filename characters are
replaced and repeated names get numbered suffixes, without hashes. Cue times are clipped to
the export range and rebased to zero; frame times use the exact rational frame rate, rounded
down to milliseconds. Text is UTF-8, with normalized newlines and escaped `&`, `<`, and `>`.
Disabled tracks are omitted; an enabled track with no cue in the range produces an empty SRT.

`render.cancel {job}` answers `{}` and requests cancellation; wait for the terminal progress
notification before assuming its files have been cleaned up. An unknown job is `-32602`.
Cancellation of the most recently completed job is harmless. Engine shutdown cancels and
joins its export before releasing the project.

The movie and SRTs are first written beside their destinations as `<path>.part`. Existing
partial files are not overwritten. On cancellation or failure these owned partials are
removed. At publication, sidecars are renamed first and the movie last; previous destinations
are retained in temporary backups until all renames succeed, then removed. A publication
failure rolls back previously renamed files. If the filesystem prevents restoring a backup,
the old file is retained and its recovery path is logged as `render_restore_failed`.
