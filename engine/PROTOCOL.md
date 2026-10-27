# The engine protocol (version 1)

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
- The directory is made if it is not there. The engine takes the lock of the project, opens
  `project.json` (a new, empty project if there is none), and listens.
- Its first line on **stdout** is one JSON object, written once it listens:
  `{"listening": "<address>", "pid": 1234, "protocol": 1}`. Nothing else is ever written to
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
  | 0 | ended as asked (idle, stdin closed, a signal) | |
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
| `hello` | `{"client": "desktop" \| "agent" \| "cli", "protocol": 1}` | `{"engine": "0.1.0", "protocol": 1, "revision": n, "project": "<dir>", "pid": 1234}` |
| `project.get` | none | `{"revision": n, "next_id": k, "project": {...}}` |
| `transaction.apply` | `{"ops": [op, ...]}` | `{"revision": n, "done": [op, ...], "next_id": k, "names": {"$a": "c7", ...}}` |
| `history.undo` | none | `{"revision": n, "done": [op, ...], "next_id": k}`, or `{"revision": n, "done": null}` if there is nothing to undo |
| `history.redo` | none | as `history.undo` |
| `media.import` | `{"paths": ["<file>", ...]}` | `{"revision": n, "added": ["m3", ...], "next_id": k}` |
| `view.attach` | `{"parent": "<handle>", "x": x, "y": y, "width": w, "height": h}` | `{"adapter": "<name>", "backend": "Dx12"}` |
| `view.place` | `{"x": x, "y": y, "width": w, "height": h}` | `{}` |
| `view.show` | `{"shown": true \| false}` | `{}` |
| `view.detach` | none | `{"detached": true \| false}` |

- `hello` says once per connection what the client is; a second `hello` is refused (`-32600`).
  From then on the connection is sent every notification. `protocol` other than 1 is refused
  (`-32602`, `data: {"protocol": 1}`). `project` is the directory as the engine writes it (see
  Address); `pid` is the engine's.
- `transaction.apply` applies its operations in order, all or none; an empty list is refused.
  `done` is the operations as they were done (see below). `names` maps each `$name` of the
  transaction (see below) to the id the engine gave it; `{}` if there were none.
- `media.import` reads each file (its identity, and what it holds, as below) and adds it to the
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
  (`C:\media\a.mp4` on Windows).
  - `identity`: `size`, `modified_us` (microseconds since 1970, UTC) and `digest`: the SHA-256 of
    the size in decimal, the first 64 KiB, and the last 64 KiB from no earlier than byte 65536,
    as the Python editing makes it.
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
- `history.undo` takes back the latest step **whoever made it** (desktop and agent share one
  line of undo); `history.redo` does again the step undone last, with the same ids. A new
  transaction clears what could be redone. 1000 steps are kept.
- `view.*`: the viewer, below.

## The view

The engine draws the picture of the viewer itself, with the graphics card, into a native child
window that it makes inside the desktop's window, over the rectangle that the page keeps for
the viewer (`crates/view`). No frame crosses between the processes; the page never sees the
picture.

- **Who sends them.** Only the desktop (`hello` with `"client": "desktop"`) may call `view.*`;
  another client is refused with `-32600`. Within the desktop, only the **Electron main
  process** sends them: it owns the window and its handle. The page (the renderer) never gets
  the handle and never calls the engine for the view: it tells the main process where the
  viewer is (over IPC), and the main process sends `view.place`. A `--headless` engine draws in
  no window: every `view.*` is answered `-32601`.
- **`parent`** is the native handle of the window, written in **decimal**, as a string: on
  Windows the `HWND` of `BrowserWindow.getNativeWindowHandle()` (a Buffer of 8 bytes: read it
  with `readBigUInt64LE()` and send its `toString()`; a signed reading is taken too). On macOS
  it would be the `NSView *` of that call, but a view can only be put in an `NSView` of the
  engine's own process, so `view.attach` cannot work across the processes on macOS as it is
  (see below).
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
  **one view at a time**: a second `view.attach` destroys the first and makes another, in the
  same or another window. The view is the engine's, not the connection's: it stays when the
  connection that made it closes, until `view.detach`, another `view.attach`, or the end of the
  engine. A desktop that reconnects attaches again.
- **`view.place`** moves and sizes the view, as the page's layout changes (the window resized,
  a panel dragged). It is answered at once; the window follows a moment later (it is moved by
  a thread of the engine that serves it, and drawn again at its new size). Send it whenever the
  viewer's rectangle changes, and after the parent's DPI changes.
- **`view.show`** hides the view, and shows it again. A native window covers whatever of the
  page lies over its rectangle ("airspace"): an HTML menu, dialog or tooltip that opens over the
  viewer is hidden under it. So the page hides the view (`{"shown": false}`) as such an overlay
  opens and shows it again as the overlay closes; meanwhile the page beneath shows (it may draw
  the last frame there, or nothing). A view that is hidden draws nothing.
- **Input** goes to the page: the view is a disabled window that never takes the focus, so the
  clicks, wheel and keys of the user over the viewer go to the page's own element there, as if
  the view were not there.
- **What is drawn.** Black, until a picture is given to the view: in phase 0 by playback and
  scrubbing (Task 13), not by a method of the protocol. A picture is fitted into the view, its
  shape kept and centred, with black bars, turned as its media says, its colours taken as
  BT.709. The engine draws only when there is something new to draw (a picture, a new size, the
  window shown or uncovered), never on a clock, and with v-sync.
- **`view.detach`** destroys the view; `detached` says whether there was one.
- **Failures.** A view whose window was destroyed with its parent (the window closed) is let
  go: `view.place` and `view.show` are then answered `-32003` with `reason: "gone"`, and the
  desktop attaches again to its new window. If drawing a frame takes more than 1 s (the card
  hangs, a driver resets), the view hides itself, so that the page is seen rather than a frozen
  picture, and shows itself again once the frame returns. A device that is lost is made again;
  so is a surface. These are logged on stderr (`view_ready`, `view_stalled`, `view_recovered`,
  `view_device_lost`, `view_surface_lost`, `view_error`), with `view_attached` and
  `view_detached`.
- **Not tested yet** (phase 0): the border of the window dragged by the user (Windows' modal
  loop of sizing), the window moved to a monitor of another DPI, fullscreen, sleep and resume,
  and a device lost for real (it is lost on purpose in a test). **macOS** is written (an
  `NSView` added to the parent's, that takes no clicks, drawn with Metal) but never run, and it
  needs the parent view in the engine's process: across processes, macOS needs another way (a
  layer of the engine shown in Electron's window with `CALayerHost`, or the pictures handed
  over as `IOSurface`s), to be settled before the desktop is built for macOS.

| `-32003` `reason` | when |
|---|---|
| `no_view` | `view.place` or `view.show` with no view attached |
| `gone` | the view's window was destroyed with its parent; the view is let go |
| `window` | the window of the view could not be made |
| `gpu` | no graphics card can draw in the window |
| `unsupported` | views are not made on this system |
| `failed` | the engine failed to make the view |

A `parent` that is not a window, or not a handle in decimal, is refused with `-32602` and
`data: {"reason": "parent"}`; a rectangle wider or higher than 16384 with `{"reason": "rect"}`.

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
| `trim_clip` | `timeline`, `id`, `source_in`, `start`, `length` |

A thing added with an empty `id` is given one by the engine (`m`/`t`/`c` + a number from
`next_id`, never reused; tracks `V<n>`/`A<n>` by place). An id that starts with `$` is never
kept (see names below). The rules are in `crates/model/src/ops.rs`.

**`done`** lists the operations as they were applied, each with every id the engine gave and the
place each thing was put at (the `index` of an added thing is filled in): applied in order to a
copy of the project at revision `n - 1` whose `next_id` was first set to the `next_id` sent with
them, they give the project at revision `n`. Without that `next_id`, a copy would refuse the
engine's new ids as `reserved`.

## Names of a transaction (`$name`)

Within one `transaction.apply`, a thing that an operation adds may be given an id of the form
`$name`: `$` and 1–32 of `A–Z a–z 0–9 _`. The thing is added as if its id were empty, and given
one by the engine. Every **later** operation of the **same** transaction that names `$name` where
it names a thing (its `timeline`, `track` or `id`, and the `media` of a clip) is made to name that
id before it is applied. The ids are answered in `names`; `done` and `project.changed` carry only
real ids. For example:

```json
{"ops": [
  {"op": "add_media", "media": {"id": "$m", "path": "C:/media/a.mp4",
    "identity": {"size": 1000, "modified_us": 1790000000000000, "digest": "0000000000000000000000000000000000000000000000000000000000000000"},
    "facts": {"duration": {"ticks": 153600, "scale": 15360},
      "video": {"width": 1920, "height": 1080, "rate": {"num": 25, "den": 1}, "codec": "h264", "bits": 8, "rotation": 0},
      "audio": {"rate": 48000, "channels": 2, "codec": "aac"}}}},
  {"op": "add_timeline", "timeline": {"id": "$t", "name": "Cut", "rate": {"num": 25, "den": 1}, "tracks": []}},
  {"op": "add_track", "timeline": "$t", "track": {"id": "$v", "kind": "video", "clips": []}},
  {"op": "insert_clip", "timeline": "$t", "track": "$v",
    "clip": {"id": "$c", "media": "$m", "source_in": {"ticks": 0, "scale": 15360}, "start": 0, "length": 50}}
]}
```
→ in a new project, `"names": {"$c": "c3", "$m": "m1", "$t": "t2", "$v": "V1"}`.

- Names may be given to media, timelines, tracks, and clips (also those inside an added timeline
  or track). A name lives only for its transaction.
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

- `by` is the kind of the client that asked (for an undo or a redo, the one that asked for
  that); `made_by` is the kind of the client that made the transaction (for an undo or a redo,
  the one that made the step taken back or done again).
- **Keeping a copy.** Call `project.get` after `hello`; then for each `project.changed`: ignore
  it if `revision` is not above yours; if it is yours + 1, set your `next_id` to its `next_id`
  and apply `done` in order; if it skips a revision (which the engine never does on one
  connection), call `project.get` again.
- `engine.error` with `kind: "save"`: the project could not be saved; the engine keeps it and
  tries again, more slowly while it keeps failing (at least every 32 s).

## Errors

| code | when | `data` |
|---|---|---|
| `-32700` | the line is not JSON (id `null`) | |
| `-32600` | not a request object, `jsonrpc` is not `"2.0"`, no `method`, a bad `id` (answered with id `null`), a request before `hello`, a second `hello`, a line over 16 MiB, `view.*` from a client that is not the desktop | |
| `-32601` | no such method; `view.*` in a `--headless` engine | |
| `-32602` | params are not what the method takes (or not an object); an operation that cannot be read; a bad `$name` | `{"index": i, "reason": "invalid_op"}` for an operation that cannot be read; see names above |
| `-32001` | a transaction, undo or redo was refused; the project is exactly as it was | the refusal, below |
| `-32002` | `media.import`: a file cannot be read; nothing was added | `{"path": "<as sent>", "reason": "...", "message": "..."}`, below |
| `-32003` | `view.*`: the view cannot be made, there is none, or its window is gone | `{"reason": "..."}`, see The view |

The connection stays usable after every error.

**Files that cannot be read (`-32002`).** `reason` is one of `not_found` (no such file),
`unreadable` (it may not be read, or it is a directory), `not_utf8` (its path is not UTF-8 or
holds a NUL), `not_media` (libav cannot open it as media of the formats above), `no_streams`
(neither picture nor sound), `unsupported` (what it holds cannot be used: a picture that libav
cannot decode, no duration, a picture with no size or rate), `read` (it cannot be read on, once
opened), or `failed` (the engine failed to read it).

**Refusals (`-32001`).** `data` is `{"reason": "...", "message": "...", ...fields}`, with
`"index": i` (the place of the refused operation in `ops`) when an operation was refused, and
`{"reason": "empty", "message": ...}` for a transaction with no operations. An undo or a redo is
refused only through a bug of the engine. Each `reason` and its fields:

| reason | fields | meaning |
|---|---|---|
| `missing` | `what` (`media`, `timeline`, `track`, `clip`), `id` | the thing named is not there |
| `taken` | `what`, `id` | the id of an added thing is held by another of its kind |
| `reserved` | `id` | an id of the engine's own form (`m`/`t`/`c` + number) not made yet |
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
| `empty` | | the transaction has no operations |
