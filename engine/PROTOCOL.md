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
| `transport.*` | see The transport | |

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
- `view.*`: the viewer, below; `transport.*`: playing a source, below.

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
  **one view at a time**: a second `view.attach` destroys the first and makes another, in the
  same or another window. The view is the engine's, not the connection's: it stays when the
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
  viewer is hidden under it. So the page hides the view (`{"shown": false}`) as such an overlay
  opens and shows it again as the overlay closes; meanwhile the page beneath shows (it may draw
  the last frame there, or nothing). A view that is hidden draws nothing. The view is put on
  top of the parent's other child windows each time it is moved, shown or hidden (`HWND_TOP`),
  so a native window of the app's own over the viewer (another view of Electron, a
  `WebContentsView` laid over it) is covered too: the app hides the view while such a window
  lies over the viewer, as for an overlay of the page.
- **Input** goes to the page: the view is a disabled window that never takes the focus, so the
  clicks, wheel and keys of the user over the viewer go to the page's own element there, as if
  the view were not there.
- **What is drawn.** Black, until a picture is given to the view: in phase 0 by the transport
  (below), not by a method of the view. A picture is fitted into the view, its
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
  `view_detached`. `view_ready` says the engine's `dpi_awareness`: `per_monitor_v2`, or why
  not, in which case the rectangles will be off.
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

The transport is the **source monitor** of phase 0: one media item of the project at a time is
loaded, played forward and backward, sought, scrubbed and stepped a frame at a time. Its picture
is drawn in the view (above), if one is attached, and its sound is played on the default output
device of the system. Timelines are played in phase 1.

- **Who sends them.** Any client (`desktop`, `agent`, `cli`); there is one transport, which every
  client sees and moves. A `--headless` engine has nothing to show and nowhere to play sound:
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
| `transport.load` | `{"media": "m1"}` | the state |
| `transport.play` | `{"speed": s}`: a whole number from −8 to 8, not 0; 1 if left out | `{}` |
| `transport.pause` | none | `{}` |
| `transport.seek` | `{"frame": n, "scrubbing": false}` (`scrubbing` false if left out) | `{}` |
| `transport.step` | `{"frames": n}` | `{}` |
| `transport.state` | none | the state |

**The state** (the result of `transport.load` and `transport.state`, and the params of the
notification `transport.state`):

```json
{"media": "m1", "frame": 37, "playing": false, "speed": 1, "exact": true,
 "shown": 37, "last": 99, "sound": "off"}
```

- `media`: the media loaded, or `null` (then `frame`, `shown` and `last` are `null` too).
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
  300 ms to wake), or when it stops asking for 250 ms while it plays: playing goes on at once
  from where it is, by the clock of the system, with `sound: "unavailable"` (logged
  `transport_sound_lost`). The device is opened anew at the next `transport.load` or
  `transport.play` that plays sound, and so is the default device of the system when it is
  another than the one opened (a headset plugged in, or chosen in the settings of the system),
  so that sound is `"on"` again as soon as it can be.

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
  Backward, each frame is sought: on media of long groups of frames (most from cameras) only
  some are decoded in time, and those are shown.
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
  been decoded yet) is decoded at once. Showing a frame that is not a key frame takes up to a few
  hundred milliseconds on long groups of frames (4K HEVC from a camera); proxies (Task 13b) will
  make it quick.
- **`transport.step`** pauses, then moves exactly `frames` frames (negative is back), clamped to
  the media, and shows that frame. Steps sent faster than they are done are added up.
- **The view.** The picture is drawn in the view if one is attached, turned as the media says;
  a view attached later is drawn the picture shown. Without a view the transport plays all the
  same (the state is told as ever).
- **Logs** (stderr): `transport_loaded` (`decoder`: `cuda`, `d3d11va`, `videotoolbox` or
  `software`, and the devices that failed before it, or `null` and the `grid` for sound alone),
  `transport_played` (each time playing stops: how many frames were shown, passed over or
  decoded too late, how late they were shown against the clock of the transport, how far the
  clock of the sound went from the system's, and the latency of the device and the length of
  its runs), `transport_unloaded`, `transport_error`, `transport_device_failed`,
  `transport_sound_lost`, `transport_failed`, `sound_ready`, `sound_unavailable`,
  `sound_failed`.

| `-32004` `reason` | when |
|---|---|
| `not_loaded` | `transport.play`, `.pause`, `.seek` or `.step` with no media loaded, and none being loaded |
| `failed` | the transport of the engine has stopped (a bug of the engine): it told the state with `media: null` as it stopped, and logged `transport_failed`; every `transport.*` is refused so from then on |

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
| `transport.state` | the state of the transport (see The transport) |

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
| `-32601` | no such method; `view.*` and `transport.*` in a `--headless` engine | |
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
