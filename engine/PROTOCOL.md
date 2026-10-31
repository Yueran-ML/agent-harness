# The engine protocol (version 2)

`columbina-engine` is the only program that writes a project. The desktop app and the agent
connect to the same running engine, send it changes, and are told of every change in one order.
This is what a client is written against.

## Starting and finding an engine

```
columbina-engine --project <DIR> [--headless] [--idle-seconds <N>] [--no-proxies] [--test-hooks]
```

- `--no-proxies` starts an engine that makes no proxies (`media.make_proxy` is then `-32601`).
  `--test-hooks` opens the test hook of the transport (`transport.frame_digest`, below); it is
  for tests only. The engine takes a few more options for its own tests (`--proxy-ffmpeg
  <absolute file>`, which comes before `COLUMBINA_PROXY_FFMPEG`, `--mute`, `--sound`,
  `--queue-capacity`, `--queue-bytes`, `--read-gate`, `--fail-card-at`, `--proxy-readrate`,
  `--proxy-encoder`, `--proxy-watchdog`); clients do not use them.

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
  a well-formed one. One that is not well formed (`jsonrpc` not `"2.0"`, no `method`) is
  answered with `-32600` and id `null`, as a request would be. Batches (arrays) are refused (`-32600`).
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
| `render.start` | `{"timeline":"t1", "path":"<file>", "codec":"h264", "quality":"high", "range"?:{"from":0,"to":250}, "subtitle_tracks"?:["S1"], "replace"?:false, "revision"?:n}` | `{"job": "render-1"}` (see Rendering a timeline) |
| `render.cancel` | `{"job": "render-1"}` | `{}` |
| `transport.frame_digest` | `{"monitor": m}` | test hook only, with `--test-hooks` (see The transport); `-32601` otherwise |
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
  whole import (`-32001`, `{"reason":"missing","what":"bin","id":…,"index":0}`), without
  changing the project or its history. It adds the media to the
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
  `floor(ceil(duration * rate) / 2)`. A number outside the duration is `-32602`, `data:
  {"reason":"out_of_range","frame":n}`; a source without
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
  numbered as those of the media, it has as many frames as the media where the media's index
  says how many (MP4 and QuickTime, not of fragments), and its frames are key frames. A frame of
  the media that comes late (a phone that drops one) leaves a number that no frame has, in the
  media and in the proxy alike, and the frame before it is shown there: such media has fewer
  frames than its duration has numbers. It is written to
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
  "missing"`), and that is checked first; an engine that makes no proxies answers `-32601`
  for media that is there. Deleting `failed.json` forgets
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
project written before proxies has none, and is read as it is. A proxy whose (absolute) `path` is
not of the engine's own is taken away as the project is read (logged `proxy_dropped`), and the
rest of the project is read as it is. So is a proxy whose `path` is not absolute: it is taken
away before the project is checked, so the project is read without it and the engine starts.
(Such a proxy in an operation, `set_proxy` or `add_media`, is still refused as `not_absolute`.)

### Schema 3

The top level is `{schema: 3, next_id, bins: [...], media: [...], timelines: [...], groups: [...]}`.
Schema 3 differs from schema 2 only by the grades (see Grades below). A schema 1 or schema 2
project is upgraded on open; its original bytes are kept once in `project.schema1.json` or
`project.schema2.json`.
Unknown fields are refused. `project.get` and `project.changed` are the source of truth for
clients; a client must not write `project.json` itself.

- A bin is `{id, name, parent}` (`parent: null` for the root). Bins cannot contain themselves,
  and a nonempty bin cannot be removed. Media adds `name`, nullable `bin`, `mark_in` and
  `mark_out`; marks use the media duration's time scale. The range marked is half open:
  `mark_in` is the start of its first frame and `mark_out` the start of the frame after its Out
  frame, so that the Out frame is in it, as DaVinci Resolve's Out point is the last frame of
  the range (Reference Manual 20, pp. 760, 824). A client that marks frame `n` as the Out keeps
  the start of `n + 1`, and shows the Out as frame `n`. Nullable fields (`parent`, `bin`, the
  marks, a clip's `link`) must be written, as `null` if there is none; of media, only `proxy`
  may be left out, and `facts.video` / `facts.audio` may be `null` or left out.
- A timeline is `{id, name, rate: {num, den}, width, height, sample_rate, tracks, grade}`. Width and
  height are even integers, 16–16384; sample rate is 44100, 48000 or 96000. Frames are integers
  at the timeline rate. Video tracks stack in list order, the first at the bottom.
- Every track has `id`, `kind`, `name`, `enabled`, `locked`. Video/audio tracks additionally
  have both `clips` and `transitions`; subtitle tracks instead have both `language` and `cues`.
  Fields of another track kind are refused, even empty arrays. Track ids are unique within a timeline.
  A timeline shows one subtitle track at a time, as DaVinci Resolve does: at most one of its
  subtitle tracks is `enabled` (shown in the timeline monitor). A project with two is refused
  when it is read, and so is an operation that would make two (`subtitle_shown`, below).
- A clip is `{id, start, length, source, enabled, speed, reverse, link, transform, gain_db, pan,
  fade_in, fade_out, grade, group}`. `grade` is the clip's graph and `group` a group id or `null`
  (see Grades). `start >= 0`, `length > 0`; same-track clips cannot overlap.
  `speed: {num, den}` is a positive rational from 1/100 to 100; direction is the separate `reverse`
  flag. `link` is a group id or `null`. Time conversion uses integer ticks, rounded down, so a
  cut need not be an exact sample boundary for arbitrary source time bases.
- A stored `source` is `{kind:"media", media, in:{ticks,scale}}`,
  `{kind:"color", color:"#rrggbbaa"}`, or `{kind:"title", text, style}`.
  Title style is `{font, size, color, x, y, align}`: size 1–4096 in timeline pixels; `x`/`y`
  place the anchor as shares of the picture (0 at the left or top edge, 1 at the right or
  bottom), any finite number (outside 0…1 the anchor is off the picture); align
  `left|center|right` says which side of the text the anchor is on. Title text may be empty and
  is at most 1000 characters.
- Colors accept exactly eight hexadecimal digits after `#`, with either letter case; returned
  operations and saved projects normalize them to lowercase `#rrggbbaa` (alpha last).
- `transform` is `{x,y,scale,rotation,opacity}`: offsets as shares of timeline width/height,
  scale in (0,16], clockwise degrees, opacity in [0,1]. Gain is −96…24 dB; pan is −1…1.
  Fades count timeline frames from the respective ends. A fade and transition at the same end
  are refused; clear the fade explicitly to replace it with a transition.
- A transition is `{id,kind,left,right,length,align}`. Kind is `dissolve|dip_black|dip_white`
  on video or `crossfade` on audio; alignment is `start|center|end`. Its clips must be adjacent
  on one track, have enough source handles and enough length beside their other transitions.
- A subtitle cue is `{id,start,length,text}`, with a nonnegative start and positive length;
  its text is not empty and at most 1000 characters. Cues cannot overlap on the same track. `language` is a short BCP 47 tag or an empty string.

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
| `no_view` | `view.place` or `view.show` with no view attached (`data` also names the `monitor`) |
| `gone` | the view's window was destroyed with its parent; the view is let go |
| `window` | the window of the view could not be made |
| `gpu` | no graphics card can draw in the window |
| `unsupported` | views are not made on this system (macOS, for now) |
| `failed` | the engine failed to make the view |

A `parent` that is not a window, or not a handle in decimal, is refused with `-32602` and
`data: {"reason": "parent"}`; a rectangle wider or higher than 16384 with `{"reason": "rect"}`.

## The transport

There are two independent transports: the **source monitor** loads one media item, and the
**timeline monitor** loads a timeline and composes its enabled video tracks and its shown
(enabled) subtitle track, of which a timeline has at most one.
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
  of only titles/colours or black uses `"original"`. Proxy switches and repairs of media that
  the timeline shows invalidate read-ahead and take effect on the next composition,
  immediately when paused; a proxy of other media changes nothing of the monitor.
- Edits and undo/redo redraw the paused frame. During playback the next frame uses the new
  project without stopping; a shortened duration clamps the playhead. Old decode results
  cannot be published into a new project generation. Only what a change touches starts again:
  a change of another timeline, of bins, of media that the timeline does not use (a proxy the
  engine made of it too) keeps the read-ahead pictures and the sound; a change only of what is
  seen (tracks of picture, the media and proxies under them, size, rate) starts the read-ahead
  again and keeps the sound playing; and the sound starts again at the playhead only when what
  is heard from the playhead to the end (the pieces of clips, their gains, fades, pans and
  speeds, their media, the rates and the duration) is not as it was.
- Each enabled video track reads ahead independently, including upcoming cut/transition
  handles. The shared decoder pool allows twelve decoders, and of one media and source kind
  two for each enabled track of picture that uses it (two at least), so that tracks reading one
  media at once do not take turns with its decoders. At most twelve decoders of the monitor and
  an export together use the graphics card; any more decode on the processor.
  Only unheld composition targets are reused, so a view never samples a target being rewritten.
- At playback speed 1, enabled audio tracks are mixed at the timeline sample rate in stereo,
  then converted to the device's rate and channel layout. Disabled tracks/clips are not decoded.
  Each clip applies its gain, linear fades and equal-power crossfades sample by sample. Pan is
  equal power, `√2·cos θ` on the left and `√2·sin θ` on the right with `θ = (pan + 1)·π/4`, each
  applied to its own channel: centred it preserves level, and on stereo media it is a
  **balance**, not a re-panning of a mono sum, so a hard pan keeps one channel (3 dB louder) and
  drops the other. Combined tracks are limited to [-1, 1]. Clip speed resampling changes pitch,
  with a windowed-sinc low-pass filter for speed-up whose length grows with the speed (16 zero
  crossings each side up to 32 times, 512 samples each side above that), so that what would be
  above the new Nyquist frequency does not fold back; reverse clips read backward. Pitch preservation belongs to a later audio phase.
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
heard since that output started), `audio_frame` (the corresponding timeline clock frame) and
`audio_sample` (where the mixer is in the timeline, in timeline samples: the sample it started
at plus those heard, at the timeline's `sample_rate`), so a test can tell the sound from the
picture by the mixer itself rather than by the clock that the sound drives.
They are given while the timeline's sound is `"on"`; `audio_frame` or `audio_sample` is `null`
while the output has no clock yet or the mixer has not started. These fields are absent after
pause and are only for audio/picture regression checks.

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
  `ffmpeg`), `proxy_made` (its `encoder`, size, `frames` written, `frames_of_media` as the
  media's index counts them or `null`, `numbers_of_media` as its duration has them, and seconds;
  `encoder: null` for the proxy of other media of the same file), `proxy_recipe_failed` (an encoder that failed, before the next
  is tried), `proxy_failed`, `proxy_skipped` (media that could not be read to tell whether it
  needs one, or whose proxy failed before), `proxy_deleted` (a file of the cache that no media
  names, as the engine starts), `proxy_dropped` (a proxy that is not the engine's own, taken
  away as the project is read), `proxy_not_deleted` (a file that was not deleted, not being one
  of the proxies of the project).

| `-32004` `reason` | when |
|---|---|
| `not_loaded` | `transport.play`, `.pause`, `.seek` or `.step` with nothing loaded on that monitor (no media on the source monitor, no timeline on the timeline monitor), and nothing being loaded; `transport.frame_digest` with no picture yet |
| `failed` | the transport of the engine (of the source monitor, or the timeline transport) has stopped (a bug of the engine): it told the state with `media: null` as it stopped, and logged `transport_failed`; from then on every `transport.*` is refused so, but `transport.state`, which still answers with that state |
| `draw` | `transport.frame_digest` (a test hook) could not draw or read back the frame it was asked for |

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
| `set_proxy` | `media`, `proxy` (a proxy, or `null` or left out to take it away; its inverse sets the one before). A proxy must be a file of the engine's own (see Security), else it is refused as `not_a_proxy_path`; so must one on media that `add_media` adds |
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

A timeline shows at most one subtitle track. `set_track` with `enabled: true` on a subtitle
track while another subtitle track of its timeline is enabled is refused as `subtitle_shown`
(naming the shown one), and so are `add_track` of an enabled subtitle track then, and
`add_timeline` with two enabled subtitle tracks. The basic operations change no other track; to
show another track, a client disables the shown one first in the same transaction:
`[{"op":"set_track","timeline":"t1","id":"S1","enabled":false},
{"op":"set_track","timeline":"t1","id":"S2","enabled":true}]`. Its undo takes them back in the
reverse order. Disabling every subtitle track is allowed.

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
with only one end selected must be removed first. The operation refuses, atomically, a
clip moved before frame 0 (`out_of_timeline` `start`), past the largest integer (`too_large`),
missing ids, or overlap. It does not look at the kind of the track: on a subtitle track its
clips are `missing`. Its inverse moves the same ids by `-by`. `shift_cues` does
the same for subtitles of one subtitle track, refusing `cue_overlap`, `out_of_range` `start`
(before frame 0) or `wrong_track` for a track of picture or sound; the edits move every later
subtitle of a track with one `shift_cues`, however many there are. Both refuse an empty list
(`out_of_range` `clips` / `cues`) and `by: 0` (`out_of_range` `by`), which would make a step that
changes nothing.

Basic operations can change locked tracks, as undo/redo must. Interactive clients should send
edits for clip manipulation; those implement locks, links, clamping and ripple rules.

A thing added with an empty `id` is given one by the engine (`m`/`t`/`c`/`b`/`x`/`q`/`g`, and `n` for a node and `k` for a group, + a number from
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

A placement `source` is `{kind:"media", media, from?, to?, length?}` (from/to are media ticks,
`to` not included, as `mark_out`),
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
| `close_gap` | `track`, `at`, `straddling?:"refuse"\|"stay"` | Close the gap at `at` and ripple unlocked tracks. With `straddling:"stay"`, clips of other tracks across the gap stay where they are instead of refusing (see below). |
| `blade` | `at`, `tracks?`, `linked?` | Split crossing clips; original ids stay with first halves, second halves get new ids and link groups. |
| `move` | `clips`, `delta`, `to_track?`, `overwrite?`, `linked?` | Move clips; default refuses occupied destinations, `overwrite:true` clears them. With `to_track`, every clip that moves, the linked ones too, goes as many tracks up or down among the tracks of its own kind as `to_track` is from the track of the first clip (see below). |
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
| `add_track` | `kind`, `index?`, `name?`, `language?` | Add an empty track; index is among tracks of its kind; name is its display label. A subtitle track is added disabled (hidden) while another subtitle track of the timeline is enabled, and enabled otherwise. |
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

`close_gap` takes `"straddling"`: `"refuse"` (the default, omitted when written) refuses with
`ripple_blocked` where a clip of another unlocked track covers a frame of the gap, as every
ripple does. `"stay"` (Track 1 of the Cut page; DaVinci Resolve Reference Manual 20, pp.
584–588) leaves such clips where they are: what starts at the end of the gap or later still
moves back by the gap on every unlocked track, so superimposed clips stay in sync, and subtitles
in the gap are cut out with it as in every ripple. Where a clip after the gap would then run into
a clip that stays on its track, the edit is refused with `ripple_blocked` naming that track and
the clip that stays. Any other value makes the edit unreadable (`-32602`).

`move` with `to_track` moves clips across tracks as Linked Move Across Tracks does in DaVinci
Resolve (Reference Manual 20, p. 769, on by default there): the offset is that of `to_track`
from the track of the first clip, among the tracks of its kind, and every clip that moves goes
by that offset among the tracks of its own kind. A picture moved from V1 to V2 takes its linked
sound from A1 to A2, and a sound moved from A1 to A2 takes its picture from V1 to V2. With
`linked:false` only the clips named move, each still within its own kind. A clip linked to one
named that is on a locked track stays where it is. No track is made: a clip with no track of
its kind that far up or down is refused with `no_track` (`kind`), and one whose track to go to
is locked with `locked` (`track`). `to_track` must be of the kind of the first clip
(`wrong_kind`). Without `to_track`, clips stay on their tracks.

Trims, rolls, slips and slides clamp to available handles and space, reporting their input
index in `clamped`; zero delta or an impossible movement is refused. Editing through a
transition keeps it where its clips still meet, shortens it to available handles/coverage, or
removes it if none fits. A repair shortened to zero removes the transition and is also reported as clamped. Transitions and same-end
fades are not silently substituted. All changes are still one reversible transaction.

## Grades

Schema 3 adds the grades of the Color page (`crates/model/src/grade.rs`), as DaVinci Resolve
builds them (Reference Manual 20, Chapters 143–147, pp. 3308–3379). The project gains
`groups: [...]`; every timeline gains `grade`; every clip gains `grade` and `group` (a group id
or `null`). The engine always writes them; a client that builds a clip, a timeline or a project
without them (as one written for schema 2 does) may leave them out, and they are read as the
empty grade, no group and no groups. A clip of sound keeps a grade as it was set, as it keeps its
transform. A project of schema 2 is upgraded on open: each timeline and clip gets the empty
grade below, each clip `group: null`, and the project `groups: []`; nothing else changes. Its
original bytes are kept once in `project.schema2.json` (or `project.schema2-<digest>.json`
where that holds another project), as a schema 1 project is kept in `project.schema1.json`.

The picture goes through four levels, in this order (p. 3330): the pre-clip grade of the
clip's group, the clip's grade, the post-clip grade of its group, and, after the tracks are
put together, the timeline's grade. An operation names one with a `grade` target:

```json
{"level": "clip", "timeline": "t1", "clip": "c4"}
{"level": "pre_clip", "group": "k7"}
{"level": "post_clip", "group": "k7"}
{"level": "timeline", "timeline": "t1"}
```

A **group** is `{id, name, pre, post}` (`pre`/`post` are grades; `name` at most 100
characters). A clip is in at most one group, named by its `group`; a group that a clip is in
cannot be taken away (`grouped`). Groups are of the project, so clips of several timelines may
share one.

### A grade

```json
{"nodes": [], "links": [{"from": "source", "to": "output", "kind": "rgb", "input": 0}]}
```

is the empty grade every new clip and timeline has: the source linked straight to the output.
`nodes` are in the order they were added, which is how an editor numbers them (number = place
among the nodes + 1): an added node goes to the end, so a node added between nodes 1 and 2 is
number 4, and taking a node away moves the ones after it up a number (p. 3318).

A **link** `{from, to, kind, input}` goes from the output of `kind` (`rgb` or `key`) of `from`
(a node id or `source`) into input `input` (from 0) of that kind of `to` (a node id or
`output`). An output may feed any number of inputs; an input takes at most one link
(`input_taken`), as in DaVinci (p. 3319). Links never make a loop (`loop`). **Links are kept in
one order**, by `to` (compared as strings, byte by byte), then `kind` (`rgb` before `key`), then
`input`; the engine sorts the links of every grade it is given and of every project it reads,
and a client copy must keep them so to stay equal to the engine's.

Inputs and outputs of each kind of node (`no_port` for any other):

| kind | RGB inputs | key inputs | outputs |
|---|---|---|---|
| `source` | — | — | rgb |
| `output` | 1 | — | — |
| `corrector` | 1 | 1 | rgb, key |
| `parallel_mixer` | `params.inputs` | — | rgb |
| `layer_mixer` | `params.inputs` | — | rgb |
| `key_mixer` | — | `params.inputs.length` | key |

A grade whose output has no link, or with a node whose every RGB input (key input, for a key
mixer) is not linked or whose RGB output (key output) feeds nothing, is kept but leaves the
picture as it is, as DaVinci disables such a grade (pp. 3312, 3319); the model's
`Grade::is_complete` tells it.

A **node** is `{id, kind, label, enabled, params}`. `id` is unique within its grade (not in the
project: copying a grade keeps its ids), never `source` or `output`; the engine makes `n` +
`next_id`. `label` is at most 100 characters (an editor shows 12, p. 3313). A disabled node
passes its picture through. `params` by `kind`:

- `parallel_mixer`: `{"inputs": 2}` — 1…256 inputs, mixed equally (pp. 3334–3335).
- `layer_mixer`: `{"inputs": 2, "composite": "normal"}` — the lowest input is on top
  (p. 3336); `composite` is one of the composite modes of pp. 1055–1058 in snake case
  (`normal`, `add`, `color`, `color_burn`, `color_dodge`, `darken`, `darker_color`,
  `difference`, `divide`, `exclusion`, `hard_mix`, `hardlight`, `hue`, `lighten`,
  `lighter_color`, `linear_burn`, `linear_dodge`, `linear_light`, `luminosity`, `multiply`,
  `overlay`, `pin_light`, `saturation`, `screen`, `softlight`, `subtract`, `vivid_light`).
- `key_mixer`: `{"inputs": [{invert, matte, gain, offset}, ...], "output": {invert, gain,
  offset}}` — two inputs at first, in Mask mode (`matte: false`), gains 1 (pp. 3358–3363).
- `corrector`, in the units DaVinci shows (the page gives the manual's range or default; the
  others are ours, as the manual gives none):

| field | default | range |
|---|---|---|
| `primaries.lift` / `gamma` `{y,r,g,b}` | 0 | −1…1 |
| `primaries.gain` `{y,r,g,b}` | 1 | 0…16 |
| `primaries.offset` `{r,g,b}` | 25 | −75…125 |
| `primaries.contrast` | 1 | 0…2 |
| `primaries.pivot` | 0.435 | 0…1 |
| `primaries.saturation` | 50 (p. 3064) | 0…100 |
| `primaries.hue` | 50 (p. 3064) | 0…100 |
| `primaries.lum_mix` | 100 (p. 3064) | 0…100 |
| `primaries.temperature` | 0 | −4000…4000 (p. 3063) |
| `primaries.tint`, `color_boost`, `midtone_detail`, `shadows`, `highlights` | 0 | −100…100 (pp. 3063–3064) |
| `log.shadow` / `midtone` / `highlight` `{r,g,b}` | 0 | −1…1 |
| `log.low_range`, `log.high_range` | 0.333, 0.55 | 0…1 (p. 3073) |
| `curves.ganged` | `true` | |
| `curves.y` / `r` / `g` / `b` | `{"points": [[0,0],[1,1]], "intensity": 100}` | points `[in, out]` in 0…1, `in` rising, ≥ 2; intensity 0…100 (p. 3116) |
| `hsl.hue_vs_hue`, `hue_vs_sat`, `hue_vs_lum`, `lum_vs_sat`, `sat_vs_sat`, `sat_vs_lum` | `[]` (flat) | points `[in, out]`, `in` 0…1 rising, `out` −1…1, 0 changes nothing |
| `lut` | `null` | a path or library name, 1…4096 characters, no NUL |
| `soft_clip.low` / `high` / `low_soft` / `high_soft` `{r,g,b}` | 0 / 100 / 0 / 0 | 0…100, a share of the 10-bit scale (pp. 3119–3121) |
| `key.input` `{invert, matte, gain, offset}` | `false`, `true`, 1, 0 | gain 0…2, offset −1…1 (p. 3362) |
| `key.output` `{invert, gain, offset}` | `false`, 1, 0 | gain 0…2 (p. 3362), offset −1…1 |

An Outside node is a corrector with `key.input.invert: true` linked to the key of the node
before it (p. 3361). Within a corrector the settings apply in the order of p. 3331.

Every number setting above is a **parameter**: a number, or keyframes
`{"keys": [{"frame": 0, "value": 1.0, "interp": "linear"}, ...]}`. Frames count the frames of
the clip from 0 (of the timeline, for its grade), rise strictly, are within ±(2^53−1), and there
is at least one key; every value is within the range. Before the first key the value is the
first's, after the last the last's. `interp` says how the value goes to the next key: `"linear"`,
`"hold"` (a static keyframe, p. 3384) or `{"bezier": {"out": [t, dv], "in": [t, dv]}}`, a cubic
curve whose handles are each a share `t` (0…1) of the frames between the keys and a change of
value `dv` (any finite number): `out` from this key forward, `in` from the next key back
(`[0.33, 0]` for both eases out and in, p. 3385). `Param::value_at(frame)` evaluates it.

### Operations

| op | fields | inverse |
|---|---|---|
| `add_node` | `grade`, `node` (empty `id` for the engine to make one), `index?`, `links?` | `remove_node` with `links` |
| `remove_node` | `grade`, `id`, `links?` | `add_node` with its links |
| `connect` | `grade`, `link` | `connect` of the link the input had, or `disconnect` |
| `disconnect` | `grade`, `to`, `kind`, `input` | `connect` |
| `set_node` | `grade`, `id`, `label?`, `enabled?`, `params?` | `set_node` with the values as they were |
| `set_grade` | `grade`, `value` (a grade, its nodes with ids) | `set_grade` of the grade as it was |
| `add_group` | `group` (empty `id` for `k` + `next_id`), `index?` | `remove_group` |
| `remove_group` | `id` (no clip in it) | `add_group` |
| `set_group` | `id`, `name?` | `set_group` |
| `set_clip_group` | `timeline`, `clip`, `group` (an id or `null`) | `set_clip_group` |

- `add_node` links each come from the node or go into it, and name it by its id as given (by
  `""` when it is given none, or by its `$name`); a link into an input that has one takes its
  place (p. 3319). Two links into one input, or one that does not touch the node, are
  `out_of_range` `links`. Its `done` has the id the engine gave, the `index`, and the links in
  order; its inverse is `remove_node` with `links` set to the links it took the place of.
- `remove_node` without `links` links what came into the node's first linked input of each kind
  into every input its output of that kind went into, as DaVinci links the nodes on either side
  of a deleted node (p. 3318). With `links`, those are put instead: each must go into an input the
  node fed, and not touch the node (`out_of_range` `links`). Its `done` always carries the
  `links` it put.
- `set_node` `params` is a patch of the node's `params`: each object in it is put into the
  object of that name, and any other value (a number, a `{"keys"}` parameter put where a number
  was, an array, a string, `null`) takes the place of the one there, whole. What it makes must
  be the params of the node's kind (`params`, with serde's message) within their ranges
  (`out_of_range` with the path, such as `primaries.saturation`); a mixer cannot lose inputs
  that links go into (`no_port`). Its `done` and inverse carry the values at the patch's paths
  as the engine writes them (`60` comes back as `60.0`).
- Nodes of a grade put in whole (`set_grade`, `add_group`, a clip of `insert_clip`, `add_track`
  or `add_timeline`, a timeline's) have ids (`no_id`), unique in the grade (`taken` `node`), of
  which an `n…` id must be below `next_id` (`reserved`).

`$names` (see below) may be given to a node by `add_node` — its links name it by the same
`$name` — and to a group by `add_group`; later operations name them where they name a node, a
group or a clip's `group`.

### Edits

Each names a `grade` target as above; none needs `timeline`. They are worked out into the
operations above when applied, as the other edits are.

| edit | fields | behavior |
|---|---|---|
| `add_serial` | `grade`, `node` (a node with an RGB output, or `source`) | Add a corrector after it, taking what its RGB output fed (Add Serial, Option-S, p. 3318). |
| `add_serial_before` | `grade`, `node` (a corrector, or `output`) | Add a corrector before it, taking what came into it (Add Before Current, Shift-S). |
| `append_node` | `grade` | `add_serial_before` the output (Append Node, Option-K): the first node of an ungraded clip. |
| `add_parallel` | `grade`, `node` (a corrector) | Add a corrector taking what the node takes; if the node's picture goes only into a parallel mixer, the mixer gets one more input for it, else a new parallel mixer takes the node at input 0, the new corrector at input 1, and the node's place (Option-P, pp. 3334–3335). |
| `add_layer` | `grade`, `node` (a corrector) | As `add_parallel`, with a layer mixer; the new corrector is at the lowest input, on top (Option-L, p. 3336). |
| `extract_node` | `grade`, `node` | Take the node out of the tree, bridged as by `remove_node`, and keep it unlinked (E, p. 3319). |
| `reset_node` | `grade`, `node` | Set the node as a new one of its kind, keeping its label, `enabled` and inputs. |
| `copy_grade` | `from`, `to: [target, ...]` | `set_grade` each `to` to the grade `from`, ids kept (pp. 3286, 3290). |
| `append_grade` | `from`, `to: [target, ...]` | Add the nodes of `from` after the end of each `to`, with new ids: what fed the output feeds what the source fed, and what fed the output of `from` feeds the output (Append Node Graph, p. 3290). |
| `paste_value` | `from`, `from_node`, `to`, `to_node`, `param` | Set one field of `params`, by its path with dots (`primaries.saturation`, `primaries.lift`), to its value in the other node (Paste Value, p. 3286). A path that either node lacks is `out_of_range` `param`. |
| `paste_node` | `from`, `from_node`, `to`, `to_node` | Set the `params` of `to_node` to those of `from_node`, of the same kind (p. 3320). |

A wrong node for an edit is `wrong_node`; an empty `to` is `out_of_range` `to`.

### Refusals of grades

| reason | fields | meaning |
|---|---|---|
| `no_port` | `node`, `kind`, `input` (`null` for an output) | a link from an output, or into an input, that the node or end does not have |
| `input_taken` | `node`, `kind`, `input` | two links into one input |
| `loop` | `node` | the links would go round in a loop through `node` |
| `grouped` | `group`, `clip` | `remove_group` of a group that clip `clip` is in |
| `params` | `node`, `why` | `set_node` params that are not those of the node's kind |
| `wrong_node` | `node`, `kind` (the node's kind, `source` or `output`) | an edit that cannot be done to or about that node |

`missing` also names `group`, `node` and `link`; `taken` `node` and `group`; `out_of_range` also
`label`, `inputs`, `links`, `lut`, `param`, `to` and the paths of corrector settings.

### Grades in the picture

The timeline monitor and an export draw the grades alike (one code path, `grading.rs`): each
layer of a clip is graded by its group's pre-clip grade, its own grade and its group's post-clip
grade, in that order, after its picture is decoded and before it is moved, scaled and laid on
the canvas with its opacity; the timeline's grade is done over the canvas once every layer is on
it; the subtitles of the monitor are drawn over that, ungraded (p. 3330). An export burns no
subtitles in. A grade that is not complete, or has no node, changes nothing.

- **What is graded**: every clip of a track of picture, titles and colours as much as media (the
  model lets any clip carry a grade; DaVinci grades a Text+ title "like any other clip", p. 1047).
  The colour that a dip transition goes through is no clip and has no clip grade; the timeline
  grade is over it.
- **Frames of keys**: a key of a clip's grade, and of its group's grades as they apply to that
  clip, is at a frame of the timeline counted from the clip's first frame (0). The clip's speed
  and `reverse` do not move them: the Color page's Keyframe Editor mirrors the record timecode
  of the clip (p. 3382), and a speed change moves only Edit-page keys, when asked (p. 1073).
  Within a transition a clip is seen before its first frame or after its last, where its grade
  holds its first or last key. The keys of the timeline's grade count the frames of the
  timeline. (DaVinci ties a clip's keys to the source frames under them, pp. 3383, 3393: there a
  trim of the head leaves them on the same pictures, here at the same distance from the clip's
  first frame.)
- **Cost**: a grade is made into code once, when it changes (any of its settings, or the card);
  each frame takes only its parameters at that frame, so playing keyframes or dragging a setting
  compiles nothing, and a change of grades alone does not start the read-ahead again.
- **LUTs**: a corrector's `lut` that is an absolute path is read there; another is looked for in
  the folder of the project, then in the library of LUTs: the folders that the environment
  variable `COLUMBINA_LUTS` of the engine lists (absolute paths, separated as `PATH` is: `;` on
  Windows), or, where it is not set, `luts` in `COLUMBINA_HOME`, else in `.columbina` in the
  user's home. Only `.cube` is read; a cube is read between its points trilinearly (DaVinci's
  default, p. 152). A LUT is read again when the grade that names it changes; one that could not
  be read is looked for again at every change of the project.
- **Missing LUTs**: a LUT that cannot be read (no such file, not a `.cube`, a file that does not
  parse) leaves its node as though it named none, and is told as DaVinci marks it (p. 3413):
  `missing_luts: [{"name": "looks/film.cube", "message": "there is no LUT looks/film.cube in the
  folder of the project or in the library"}]`, by name, once each, of every grade of the timeline
  (its clips' on tracks of picture, their groups', its own), in the **timeline state** (left out
  where there is none, so a state without it has none missing) and in each `render.progress` of
  an export of that timeline (left out likewise).

## Names of a transaction (`$name`)

Within one `transaction.apply`, a thing that an operation adds may be given an id of the form
`$name`: `$` and 1–32 of `A–Z a–z 0–9 _`. The thing is added as if its id were empty, and given
one by the engine. Every **later** operation of the **same** transaction that names `$name` where
it names a thing (its `timeline`, `track` or `id`, the `media` of a clip, the `media` of
`set_proxy`, and the ids listed in `clips` of `shift_clips` and `cues` of `shift_cues`) is made
to name that
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

- Names may bind media, bins, timelines, tracks, clips, transitions, cues, nodes and groups (see
  Grades), including those
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
  output must exist. An empty name object, an unknown selector or a value that is not a string
  cannot be read: the transaction is refused as `-32602` `{"index": i, "reason": "invalid_op"}`. A missing output refuses the **whole** transaction with `-32001`,
  `data:{reason:"missing",what:"named output",id:"$a",index:i,message:...}`, preserving the
  project, revision, id counter and undo/redo.
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
  | `invalid_name` | a name given as a string that is not of the form `$` + 1–32 of `A–Z a–z 0–9 _` |
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
| `render.progress` | `{"job", "state": "running" \| "done" \| "failed" \| "cancelled", "frame", "frames", "fps", "path", "encoder"?, "message"?, "missing_luts"?}` (see Rendering a timeline, and Grades in the picture) |

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
| `-32600` | not a request object, `jsonrpc` is not `"2.0"`, no `method`, a bad `id` (answered with id `null`), a request before `hello`, a second `hello`, a `hello` of another protocol, a line over 16 MiB, `view.*` or `engine.yield` from a client that is not the desktop | `{"reason": "protocol", "supported": [2]}` for a `hello` of another protocol |
| `-32601` | no such method; `view.*` and `transport.*` in a `--headless` engine; `media.make_proxy` in an engine that makes no proxies; `engine.yield` in an engine that is not headless | |
| `-32602` | params are not what the method takes (or not an object); an operation that cannot be read; a bad `$name`; a speed of `transport.play` out of −8…8 or 0; `transport.load` of media or a timeline not in the project; `timeline.duration` of a timeline not there; `media.import` or `media.inspect` with no paths or a relative path; `media.thumbnail`, `media.waveform` or `media.make_proxy` of media not there; `media.thumbnail` of a frame outside the media, a height outside 1…270 or a width over 16384; a bad parent or rectangle of `view.*`; `render.start` with a bad path, codec, quality, timeline, range (an empty timeline has none, so it cannot be exported) or `subtitle_tracks`, and `render.cancel` of a job that is not there | `{"index": i, "reason": "invalid_op"}` for an operation that cannot be read; see names above; `{"reason": "speed"}`; `{"index": i, "path": "...", "reason": "relative"}`; `{"reason": "parent"}` or `{"reason": "rect"}` (`view.*`); `{"reason": "missing", "media": "..."}` or `{"reason": "missing", "timeline": "..."}` (`render.start` too); `{"reason": "out_of_range", "frame": n}`; `{"reason": "subtitle_track", "track": "..."}` for a `subtitle_tracks` entry that is not a subtitle track of the timeline, or is named twice |
| `-32001` | a transaction, undo or redo was refused; the project is exactly as it was; or `render.start` was refused before anything was written or taken away (with `{"reason":"stopping"}` while the engine is stopping; with no `data` when the export could not be started) | the refusal, below |
| `-32002` | `media.import` or `media.inspect`: a file cannot be read; nothing was added. `transport.load`, `media.thumbnail`, `media.waveform`: the file of the media cannot be opened or decoded. The analysis of the engine stopped (no `data`) | `{"path": "<as sent>", "reason": "...", "message": "..."}`, below |
| `-32003` | `view.*`: the view cannot be made, there is none, or its window is gone | `{"reason": "..."}`, see The view |
| `-32004` | `transport.*`: no media is loaded, the transport has stopped, or the test hook could not draw | `{"reason": "..."}`, see The transport |

The connection stays usable after every error.

**Files that cannot be read (`-32002`).** `reason` is one of `not_found` (no such file),
`unreadable` (it may not be read, or it is a directory), `not_utf8` (its path is not UTF-8 or
holds a NUL), `not_media` (libav cannot open it as media of the formats above), `no_streams`
(neither picture nor sound), `unsupported` (what it holds cannot be used: a picture that libav
cannot decode, no duration, a picture with no size or rate), `read` (it cannot be read on, once
opened), or `failed` (the engine failed to read it). `transport.load` gives these too, and
`decode` (its first frame cannot be decoded) and `seek`; `media.thumbnail` and `media.waveform`
may also give `no_video`, `no_audio`, `too_large` or `encode`, and the reading of media may give
`device`, `out_of_range`, `no_encoder` or `write`.

**Refusals (`-32001`).** `data` is `{"reason": "...", "message": "...", ...fields}` for a refused
transaction, undo or redo, with
`"index": i` (the place of the refused operation in `ops`) when an operation was refused, and
`{"reason": "empty", "message": ...}` for a transaction with no operations. An undo or a redo is
refused only through a bug of the engine. The refusals that are not of a transaction
(`stale_revision`, and `busy`, `exists`, `part_exists`, `source` and `stopping` of `render.start`) carry no
`message` in `data`: it is the `message` of the error. Each `reason` and its fields:

| reason | fields | meaning |
|---|---|---|
| `missing` | `what` (`media`, `bin`, `timeline`, `track`, `clip`, `transition`, `cue`, `named output`), `id` | the thing named is not there |
| `taken` | `what`, `id` | the id of an added thing is held by another of its kind |
| `reserved` | `id` | an id of the engine's own form (`m`/`t`/`c`/`b`/`x`/`q`/`g`/`n`/`k` + number) not made yet |
| `dollar` | `id` | an id that starts with `$` (only names of a transaction do; the engine replaces or refuses them before this) |
| `out_of_list` | `at`, `len` | an `index` past the end of its list |
| `no_stream` | `media`, `kind` (`video`, `audio`) | the media has no stream for the track |
| `wrong_kind` | `track`, `kind` | a clip moved to a track of the other kind |
| `out_of_source` | `media`, `why` (`length`, `before_start`, `past_end`, `handles`, `marks`) | the clip lasts no frame, shows what is not in its media, a transition needs more of it than there is, or a mark is outside the media (or `mark_in` is not before `mark_out`) |
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
| `stale_revision` | `expected`, `actual` | `transaction.apply` or `render.start` with a `revision` that is not the project's (see Methods, and Rendering a timeline) |
| `busy` | | `render.start` while an export is running |
| `exists` | `path` | `render.start` without `replace` onto an existing movie or SRT |
| `part_exists` | `path` | `render.start` where the partial file `<path>.part` of the movie or an SRT is there (see Rendering a timeline) |
| `source` | `path`, `media` | `render.start` onto the file of media of the project, or of its proxy |
| `stopping` | | `render.start` while the engine is stopping |

### Additional schema 2 and edit refusals

| reason | fields | meaning |
|---|---|---|
| `locked` | `track` | an edit directly names content on a locked track |
| `ripple_blocked` | `track`, `clip` | an unselected clip occupies frames a ripple would close |
| `no_room` | `track` | there is no usable space for the edit |
| `no_handles` | `clip`, `need`, `have`, `longest?` | insufficient source/coverage; `longest` is the maximum possible transition length |
| `empty_range` | | zero-length range/delta or an edit that cannot move |
| `nothing_at` | `track` (nullable), `frame` | no target clip at the requested frame |
| `no_track` | `kind` | no track for the requested source, or, for a `move` with `to_track`, no track of `kind` for a clip that moves to go to |
| `wrong_track` | `what`, `kind` | content or settings on an incompatible track kind |
| `cue_overlap` | `with` | subtitles overlap on one track |
| `subtitle_shown` | `track` | a subtitle track would be enabled (by `set_track`, `add_track` or `add_timeline`) while subtitle track `track` of its timeline is enabled; a timeline shows one at a time |
| `out_of_range` | `what` | an invalid setting: `width`, `height`, `sample_rate`, `start`, `length`, `speed`, `transform`, `gain_db`, `pan`, `fade`, `text`, `size`, `position`, `language`, `still`; `rotation` for media whose `facts.video.rotation` is not 0, 90, 180 or 270; `digest` for media whose `identity.digest` is not 64 lowercase hex digits; `by`, `clips` or `cues` for a shift that would change nothing |
| `has_transition` | `transition` | `remove_clip`, `move_clip` or `trim_clip` of a clip that has a transition (remove it first), a `set_clip` that its transition cannot follow, or a `shift_clips` that moves just one end of a transition |
| `not_adjacent` | `left`, `right` | transition endpoints do not meet |
| `cut_taken` | `with` | a transition is already at this cut |
| `too_long` | `transition`, `clip` | transitions cover more than the available clip length |
| `source` | `clip` | `set_clip` cannot change the kind or media of a source, and `trim_clip` gives no `source_in` to a title or color clip (not the `source` of `render.start`, which carries `path` and `media`) |
| `cycle` | `bin` | cyclic bin parenting |
| `fade_at_transition` | `clip`, `edge` | fade and transition at the same clip end |
| `not_empty` | `bin`, `holds` | removing a bin that still holds content |

As with basic operations, edit refusal indexes refer to the original mixed `ops` array, not
the expanded operation list. Refusal emits no `project.changed` and adds no history step.

## Rendering a timeline

`render.start` is available to every greeted client, including a headless engine:

```json
{"timeline":"t1","path":"C:/Exports/film.mp4","codec":"h264","quality":"high","range":{"from":0,"to":250},"subtitle_tracks":["S1","S2"],"replace":false,"revision":42}
```

`codec` is `h264`, `hevc`, or `prores`; `quality` is `high`, `medium`, or `small`.
`range` is optional and half-open; when omitted, it covers the full timeline. A client that
exports from a timeline In to an Out frame sends `to` as the frame after the Out, so that the Out
frame is exported. It must be
nonempty and inside the timeline. The first exported frame becomes frame zero in the file.
`subtitle_tracks` lists the subtitle tracks of the timeline to write beside the film as SRT,
as the checkboxes of DaVinci Resolve's Deliver page choose them; it defaults to none. Each must
be a subtitle track of the timeline, named once, shown or hidden: a hidden track is exported
too. Otherwise `render.start` is refused with `-32602` and
`{"reason":"subtitle_track","track":"S9"}` before anything is written. `replace` defaults to
false. `revision` is optional: a client that prepared the export by the project at a revision
(for example the names of the SRT files, which it read with `project.get`, to tell what
`replace` would replace) gives it, and the export is refused with `-32001` and
`{"reason":"stale_revision","expected":n,"actual":k}`, as `transaction.apply` is, where the
project is at another revision, before anything is written or taken away. Unknown fields are
refused, including the `subtitles` of earlier drafts.

The path must be absolute, its parent directory must exist, and its extension must be `.mp4`
(H.264/HEVC) or `.mov` (any codec), in either letter case. Export cannot write inside the project's cache, including
through a directory link, or replace a directory or symbolic link. An existing destination or
SRT with `replace: false` is refused with `-32001` and `{reason:"exists",path}`. A destination
(movie or SRT, or the `.mov` of a ProRes fallback) that is the file of any media of the project
or of its proxy, compared as the system resolves the paths (case-insensitively on Windows), is
refused before anything is written with `-32001` and `{reason:"source",path,media}`. Invalid path,
codec, quality, timeline or range is `-32602`; a timeline that is not there is
`{"reason":"missing","timeline":"..."}`, as for `timeline.duration`. One export runs at a time;
a second start is refused with `-32001` and `{reason:"busy"}`, and one asked for while the engine
is stopping with `{reason:"stopping"}`.

The answer is `{job:"render-1"}` and is queued before that job's first `render.progress`:

```json
{"job":"render-1","state":"running","frame":36,"frames":250,"fps":48.2,"path":"C:/Exports/film.mp4","encoder":"h264_nvenc"}
```

`frame` counts frames completed, not absolute timeline frames; `frames` is the total. Running
notifications are sent at most once per 250 ms. The terminal notification is always sent,
with `state` equal to `done`, `failed`, or `cancelled`. `encoder` is present once opened:
the name of the libavcodec encoder (`h264_nvenc`, `hevc_amf`, `prores_ks`).
`message` explains a failure, a codec fallback, or a graphics device that was lost and made
again. `path` reports the actual destination.
The project revision and undo history are not changed by an export.

The export uses a snapshot taken at start, independent original-media decoders, the same
scene compositor as preview, and the same stereo timeline mixer. It never uses proxies.
Edits, undo, or deletion of that timeline after start do not change the export. Titles are
composited into the picture. Subtitle tracks are **not burned into the picture**.
The GPU is shared with preview: a waiting preview composition takes the next turn; export
submits and reads back one frame at a time. If the graphics device is lost during an export
(a driver reset), the export waits up to 30 s for a new device, draws the frame it was at
again and goes on: no frame was written for it yet, so the file is whole; the final `message`
says the device was made again. If none can be made, or it is lost three times on one frame,
the export fails, saying so. An encoder that fails after it opened (a frame, the sound or
finishing the file) fails the export with a message naming the encoder and the frame; the
ProRes fallback is only ever taken when no encoder opens, never in the middle of a file. Audio is AAC, 192 kb/s, 48 kHz stereo; a timeline with no enabled
clip on an enabled audio track gives a file with no sound. H.264 uses 8-bit picture; HEVC uses
10 bits when the project contains media of more than 8 bits; ProRes always 10 bits. If no hardware
encoder can open H.264/HEVC, the engine tries ProRes in a `.mov`, reports that change, and
checks the new destination against the same overwrite rules.

Each track in `subtitle_tracks` gets an SRT, in the order of the tracks of the timeline: one
track uses `film.srt`; multiple tracks use `film.<language-or-track-name>.srt`. Unsafe filename characters are
replaced, the label is cut to 80 characters (`subtitles` if nothing is left), and names that
are the same regardless of letter case get the suffixes `-2`, `-3`, …, without hashes. Cue times are clipped to
the export range and rebased to zero; frame times use the exact rational frame rate, rounded
down to milliseconds. The file is UTF-8 without a BOM, and every line ends with CRLF, as SubRip
wrote it: the lines of a cue's text (its newlines, `\n`, `\r\n` or `\r`, are normalized), and the
blank line between cues. An empty line inside a cue is written as one space, so
it does not end the cue early. Text is written as it is: players do not
decode entities, so `&`, `<` and `>` are not escaped. Only what a player would read as a tag is
broken up: players (ffmpeg, mpv) read `<` up to the next `>` on a line as a tag (`<i>`, `</b>`,
`<font color=…>`, and drop any other), so a word joiner (U+2060, invisible and zero-width) is
written after each `<` that a `>` follows on its line, and `<i>x</i>` is shown as typed. A `<`
with no `>` after it (`<3`, `a<b`) is written as it is. `{\…}` override codes are written as they
are, so `{\an8}` places a line at the top as players expect.
Tracks not listed are omitted, shown or not; a listed track with no cue in the range produces
an empty SRT.

`render.cancel {job}` answers `{}` and requests cancellation; wait for the terminal progress
notification before assuming its files have been cleaned up. An unknown job is `-32602`.
Cancellation of the most recently completed job is harmless. Engine shutdown cancels and
joins its export before releasing the project.

The movie and SRTs are first written beside their destinations as `<path>.part`. Existing
partial files are not overwritten: one that is there when an export starts (left by an engine
that ended while it exported, or by another program) is refused before anything is written
with `-32001` and `{reason:"part_exists",path}`, `path` naming the partial file. With
`replace: true` it is deleted once every other check of the export has passed (a refused export
deletes nothing), unless a program still has it open (as ffmpeg has the file it writes; Windows
then refuses to delete it) or it is not a file: then it is refused so too.
On cancellation or failure these owned partials are
removed. At publication, sidecars are renamed first and the movie last; previous destinations
are retained in temporary backups until all renames succeed, then removed. A publication
failure rolls back previously renamed files. If the filesystem prevents restoring a backup,
the old file is retained and its recovery path is logged as `render_restore_failed`.
