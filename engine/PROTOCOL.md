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
  `--proxy-encoder`, `--proxy-watchdog`); clients do not use them. `--sound` takes `device`,
  `null`, `stall`, `fail-after=<ms>` or `capture=<path>`: `capture` plays as `null` does (runs of
  10 ms by the clock of the system, heard 20 ms later) and writes every frame of the timeline
  monitor's sound that it counts as played, in order, as 32-bit floats, little-endian, two
  channels interleaved at 48 kHz: the first session of sound (from a play, a seek, a change of
  speed) into `<path>`, the second into `<path>.2`, the third into `<path>.3`, and so on, so that
  frame `n` of a file is the sample `from + n` of the timeline that its session started at (the
  source monitor then plays on `null`). Frames that the capture could not keep up with are left
  out and counted (`audio_capture_dropped`, below).

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

The one exception is a **gesture** (a control dragged, see Gestures): its transactions are told
at most 10 times a second, several in one `project.changed` whose `base` names the revision
that it applies to, so that the answer to one of them may come before it is told. What a
gesture held is told before anything else of the project (another change, an undo, the end of
the gesture, the answer to `hello` or `project.get`), so every client still sees every change,
in order, with no gaps.

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
| `transaction.apply` | `{"ops": [op_or_edit, ...], "revision": n, "gesture": {"id": "g1", "end": false}}` (`revision`, `gesture` and `end` optional) | `{"revision": n, "done": [op, ...], "next_id": k, "names": {"$a": "c7", ...}, "clamped": [i, ...]}` |
| `history.undo` | none | `{"revision": n, "done": [op, ...], "next_id": k}`, or `{"revision": n, "done": null}` if there is nothing to undo |
| `history.redo` | none | as `history.undo` |
| `media.import` | `{"paths": ["<file>", ...], "bin": "<bin id>"}` (`bin` optional) | `{"revision": n, "added": ["m3", ...], "next_id": k}` |
| `media.inspect` | `{"paths": ["<file>", ...]}` | `{"media": [media, ...]}` (empty ids; no project change) |
| `media.make_proxy` | `{"media": "m1"}` | `{}` (see Proxies) |
| `media.thumbnail` | `{"media": "m1", "frame": 50, "height": 90}` (`frame` optional) | `{"width": 160, "height": 90, "png": "<base64>"}` |
| `media.waveform` | `{"media": "m1", "stream"?: 1}` | `{"rate": 100, "channels": 2, "peaks": "<base64>"}`, or `{"pending": true}` |
| `view.attach` | `{"monitor":"source" \| "timeline", "parent": "<handle>", "x": x, "y": y, "width": w, "height": h}` | `{"adapter": "<name>", "backend": "Dx12", "compiler": "dxc" \| "fxc" \| "auto" \| null}` |
| `view.place` | `{"monitor":"source" \| "timeline", "x": x, "y": y, "width": w, "height": h}` | `{}` |
| `view.viewport` | `{"monitor": m}` to query; `{"monitor": m, "mode": "fit"}` or `{"monitor": m, "mode": "scale", "scale": 1.0, "center": {"x": 0.5, "y": 0.5}}` to set | The requested setting: `{"monitor", "mode"}` or `{"monitor", "mode", "scale", "center"}` |
| `view.show` | `{"monitor":"source" \| "timeline", "shown": true \| false}` | `{}` |
| `view.detach` | `{"monitor":"source" \| "timeline"}` | `{"detached": true \| false}` |
| `view.display` | `{"hdr"?: true \| false}` | `{"hdr": true, "source": {"hdr_on": true, "hdr_shown": true, "peak_nits": 400.0, "sdr_white_nits": 148.0} \| null, "timeline": ... \| null}` (see The view) |
| `view.sample` | `{"monitor": m}` or `{"timeline": "t1", "frame": n}`, `"x": 960.5, "y": 540.5, "size"?: 1, "grade"?: target, "node"?: "n1", "clip"?: "c1"` | `{"frame", "x", "y", "size", "width", "height", "region", "rgb": [r, g, b], "node"?: {"grade", "node", "input", "output", "key"}}` (see Samples and the pointer) |
| `view.pointer` | `{"monitor": m, "on": true \| false, "cursor"?: "arrow" \| "crosshair" \| "move" \| "resize_ew" \| "resize_ns"}` | `{"monitor", "on", "cursor"}`, then `view.pointer` notifications (see Samples and the pointer) |
| `window.bind` | `{monitor:"timeline",revision,timeline,target,clip?,node,selected,visible,outline,tool}` | `{binding,sequence}`; existing Window controls of every shape (see Circular and Gradient Window controls, Linear and Polygon Window controls, Curve Window controls) |
| `window.input` | `{binding,kind:"down"\|"move"\|"up"\|"cancel"\|"delete"\|"clear_points"\|"delete_points",event?,capture?}` | Down returns `arming` immediately; `window.state` reports the asynchronous result. `delete` names a native middle-button down and answers `{binding,phase:"deleted",selected,point,revision}`; `clear_points` and `delete_points` (Esc and Backspace on a Curve's point selection) answer `{binding,phase:"cleared",cleared}` and `{binding,phase:"deleted",selected,points,revision}` (see Curve Window controls) |
| `window.unbind` | `{binding}` | Ends capture and clears this connection's Window selection, or releases its still-owned retired binding's pointer |
| `track.start` | `{grade,node,window,revision,frame,direction,tracker?,components?}` | `{job,first,last,frame_size,direction}` at once, then `track.progress` and `track.state` (see Window tracking) |
| `track.stop` | `{job}` | `{}`; the track ends between frames, keeping what it tracked |
| `track.edit` | `{grade,node,window,revision?,edit:{kind,...}}` | `{revision}`; one step to undo (see Window tracking) |
| `render.start` | `{"timeline":"t1", "path":"<file>", "codec":"h264", "quality":"high", "range"?:{"from":0,"to":250}, "subtitle_tracks"?:["S1"], "replace"?:false, "revision"?:n, "audio"?:{"bus"?, "rate"?, "codec"?, "bitrate"?, "normalize"?}}`, or with `"video":null` and no `codec` or `quality` for the sound alone | `{"job": "render-1"}` (see Rendering a timeline, and The sound of an export) |
| `render.cancel` | `{"job": "render-1"}` | `{}` |
| `audio.meters` | `{"timeline": "t1", "reset"?: false}` | what the meters of the timeline show now: `{"timeline", "playing", "rate": 30, "scale", "strips": [...], "control_room", "loudness": {...}, "lost"}` (see Meters, loudness and the analysis of sound) |
| `audio.loudness` | `{"timeline": "t1", "action"?: "start" \| "pause" \| "reset"}` | the loudness meter after the action: `{"running", "standard", "target", "momentary", ...}` |
| `audio.standards` | none | `{"standards": [{"id", "name", "target", "tolerance", "true_peak", "measure", "dialogue", "resolve", "official", "verified", "source"}, ...]}` |
| `audio.analyze` | `{"timeline": "t1", "targets": [{"clip": "c1"} \| {"track": "A1"} \| {"bus": "B1"}, ...], "range"?: {"from": 0, "to": 250}, "standard"?: "ebu-r128", "revision"?: n}` | `{"job": "audio-1"}` at once, then `audio.analysis` |
| `audio.normalize` | `{"timeline": "t1", "clips": ["c1", ...], "level"?: "sample_peak" \| "true_peak" \| "<standard>", "target"?: -9, "mode"?: "relative" \| "independent", "apply"?: false, "revision"?: n}` | `{"job": "audio-2"}` at once, then `audio.analysis` with the gains (Normalize Audio Levels) |
| `audio.level_match` | `{"timeline": "t1", "reference": "c1", "clips": ["c2", ...], "apply"?: false, "revision"?: n}` | `{"job": "audio-3"}` at once, then `audio.analysis` with the gains (Level Matcher) |
| `audio.cancel` | `{"job": "audio-1"}` | `{}`; the job ends `cancelled` |
| `transport.frame_digest` | `{"monitor": m}` | test hook only, with `--test-hooks` (see The transport); `-32601` otherwise |
| `transport.*` | see The transport | |
| `automation.record` | `{"timeline": "t1", "on": true, "glide_ms"?: 250}` | `{"on": true, "glide_ms": 250, "pending": 0, "read": false}` (see Recording automation) |
| `automation.touch` | `{"timeline": "t1", "track" \| "bus" \| "vca": "A1", "path": "fader_db", "value": -6.0, "touching": true}` | `{}`, at once (see Recording automation) |
| `automation.punch` | `{"timeline": "t1"}` | `{"lanes": n}` (see Recording automation) |
| `automation.all_read` | `{"timeline": "t1"}` | `{"read": bool}` (All Read, T16b; see Recording automation) |
| `automation.join` | `{"timeline": "t1"}` | `{"lanes": n, "at": time \| null}` (Join Mix, T16b; see Recording automation) |
| `automation.fill` | `{"timeline": "t1", "from": time, "to": time, "glide"?: false}` | `{"lanes": n}` (Fill Range, Glide Range, T16b; see Recording automation) |
| `automation.copy` | `{"timeline": "t1", "of": [{"track" \| "bus" \| "vca": id}], "from"?: time, "to"?: time, "at"?: time}` | `{"lanes": [{of, path, points?, value?}]}` (Copy of automation, T16b; see Recording automation) |
| `scopes.subscribe` | `{"view": "timeline" \| "source", "settings": {"kind": "waveform", ...}}` | `{"scope_id": "scope-1", "size": {"width": 512, "height": 256}, "graticule": {...}, "settings": {...}}`, then `scopes.frame` (see Scopes) |
| `scopes.set` | `{"scope_id": "scope-1", "settings": {...}}` | as `scopes.subscribe` |
| `scopes.unsubscribe` | `{"scope_id": "scope-1"}` | `{}` |
| `scopes.statistics` | `{"view": "timeline" \| "source", "bins"?: 256}`, or `{"timeline": "t1", "frame": n, "bins"?}`; either with `"grade": target, "node": "n1", "at"?: "input" \| "output", "clip"?: "c1"` | the figures of the frame shown, of a frame drawn off screen, or of the picture at a node (see Scopes) |
| `luts.list` | none, or `{"folder"?: "Film"}` | `{"roots": [...], "luts": [...], "truncated": false}` (see The library of LUTs) |
| `luts.preview` | `{"luts": ["Film/Look.cube", ...], "height"?: 72, "frame"?: {"timeline": "t1", "frame": 0, "clip"?: "c1", "grade": target, "node": "n1"}}` | `{"previews": [{"lut", "width", "height", "png": "<base64>"} \| {"lut", "error"}, ...]}` (see Previews of LUTs) |
| `grade.export_lut` | `{"grade": target, "node"?: "n1", "size": 17 \| 33 \| 65, "path": "<name or file>.cube", "frame"?: n, "replace"?: false}` | `{"path", "name", "size", "nodes": [...], "ignored": [{"node", "why"}]}` (see Generate LUT) |
| `viewer.wipe` | `{"on"?: true, "style"?: "horizontal", "position"?: 0.5, ..., "reference": {"kind": "still", "path": "..."}}`, or `{"on": false}` | what the timeline monitor compares with (see Comparisons in the viewer) |
| `viewer.split` | `{"on"?: true, "items": [{"kind": "current"}, ...], "selected"?: 0, "outline"?: true}`, or `{"on": false}` | as `viewer.wipe` |
| `still.grab` | none, or `{"view"?: "timeline", "album"?: "a1", "clip"?: "c3", "level"?: "clip" \| "pre_clip" \| "post_clip", "format"?: "auto" \| "png" \| "exr"}` | the still (see The Gallery) |
| `still.get` | `{"still": "s1"}` | all the still holds, its grades too, and its `album` |
| `still.apply` | `{"still": "s1", "to": [target, ...], "mode"?: "apply" \| "append", "keyframes"?: "none" \| "source_timecode" \| "start_frame", "preserve_nodes"?: 0, "revision"?: n}` | as `transaction.apply` |
| `still.label` | `{"still": "s1", "label": "Day exterior"}` | `{"id", "label"}` |
| `still.delete` | `{"stills": ["s1", ...]}` | `{"deleted": [...]}` |
| `still.restore` | `{"stills": ["s1", ...]}` | `{"restored": {"s1": "a1", ...}}` |
| `still.copy` | `{"stills": ["s1", ...], "album": "a3"}` | `{"copied": {"s1": "s9", ...}}` |
| `still.move` | `{"stills": ["s1", ...], "album": "pa1"}` | `{"moved": {"s1": "ps4", ...}}` |
| `still.export` | `{"stills": ["s1", ...], "dir": "<folder>", "prefix"?: "Show", "replace"?: false}` | `{"written": [{"image", "json"}, ...]}` |
| `still.import` | `{"paths": ["<file>", ...], "album"?: "a1"}` | `{"added": [still, ...]}` |
| `gallery.list` | none, or `{"album"?: "a1"}` | `{"albums": [{"id", "name", "kind", "stills": [still, ...]}, ...], "memories": album \| null}` |
| `gallery.thumbnail` | `{"still": "s1", "height"?: 90}` | `{"width": 160, "height": 90, "png": "<base64>"}` |
| `gallery.album.create` | none, or `{"kind"?: "stills" \| "powergrade" \| "memories", "name"?: "Looks"}` | `{"id", "name", "kind"}` |
| `gallery.album.rename` | `{"album": "a1", "name": "Looks"}` | `{"id", "name"}` |
| `gallery.album.delete` | `{"album": "a2"}` | `{"deleted": [still ids]}` |
| `grade.append_node` | `{"from": target, "node": "n4", "to": [target, ...], "keyframes"?: "none" \| "source_timecode" \| "start_frame", "at"?: 120, "revision"?: n}` | as `transaction.apply` (Append Node to Selected Clips, see The Gallery) |
| `viewer.highlight` | `{"mode": "off" \| "highlight" \| "bw", "grade"?: target, "node"?: "n1", "monitor"?: "timeline"}` | `{"monitor": "timeline", "mode", "grade"?, "node"?}` (see Highlight) |
| `viewer.bypass` | `{"on": true, "monitor"?: "timeline"}` | `{"monitor": "timeline", "on"}` (see Bypass Color) |
| `viewer.preview` | `{"grade": target, "node": "n1", "lut": "Film/Look.cube", "frame"?: n, "monitor"?: "timeline"}`, or `{"on": false}` | `{"monitor": "timeline", "preview": {...} \| null}` (see Live Preview and the Color Viewer LUT) |
| `viewer.lut` | `{"on": true \| false, "monitor"?: "timeline"}` | `{"monitor": "timeline", "on"}` (see Live Preview and the Color Viewer LUT) |
| `timeline.thumbnail` | `{"timeline": "t1", "frame": 0, "clip"?: "c1", "height"?: 90}` | `{"width": 160, "height": 90, "png": "<base64>", "frame"}` (see Thumbnails of the Color page) |
| `window_presets.list` | none, or `{}` | `{"presets": [{"id": "wp1", "name": "Face", "windows": [window, ...]}, ...]}` (see Window presets) |
| `window_presets.save` | `{"name": "Face", "grade": target, "node": "n4", "frame": 120}` | the preset `{"id", "name", "windows"}` (Save as New Preset) |
| `window_presets.update` | `{"preset": "wp1", "grade": target, "node": "n4", "frame": 120}` | the preset (Update Preset) |
| `window_presets.delete` | `{"preset": "wp1"}` | `{"deleted": "wp1"}` (Delete Preset) |
| `window_presets.apply` | `{"preset": "wp1", "grade": target, "node": "n4", "revision"?: n}` | as `transaction.apply` (recalling a preset) |

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
  change. Omit it for commands meant to operate on the current state. An optional `gesture`
  makes it part of a drag, kept with the others of the drag as one undo step (see Gestures);
  without it, a transaction is exactly as before.
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
- `view.*`: the viewer, below; `transport.*`: playing a source, below; `scopes.*`: the scopes
  of the monitors, below (Scopes); `luts.list`: below (The library of LUTs); `viewer.*`: below
  (Comparisons in the viewer); `still.*`, `gallery.*` and `grade.append_node`: below (The
  Gallery).

## Gestures

A control dragged on the Color page (a wheel, a slider, a point of a curve) changes the project
many times a second; DaVinci Resolve draws each change at once, and its undo takes the whole drag
back (pp. 3061–3062). A client sends such a drag as transactions of one **gesture**:
`transaction.apply` with `"gesture": {"id": "<1 to 64 characters>", "end"?: true}`.

- **Each is applied and drawn at once.** It is a transaction as any other: applied whole or
  refused, with its own revision and answer, saved, and drawn by the monitors as soon as they
  draw again. So every client's copy and the picture follow the drag as it goes.
- **They are one step to undo.** The first transaction of a gesture is a step of its own, and
  lets go of what could be redone, as any transaction does; each next one of the same gesture is
  put into that step (its operations after the step's, its inverses after the step's inverses).
  `history.undo` takes the whole gesture back, last first, to the project as it was before it,
  whoever asks; `history.redo` does it again, as one step. A gesture that ends with the project
  as it was before it (a drag let go where it began, or put back by Escape) leaves no step at
  all, though each of its changes was told; `next_id` does not go back.
- **What ends a gesture.** A transaction with `"end": true` (the release of the control: it is
  applied, and the gesture ends with it); anything else done to the history in between, by
  anyone: a transaction of another gesture or of none (another client's, or the same client's),
  an undo, a redo; its connection closing; or 10 s with no transaction of it. The next
  transaction of the same id then begins another gesture, another step. A change that the engine
  makes itself (a proxy set) does not end one. The ids are the connection's own: another
  connection's gesture of the same id is another gesture. A refused transaction changes nothing,
  the gesture included, but one with `"end": true` still ends its gesture.
- **Revisions.** Every transaction of a gesture makes a revision. A `revision` given with a
  transaction of the gesture that is going on is taken if it is any revision since the one before
  the gesture began (only the gesture has changed the project since), so a client may send each
  change of a drag at the revision that its copy has, however far behind the gesture its copy is.
  Once another change has ended the gesture, such a `revision` is stale (`-32001`, `reason:
  "stale_revision"`, with the revision of the project in `actual`), as it would be without a
  gesture: the client fetches the project or takes in the change, and begins again.
- **Notifications.** The first change of a gesture is told at once (unless a change was told
  less than 100 ms before); each next one at most every 100 ms (10 a second): what is not told
  yet is held, and told as one `project.changed` of the gesture's `by` and `made_by`, `done`
  every operation held in order, `revision` the last, and `base` the revision that it applies to
  (left out where that is `revision − 1`). What is held is told once 100 ms have gone by since a
  change was last told, as the gesture ends, as another gesture begins, and before anything else
  of the project is told or answered (`hello`, `project.get`, an undo, another transaction). The
  client that sends the gesture may therefore be answered before it is told; it is told in time.
- A client that sends no `gesture` (the Python client, a command) is answered and told exactly
  as before.

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
- **Streams of sound.** Without `stream` the waveform is of the media's sound: its best stream
  of sound, as libav ranks them, in the channels that `facts.audio` counts, laid out by
  swresample in the default layout of libav for that many channels. That is the stream's own
  order where its layout is that default (mono, stereo, 5.1), but not where it is another:
  3.0 (L R C) is mixed into 2.1 (L R LFE), its C into the left and right. A waveform of each
  channel as it is (for the lanes of a track, or to pick a channel) passes `stream`. `stream`
  asks for a stream of the file by its index among all the streams of the file, as `ffprobe`
  numbers them (the sound of a file with a picture is usually stream 1): its channels are given
  as the stream has them, in the order of the layout that the stream says it has, none mixed
  into another, as a clip's channel configuration picks them (RM20 pp. 3833-3835). Its time
  zero is that of the media's sound (the first video frame, or the start of the best stream of
  sound), so a stream that starts later is silent until it starts. An index that is not a
  stream of sound the engine can decode is `-32602`, `data: {"reason":"stream", "stream":n}`.
  The cache of a stream's waveform is read first, by the channels that its header says, without
  opening the file; a file that cannot be opened and has no cache is answered as for the sound
  of the media: pending, then the notification, then the error.
- **Background work.** An uncached waveform returns `{"pending":true}` and runs on a separate
  single-worker queue, leaving the thumbnail queue available. Repeated requests for the same
  pending source share the job. Completion sends `media.waveform` with `{"media":"m1"}` (and
  the `stream` asked for, if one was) to all clients that said hello, provided the same source is still in the project. Query again
  after this notification to receive the data or a decode error; the notification does not
  assert success. An error is consumed by the next query, and a further query can retry (for
  example after an offline file is restored). Shutdown cancels queued jobs and stops decoding
  at the next audio block.
- **Caches.** PNG files live in `cache/thumbs/<media>-<digest12>-<frame>-<height>.png`;
  waveforms in `cache/waveforms/<media>-<digest12>.bin`, and that of a stream in
  `<media>-<digest12>-s<stream>.bin`. The digest is the existing imported
  identity, not a newly computed hash. Punctuation in media ids is percent-encoded in filenames.
  Files are replaced atomically. Malformed caches are rebuilt; a cache that cannot be written
  does not prevent a computed answer. Valid caches can be read while the source is offline,
  including after restart. A redirected cache directory or a file symlink is not read through.
  Waveform files start with `CWAV1`, a little-endian u16 channel count, then the peak bytes.
- **Size limits.** Thumbnail width must not exceed 16384 pixels (`-32602`). Encoded answers
  must fit the 16 MiB protocol line limit, leaving 1024 bytes for JSON; an oversized waveform
  or PNG returns `-32002`, reason `too_large`. Waveform size is checked before allocation;
  PNG size is checked before base64 response encoding.

### Thumbnails of the Color page

**`timeline.thumbnail`** `{"timeline": "t1", "frame": 0, "clip"?: "c1", "height"?: 90}` gives a
small picture of a frame of a timeline as it is graded: the thumbnails of the Color page "always
show the media as it's currently graded" (p. 3026), where `media.thumbnail` gives a frame of
media as its file has it. It answers `{"width", "height", "png": "<base64>", "frame"}`, as
`media.thumbnail` does: an upright PNG of R'G'B'A of 8 bits, opaque, `height` pixels high (1 to
270, 90 where left out; no higher than the timeline), as wide as the timeline's aspect makes it.

- The frame is drawn off screen as `view.sample` and `scopes.statistics` of a timeline and frame
  draw theirs (its pictures from the original media, through their input transforms, graded,
  and through the output transform; no subtitles), at twice the height asked for (or the
  timeline's height, where that is less, its grades' neighbourhoods and titles scaled with it),
  and made `height` high by the mean of the pixels; its values, the output signal, kept to 0..1.
- With **`clip`**, the layer of that clip alone, fitted to the frame and opaque, by its own
  grades (its group's pre-clip grade, its own, its group's post-clip grade) and not the
  timeline's: the clip's picture as it is graded, whatever its place on the canvas, and on a
  track that is off too (the thumbnail timeline shows every clip); only that clip's picture is
  read. A clip that is itself off is not seen. Without it, the whole frame as an export draws it.
- It changes nothing and works in a headless engine. `-32602` for params that are not the
  method's, with `data` `{"reason": "timeline"}` for a timeline that is not there, `{"reason":
  "clip"}` for a clip that is not seen at the frame, and `{"reason": "height"}` for a height out
  of its range; `-32004` with `{"reason": "draw"}` where the card could not draw it.

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

### Schema 4

The top level is `{schema: 4, next_id, bins: [...], media: [...], timelines: [...], groups: [...], color: {...}}`.
Schema 3 differs from schema 2 by the grades (see Grades below) and by colour management: the
project's `color` and each media item's `color_override` (see Colour management below). Schema 4
adds the sound of the Fairlight page (see Sound below). A schema 1, 2 or 3 project is upgraded
on open, a schema at a time (1→2→3→4); its original bytes are kept once in
`project.schema1.json`, `project.schema2.json` or `project.schema3.json`.
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
  and `color_override` may be left out, and `facts.video` / `facts.audio` may be `null` or left
  out.
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

### Sound (schema 4)

What schema 4 adds (`crates/model/src/audio.rs`, `audio/fx.rs`, `audio/routing.rs`,
`audio/param.rs`). The engine always writes every field; each may be left out where a project or
an operation is read, and is read as its default (a new timeline's, as DaVinci Resolve makes it;
Reference Manual 20 pages in brackets). Levels in decibels of a fader, a send, a VCA and a boom
are at most +10; −100 or below is −∞.

- **Clip.** `gain_db` (−96…24) and `pan` (−1…1) are each a number, as before, or
  `{"keys": [{"at": {ticks, scale}, "value", "interp": "linear"|"ease_in"|"ease_out"}, ...]}`:
  at least one key, in strictly increasing time; `at` is a time of the clip's media from its
  start (it stays on its sound when the clip is trimmed or moved); before the first key the value
  is the first's, after the last the last's. A key that eases in is reached, and one that eases
  out left, with a slope of 0 (cubic Hermite); otherwise straight lines (p. 3838). Added:
  `audio_fade_in`, `audio_fade_out` (`null`, or `{length: {ticks, scale}, curve, skew}`; `null`
  is a straight fade over `fade_in`/`fade_out` frames; `curve`, `skew` ∈ [−1, 1]: rise
  `g(t') = (1−|c|)·t' + |c|·e(t')`, `t' = t^(2^−skew)`, `e = sin(πt'/2)` for c > 0 and
  `1 − cos(πt'/2)` for c < 0; a fade out is `g(1−t)`; the two together no longer than the clip),
  `eq` (4 bands, see below; off), `effects` (inserts, see below, any number, in order: they act
  after the clip's equalizer and before its volume, fades and pan, their latency made up so that
  the clip still sounds on its own samples; an effect of a clip has no sidechain and no lanes, and
  one that cannot work on the channels of the track, a Stereo Width on a mono track, is
  skipped), `pitch_correction` (false), `pitch` (`{semitones: −12…12 whole, cents: −100…100}`)
  and `channels`: `null` (the media's channels mixed to the track's as libav mixes them) or one
  `{source, enabled, mute, level_db}` per channel of its track (p. 3833); `source` is `null`
  (silence), `{stream, channel}` (a stream as libav/ffprobe number all streams of the file, and a
  channel of it in its own order) or a list of them, whose mean is played (a stereo pair made
  mono is `0.5·(L+R)`); a stream or channel that the media has not plays silence; `level_db` is at
  most 24. A blade leaves no fade of sound at its cut (the first half has no `audio_fade_out`, the
  second no `audio_fade_in`); a blade or a trim shortens a fade of sound that the clip no longer
  holds, the one at the edge that moves first, exactly where a scale of 32 bits holds the new
  length and otherwise rounded down in its own scale.
- **Track of sound.** Beside `clips` and `transitions`: `format` (`mono|stereo|5.1|adaptive`,
  stereo), `channels` (as many as the format has; 1–24 for `adaptive`, which may not leave it
  out), `order` (`smpte|film`, of 5.1), `mute`, `solo`, `solo_safe`, `adc` (true),
  `automation_arm` (the Automation Arm, p. 3906: `false` not armed, `true` armed, `"safe"` or
  `"off"`, T16b; read also as `"armed"`, `"unarmed"`; anything else is refused; see Recording
  automation), `strip` (where it is left out, at unity, out to `B1` where the timeline has it,
  and to none where it has not) and `lanes`. A track of picture or subtitles
  has none of them (refused if given). `enabled: false` is Disable Track (not read, not heard);
  `mute` is the header's button (p. 3885): not heard nor rendered. A track is heard (and
  rendered) where it is on and not muted, its sound reaches a main output through outputs and
  sends that are on and buses that are not muted, and, while any track that is on or any bus is
  soloed, it is soloed, solo-safe or feeds a soloed bus. A track that is off solos nothing. The
  sound that drives a sidechain or a Ducker is taken before the mute and the fader of its track.
- **Strip** (of a track or a bus): `{trim_db (−20…20), invert, track_fx (null or a Ducker; a bus
  has none), order (a permutation of "fx", "dynamics", "eq"; that order), inserts (≤ 6),
  inserts_on (Effects In, true), dynamics, eq (6 bands), fader_db, pan, sends (≤ 10), outputs
  (≤ 10, each bus once), vcas}`. `pan` is `{on, x, y (−1…1), rotate (±180), spread (1…99, 99),
  divergence (0…100), lfe_db (−∞, −100), lfe_on, lfe_pre}`. A send is `{bus, on (true), pre (false: after
  the fader), level_db (0), pan}`; an output `{bus, on}`.
- **Effects.** An insert is `{id, on (true), effect}`; `id` (`fx1`, `fx2`, …) is unique in its strip
  and stays with it as it moves, and an insert of a strip added without one is given the next; `effect` is `{kind, ...settings}` of `gain`,
  `limiter` (with `sidechain`), `soft_clipper`, `stereo_width`, `de_esser`, `de_hummer`,
  `noise_reduction` (with a learned `profile` or `null`, and the `band` that it works on:
  `full` (default), `rumble` (below 250 Hz), `hiss` (above 4 kHz) or `rumble_and_hiss`, as its
  presets set it; it has no lane) or `pitch`. A Ducker is `{on, sources:
  [{"track": id} | {"bus": id}], amount_db (2.7), lookahead_ms (15), rise_ms (10), hold_ms (150),
  recovery_ms (750)}`. Dynamics is `{on, expander: {on, mode: expander|gate, ...}, compressor:
  {on, ..., sidechain: {on, listen, source}}, limiter: {on, ...}, makeup_db}`. An equalizer is
  `{on, bands: [{on, kind, freq, gain_db, q, slope, range}], gain_db}`; kinds
  `low_shelf|bell|high_shelf|high_pass|low_pass|notch`, ranges of the middle bands of a strip
  `low (20–200 Hz)|medium_low (100 Hz–1 kHz)|medium_high (500 Hz–5 kHz)|high (2–20 kHz)`. The
  defaults and ranges of every setting are in `crates/model/src/audio/fx.rs`; a setting out of its
  range is `out_of_range` naming what it is of.
- **Timeline.** Beside `grade`: `buses` (`[{id: "B1", name: "Bus 1", color, format: "stereo",
  strip, lanes}]`), `vcas`, `track_groups` and `audio`. A bus is `{id, name, color, format
  (mono|stereo|3.0|4.0|5.0|5.1|7.0|7.1), strip, mute, solo, solo_safe, automation_arm, lanes}`; its strip's outputs
  are the buses it feeds, and a bus that outputs to none is a main output. A VCA is `{id, name, fader_db,
  mute, solo, vcas, automation_arm, lanes}` (members name it in their strip's `vcas`; a VCA may be a member of
  another). `automation_arm` of a bus or a VCA is as a track's (armed, it is recorded under
  Touch Off, p. 3906; `"safe"`, `"off"`); `false` where it is left out, and so left out where
  it is `false`. A track group is `{id, name, color, members (tracks of sound), linked
  (editing|fader|solo|mute|sends|panning|automation; all), active (true)}`. `audio` is
  `{monitor ("B1"), meter (ppm|vu|custom), custom_meter, prefader_metering (false), loudness:
  {standard (ebu_r128), target (−23, −36…−10), scale (ebu9), relative (true), lock_to_transport
  (true)}, exclusive_solo (false), follows_edit (true; see Edits), record: {mode, touch, on_stop,
  categories, preview, active_range}}`. At most 64 buses and 64 VCAs; no path of buses longer
  than 6, nor chain of VCAs; no loop through outputs, sends, sidechains and duckers.
- **Lane** (automation of a track, bus or VCA, by the path of its setting): `{points: [{at:
  {ticks, scale}, value}], kind: continuous|switch, scale: linear|log}`, points in time order,
  all of one scale and within 0…2^53−1 ticks (two may share a time: the later holds from it),
  times of the timeline; straight lines between points (worked out in ticks), a switch holds;
  before the first and after the last, their values. `scale` is what a continuous lane goes
  straight on (the decision of T14's review, C-A7): `log` for a frequency (a path whose last part
  is `freq` or ends in `frequency_hz`) and a Q (`q`), where the value at a share `s` of the way
  from `a` to `b` is `exp(ln a + (ln b − ln a)·s)` (100 Hz to 10 kHz passes 1 kHz half way), and
  `linear` for everything else (decibels are straight in decibels, a pan in its units). It is set
  by the path: a lane read or set without it is given the one of its path, the engine always
  writes it, and one given another is refused. The path of a lane is the path of the field of the
  strip that it automates, and its kind is that of the field: a number is continuous, a boolean a
  switch. Paths: `fader_db`, `trim_db`, `mute`, `invert`, `pan.<x|y|divergence>`, `eq.on`,
  `eq.gain_db`, `eq.<band from 0>.<on|freq|gain_db|q>`, `dynamics.<…>` as Dynamics is written
  (`dynamics.compressor.threshold_db`, `dynamics.compressor.sidechain.on`, `dynamics.makeup_db`),
  `track_fx.<…>` of a track with a Ducker (`track_fx.amount_db`, `track_fx.on`),
  `send.<bus id>.<level_db|on|pan.x|pan.y>`, `output.<bus id>.on`, and
  `insert.<insert id>.<on|setting>` (`insert.fx1.ceiling_db`). A VCA has `fader_db` and `mute`.
  No other field has a lane: a kind, a mode, a source or any `listen` (only heard while one
  listens) is refused as `out_of_range: lane`; a whole number (`eq.<band>.slope`, the
  `semitones` of a Pitch) as `out_of_range: lane_integer`; and what the graph does not move as a
  lane would, as `out_of_range: lane_not_wired`: Effects In (`inserts_on`), the pan's `on`,
  `rotate`, `spread`, `lfe_db`, `lfe_on` and `lfe_pre`, a send's `pre` and the rest of its pan, and
  the Lookahead of a Ducker (`track_fx.lookahead_ms`, the latency of the graph). A Pitch has lanes
  of its `cents`, `dry_wet` and `on`, which act on the sound that leaves it (read its latency
  later); its `semitones` move in whole steps and have none. With Effects In off, a lane of the switch of an insert
  (`insert.<id>.on`) turns no insert on. A lane keeps to the range of its setting: each of its
  values is one that the setting may be set to while the rest of the strip is as it is (a fader at
  most +10 dB, a pan −1…1, a frequency within its band, above 0 on a scale of logarithms), and a
  switch is 0 or 1; a lane of a setting that the strip has not, of another kind or scale, or out
  of that range is refused as `out_of_range: lane`. A project read with such a lane is refused
  (none was written before these checks). The engine reads a lane at each sample of the timeline
  exactly as written here: the times are compared in ticks, a point whose time falls between two
  samples acts from the first sample at or after it, and the share of the way between two points
  is worked in doubles (T14).
- **Transition.** Added `fade_in_level_db` and `fade_out_level_db`, each −3, 0 or 3 (0): how the
  clip that starts at the cut rises and the one that ends there falls on a crossfade: 0 dB `t`,
  +3 dB `sin(πt/2)`, −3 dB `t^1.5`; the falling side is the rising curve reversed.
- **Media.** `facts.audio` may have `layout` (libav's name of its layout) and `streams: [{index,
  channels, layout}]`; both are left out where they were not read.
- **Streams read later (T3c).** Media of sound whose `streams` were not read (probed before
  schema 4 read them, or added so by a client) has them read by the engine, from its file, on a
  thread of its own, at once as the project is opened and then every 10 s while the engine runs:
  so media whose file is not there (offline, its drive away) is read once it is back. A file that
  is there but is another file (its `identity.digest` is not the media's) or one that libav
  cannot open is left as it is, and read again only once it changes (its size or the time it was
  modified). What is read is set by `set_media_streams` (`layout` the layout of its best stream,
  `streams` all its streams of sound), the media of a pass in one change of the engine, as a
  proxy that it made is set: no step of the history (an undo never takes it back, and what could
  be redone is kept), told to everyone as `project.changed` with `by` and `made_by` `"engine"`,
  and saved as every change is (the engine saves by itself; there is nothing for a client to
  save, and nothing to ask). Clients that keep a copy apply it as any `done`; a client that shows
  the channels of media (Clip Attributes) shows them from then on.

A schema 3 project is upgraded so that it sounds exactly as it did: each timeline gets bus `B1`,
monitored, which every track of sound outputs to at unity through a strip that changes nothing;
every track of sound is stereo with its clips' channels mixed as before; every crossfade gets
+3 dB on both sides (equal power, as before; a new one is 0 dB); an audio track that was
`enabled: false` becomes `enabled: true, mute: true`. A schema 3 project that has any field of
schema 4 is refused.

## The view

The engine draws the picture of the viewer itself, with the graphics card, into a native child
window that it makes inside the desktop's window, over the rectangle that the page keeps for
the viewer (`crates/view`). No frame crosses between the processes; the page never sees the
picture.

- **Monitors.** Every `view.*` request but `view.display` must name `monitor: "source"` or
  `"timeline"`. There is one independent native window for each monitor. Missing, null or other
  monitor values are refused (`-32602`); protocol 2 does not default to the source monitor.
- **Who sends them.** Only the desktop (`hello` with `"client": "desktop"`) may call `view.*`;
  another client is refused with `-32600`. Within the desktop, only the **Electron main
  process** sends them: it owns the window and its handle. The page (the renderer) never gets
  the handle and never calls the engine for the view: it tells the main process where the
  viewer is (over IPC), and the main process sends `view.place`. A `--headless` engine draws in
  no window: every `view.*` is answered `-32601`. `view.sample`, `view.pointer`, `view.display`, and `view.viewport` are the
  exceptions: they touch no window of the desktop's, so the page calls them (through the main
  process, which lets them by), and any client may call `view.sample` (below, Samples and the
  pointer).
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
- **`view.viewport`** changes the picture geometry inside one monitor, independently of
  `view.place` and of project transforms. The default is `mode: "fit"`, keeping the existing
  integer letterbox layout. With `mode: "scale"`, `scale` is physical device pixels per pixel
  of the oriented picture (1 means 100%), and `center` is the normalized picture point at the
  centre of the surface. Its coordinates may lie outside 0..1. The source picture's metadata
  quarter-turn is applied before this map; a timeline canvas is already oriented. Areas outside
  the picture are black, and a magnified picture is clipped to the native surface.
  `{monitor}` queries the setting; `mode: "fit"` accepts no scale or center, and `mode: "scale"`
  requires both. Unknown fields, nulls, nonfinite numbers and invalid numeric geometry are
  `-32602`; values are never silently clamped. Scale must remain a positive normal f32, and
  destination extents for every supported 1..16384-pixel picture/surface must be in
  [2^-126, 2^126], with finite f32 edges. These bounds keep shader division defined, including
  hardware that flushes subnormals to zero (WGSL 15.7.2 and 15.7.4.1).
  Settings can be read or changed before attach and survive detach/replacement/reattach for
  this engine's lifetime. Source and timeline settings are independent; they are not stored
  in the project, do not change revision/history, and do not survive engine restart. The
  response reports the **requested** setting; drawing is asynchronous. This method is only
  for a desktop client in a non-headless engine. The page may call it through Electron's allowlist.
  Navigation uses the same method with an `operation` instead of `mode`:
  - `{monitor,operation:"prepare",instance?}` reads the requested setting plus the current
    native `instance` and `geometry:{surface:[w,h],picture:[w,h]}`. It requires a successful map
    matching both the actual surface size and requested native rectangle; it changes nothing.
    The latest submitted oriented picture dimensions must also match, even when that picture
    has left the pending slot and is still being drawn. A submitted clear is not a picture.
    An optional instance must match. This lets keyboard/header actions work before any mouse move.
    On Windows, prepare also returns `buttons:["left"|"right"|"middle",...]`, one current
    logical-button snapshot; unsupported platforms omit it (absence is not a release).
  - `{monitor,instance,operation:"zoom",factor,at?:{x,y}}` multiplies the latest requested
    scale. `at` is a device coordinate (native integer pointer coordinates plus 0.5); when omitted
    it is the surface centre. Fit starts with the smaller surface/picture ratio and finds its
    anchor through the existing integer Fit map's inverse, including black bars and rotation.
  - `{monitor,instance,operation:"pan",base,geometry,offset:{x,y}}` applies total device-pixel
    displacement from down to the prepared `base` (`{mode:"fit"}` or `{mode:"scale",scale,center}`).
    The original surface/picture dimensions must still match; changing either ends the old pan.
    A normal new frame with the same dimensions does not end it. The client checks prepare's
    dimensions against those of the native down, retains the final position even if Up arrives
    before prepare, and sends no mutation for an unmoved middle click.
  - `{monitor,instance,operation:"scale",scale}` selects a percentage while preserving the
    requested normalized centre; `{monitor,instance,operation:"fit"}` restores centred Fit.
  All mutations resolve and publish in the monitor's view-operation order against the latest
  requested setting, so navigation accumulates while presentation is pending. Every navigation
  reply describes an accepted request, not a completed present. Replaced/missing instances fail
  with `-32003`, reason `instance`; invalid/stale geometry is `-32602`, reason `viewport`.
  The old absolute/query forms still work without a window or picture.
  Color's project defaults are 1.1 per 120 wheel units (fractional units retained), pointer-centred
  wheel zoom, centre-anchored Ctrl+=/-; Alt+Shift+Z is 100%, Shift+Z is Fit. Its sole input owner
  keeps basic navigation active with OSC Off/hidden; Window obtains the pointer only via bind,
  and Wipe/Split alone receive comparison input. While left is physically held navigation is
  ignored; after native Up it waits for that capture's normal ending. A MiddleDown after LeftUp
  may wait for the same capture to finish
  normally while retaining the new pan's Move/Up positions. Middle pan excludes other
  navigation until release. These operations never write project/history or resume playback.
  Keyboard/header intentions pin any instance already known when queued. After a tool handoff,
  an inherited held bit without a current capture/pan may be confirmed once by read-only prepare
  on a navigation intention: still-held or unavailable state discards that intention. A changed
  owner, instance, or pointer event during the query discards the answer; it cannot clear a new
  press. This also handles a release outside the child after the old capture was relinquished.
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
- **`view.display`** is the setting "Display HDR On Viewers If Available" of the viewer's menu
  (RM20 p244), for both monitors: `hdr` allows the views to show HDR where Windows has HDR on
  for the monitor that their window is on, or keeps them to SDR. It is **allowed** unless sent
  `false`; it is the engine's, kept until it ends, and given to each view attached since.
  Without `hdr` it changes nothing. Either way it says the setting, and for each monitor what
  its view shows as it last drew (`null` with no view, or before its first frame): `hdr_on`
  whether Windows has HDR on for the monitor of its window, `hdr_shown` whether the view shows
  HDR (`hdr_on` and allowed), and the monitor's `peak_nits` and `sdr_white_nits` (the
  brightness of SDR content in the settings of Windows), each `null` where the monitor does not
  say. The engine reads the monitor through DXGI (`IDXGIOutput6::GetDesc1`, and
  `DisplayConfigGetDeviceInfo` for the white of SDR) when the view is made and, while HDR is
  allowed, at most once a second, so a window moved to another monitor, or HDR or the
  brightness turned in the settings of Windows, is followed within a second.
  - **What is shown in HDR.** The surface is one of half floats in scRGB (linear BT.709, 1.0 =
    80 nits). A picture of SDR, which is every picture for now, is put with its white on the
    white of SDR of Windows, through the sRGB curve, as Windows shows SDR: it looks as it does
    with HDR off. HDR content (PQ or HLG, once the project's colour management says what a
    picture holds) is shown in nits, rolled off to the monitor's peak. With HDR off or not
    allowed, an SDR picture is drawn exactly as before, bit for bit.
  - The setting has no other effect: the scopes, `transport.frame_digest` and exports read the
    picture before it is shown, and are the same either way.
- **Failures.** A view whose window was destroyed with its parent (the window closed) is let
  go: `view.place` and `view.show` are then answered `-32003` with `reason: "gone"`, and the
  desktop attaches again to its new window. If drawing a frame takes more than 1 s (the card
  hangs, a driver resets), the view hides itself, so that the page is seen rather than a frozen
  picture, and shows itself again once the frame returns. The engine has one graphics card,
  made when the first view is attached (`gpu_ready`: its adapter, its API, whether it is the
  system's in software, and `compiler`, what compiles its shaders, with `compiler_path`), which
  the view draws with. On Windows the shaders are compiled by Microsoft's DXC where
  `dxcompiler.dll` and `dxil.dll` lie beside the engine (as the package of the desktop brings
  them), else by FXC; `COLUMBINA_DX12_COMPILER` (`auto`, `fxc`, or the path of a
  `dxcompiler.dll`) names another. The engine says which at its start (`shader_compiler`:
  `compiler`, `path`, `by`); a DXC with which no device can be made is given up for FXC for as
  long as the engine runs (`shader_compiler_fallback`, with why). A device that is lost
  (`gpu_lost`) is made again, a second apart until it can be (`gpu_remaking`), and the view
  draws again on it. For
  10 s after a card was lost only a card is taken, since a driver that resets brings it back; then
  the software adapter is taken if there is no card, and a card is looked for every 30 s while the
  engine draws in software (`gpu_upgraded` once one is taken). A surface that is lost is made
  again. These are logged on stderr (`view_ready`, `view_stalled`, `view_recovered`,
  `view_device_lost`, `view_surface_lost`, `view_error`, `gpu_error`), with
  `view_attached` and `view_detached`; `view_display` says what a view shows each time it
  changes (`hdr_on`, `hdr_shown`, `peak_nits`, `sdr_white_nits`, rounded), and
  `view_hdr_allowed` each time `view.display` changes the setting. `view_ready` says the
  engine's `dpi_awareness`: `per_monitor_v2`, or why not, in which case the rectangles will be
  off.
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

### Samples and the pointer

The pickers of the Color page (the black and white points of Lift and Gain, the samples of the
curves and of the HSL curves, p. 3122), the line of a wipe dragged on the viewer, and the windows
of a node drawn on it are tools used on the picture; but the picture is the engine's own window,
and the page is told nothing of what is done on it. So the engine reads the picture for the page,
and tells it what the pointer does there.

**`view.sample`** `{"monitor": m}` or `{"timeline": "t1", "frame": n}`, `"x"`, `"y"`, `"size"?`
gives the colour of a point of a picture: the mean R'G'B' of the square of `size` pixels (an odd
number from 1 to 31, 1 unless given) about the pixel under `(x, y)`, cut by the edges of the
frame. `x` and `y` are pixels of the frame from its top left corner, a pixel's centre at `.5`, as
`view.pointer` tells them: of the timeline for the timeline monitor (its width and height), of
the picture as it is shown (turned as its media says) for the source monitor.

- With `monitor`, it is what that monitor shows now, as its scopes measure it (the comparison
  with its reference, where a wipe or a split screen is on), before the view maps it to its
  display: an engine with no monitor (`--headless`) answers `-32601`, and one whose monitor shows
  nothing `-32004` `not_loaded`. With `timeline` and `frame`, the frame is drawn off screen as an
  export draws it (graded, without subtitles), in a headless engine too.
- With `grade` (a grade target, as the operations on grades name it, see Grades) and `node`, it
  also gives the colour that goes into the node at that point and the colour that comes out of it,
  in the working space of the timeline, as the node sees them (see below, The picture at a node).
  With `monitor` this is of the timeline that the timeline monitor shows, at the frame it shows;
  the source monitor shows media, ungraded, and is refused (`node`).
- Answer: `{"monitor": m` or `"timeline": "t1", "frame", "x", "y", "size", "width", "height"`
  (of the frame), `"region": {"x", "y", "width", "height"}` (the pixels averaged), `"rgb": [r,
  g, b]`, and with a node `"node": {"grade", "node", "input": [r, g, b], "output": [r, g,
  b], "key": k}}`. Values are from 0 to 1, or beyond where the picture is. `key` is the node's
  key there, 0 to 1, the mean of the square: what Highlight BW shows of the node (see
  Highlight), where only the layers of the grade are drawn (0 about them). A picker of the
  Qualifier palette samples `input`, the colour that the qualifier keys.

**The picture at a node** (`view.sample` and `scopes.statistics` with `grade` and `node`) is the
picture that goes into the node (its first picture input, `"at": "input"`) or comes out of it
(`"output"`, the default), drawn by the grade cut there: the node and everything that leads to it,
and nothing after it. The grades before it are done whole and none after it: for a node of a
clip's grade, the pre-clip grade of its group and the clip's grade up to the node; for the
pre-clip grade of a group, that grade up to the node; for its post-clip grade, the pre-clip and
the clip's grades and that grade up to the node; and for these, only the layer of the clip is
drawn (for a grade of a group, the clip shown on top that is in the group, or `clip`), where it
lies on the canvas, not laid over the others nor graded by the timeline. For a node of the
timeline's grade, every layer is drawn and laid together, and that grade done up to the node.
None has an output transform. Its key (`"at": "key"` of `scopes.statistics`, `key` of
`view.sample`) is the grade not cut, its key shown in R, G and B alike as Highlight BW shows it,
the grades after it left out as above. Refused with `-32602` and `reason`: `node` (a node half named, no
such node, a node that gives no picture (a key mixer) or takes none, one whose input has nothing
linked, a grade that cannot be cut there because a node leads to the node only by its key),
`grade` (no such group), `timeline` (the grade is of another timeline than the frame), `clip`
(the clip, or no clip of the group, is shown at that frame).

**`view.pointer`** `{"monitor": m, "on": true, "cursor"?: "arrow"}` has the view of that monitor
take the pointer, showing that cursor over itself (`crosshair` for a picker, `move`,
`resize_ew`, `resize_ns` for the handles of a wipe or a window), and tell what the pointer does
there to this connection alone; `{"monitor": m, "on": false}` lets it go. Only the desktop may
call it (`-32600` from another client; `-32601` in a headless engine), and only with a view
attached (`-32003` `no_view`); a view attached again in place of one takes the pointer as the one
before did. A connection that asks takes it from one that asked before; one that closes lets it
go. While a view does not take the pointer (as at first), the mouse goes through it to the page,
as it always did, and nothing is told.

While it takes it, the view is the target of the mouse over the picture, and the engine tells
`view.pointer` `{"monitor", "kind", "button", "buttons", "x", "y", "inside", "window", "shift",
"ctrl", "alt", "delta", "clicks", "middle_moved"}` for each press (`down`), move (`move`),
wheel (`wheel`) and release (`up`) of a button: `button` the
one pressed or released (`"left"`, `"right"`, `"middle"`, null for a move), `buttons` those held
after it, `x` and `y` where it is on the frame drawn (as `view.sample` takes them; null where no
frame is drawn), `inside` whether that is on the picture (not on the black about it), `window`
where it is in the view's window in device pixels, and the keys held. A button pressed over the
view captures the mouse until the last button is released, so a drag goes on beyond the view
(`inside` false, `x` and `y` beyond the frame); if another window takes the mouse meanwhile, the
drag ends with `cancel`. Presses and releases are each told; moves as fast as the connection
reads them (the next once the one before was written, those between dropped), and the last place
the pointer stopped at is always told, before the next press, wheel, or release. Each wheel is
told; `delta` is its signed native high-resolution delta (120 per detent, zero otherwise).
Wheel screen coordinates are converted to this child window's client coordinates before the
same pixel-centre mapping. `clicks` is 1 for an ordinary down, 2 for the system's double-click
second down (all three buttons), and 0 otherwise. `middle_moved` latches any device-pixel
movement from middle down before move coalescing, remains true when the pointer returns to
the origin, and is included on Up/Cancel before clearing. The view never takes
the keyboard: keys stay the page's.

Pointer off relinquishes capture but remembers which formerly captured buttons still need a
release. On re-enable, one physical snapshot (mapped to logical left/right) synchronizes any
release that happened while off with `up`, `clicks:0`, current `buttons`, and the last known
window point. Still-held buttons retain their pending release until a native Up or later button
snapshot; no Down or Cancel is invented and old capture is never reacquired. Physical state
updates are accepted before the desktop owner's activation reply, with the usual session,
instance and event ordering; gesture dispatch still requires the active owner/binding.

Pointer notifications additionally carry `instance`, `presentation`, and `snapshot`.
`instance` is an engine-local, increasing native attachment number; reattaching creates a
new instance even on the same monitor. Moves still held by the server are discarded on
replacement. Lines already queued to the connection retain their old instance: consumers
must check it, and must not treat an old line as input for the new window.

`presentation` is null before a successful native present or after invalidation; otherwise
it is `{serial, tag, map}`. `serial` increases for each successful presentation in this
instance. `tag` is the producer's opaque number, or null for an untagged frame. `map` is
null for a successful black clear, otherwise `{surface:[w,h], picture:[w,h], dest:[x,y,w,h]}`;
`dest` contains the same once-quantized values used by native rendering and pointer mapping.
A pending texture or viewport request does not replace this successful presentation.

`snapshot` is null for an unrecognized/invalidated/evicted tag or an untagged source-monitor
frame. A known timeline tag gives `{revision,timeline,frame,canvas:[w,h],ready,editable,comparison}`,
where `comparison` is `off`, `wipe`, or `split`. The server retains the matching immutable
project, actual source-frame provenance and final scene placements internally; no source paths,
file identities or project contents are added to pointer notifications. A candidate registration
alone is never proof of display. Each monitor/instance retains its confirmed current frame and
at most eight pending candidates; a late presentation of an evicted candidate remains unknown,
never resolved using the latest project. Unloading retires those descriptions even while the
native clear is pending. Surface/device loss requests a fresh producer redraw; native-window
destruction releases descriptions without scheduling redraws.

`ready` is false while the actual Colour differs from that project's requested Colour or
composition/Viewer LUT used provisional programs. A layer with a late cross-clip/source-frame
fallback or an inexact scrub is not editable, even though the existing preview fallback remains
visible. Split Screen is not editable. The flag describes eligibility of the current scene;
a wipe still contains a reference image, so it does not authorize edits on the reference portion.
These fields preserve the existing `x`/`y` meanings. The Window controller below additionally
uses native evidence retained inside the engine.
The TypeScript fields are optional for compatibility with older recorded notifications.

Pointer coordinates use the map of the last successful native presentation, including its
viewport. Queuing a new frame, viewport, or clear does not change that map before it is shown.
While a resized native window differs from the last presented surface size, `x`/`y` are null
and `inside` is false until a frame is successfully presented at the new size. An invalidated
surface/device also has no pointer map until presentation resumes. `inside` requires both the
picture bounds and the visible native surface bounds; captured drags outside a zoomed viewer
remain outside even if their extrapolated picture coordinates are within the picture.

### Circular and Gradient Window controls

`window.bind`, `window.input`, and `window.unbind` are desktop-only, non-headless methods for
the timeline monitor. They control an existing corrector's Circular and Gradient windows,
including movement, Circular Size/Aspect, rotation and Soft 1, and its Linear and Polygon windows
(see Linear and Polygon Window controls). They do not create nodes/windows, add keyframes, edit
Curve windows, or implement tracking.

**`window.bind`** takes the complete selection:

```json
{"monitor":"timeline","revision":12,"timeline":"t1",
 "target":{"level":"clip","timeline":"t1","clip":"c1"},
 "clip":"c1","node":"n1","selected":"w1",
 "visible":true,"outline":true,"tool":"window"}
```

`target` is the normal grade target. Clip and pre/post-group targets require `clip`, the
current clip in this timeline; a group target requires that clip's membership. A timeline
target uses the canvas and may omit `clip`. `selected` is a local window id or null. The node
must be an enabled, unlocked corrector in a complete grade. `tool` is `window`, `image_wipe`,
or `none`. The answer is `{binding,sequence}`; binding ids increase within the engine and
belong to the connection that bound them. Rebinding ends the previous capture. With visible
Window controls the native view takes the pointer for that connection, using the move cursor.
Hidden controls, outline off, and the other tools release Window pointer ownership. Image Wipe
may then use the existing `view.pointer` route for its divider. Enabling a comparison alone
does not change the selected tool or steal Window OSC.

The worker receives immutable selections. It draws enabled Circular/Gradient outlines and handles evaluated at the
actual displayed frame, after final scene selection, Highlight, comparison and Viewer LUT.
Only the selected window draws a white outline, its feather lines and handles; every other window
is a grey outline without feather lines or handles, whose outline and body still hit as Move.
Unsupported/disabled shapes have no invented handles. Split Screen suppresses the overlay.
An incompatible target, node, current clip, retained layer, revision, source provenance or
Highlight also suppresses it for that frame. The same immutable overlay is retained by the
frame snapshot and submitted native texture. `snapshot.window` is null or
`{binding,selected,points}`, `points` the selected Curve anchors that frame shows (see Curve
Window controls).

Each `view.pointer` also carries an increasing native `event` sequence. On a native left down,
the engine retains that event's connection, instance, successful presentation and exact frame
snapshot **before** sending the notification. The desktop claims it once:

```json
{"binding":2,"kind":"down","event":41}
```

Client coordinates, tags and project revisions cannot supply or replace down evidence. A down
must hit a visible control, outline or shape body inside the actual current-A picture. A soft or
bilinearly mixed A/B sample remains eligible; pure B does not hit or draw a stroke. Mix is
eligible except at pure B; Difference allows the whole picture. Blank/right clicks do not
edit or create history. Hit priority is selected first, then the last supported Window in the list. Within a window, explicit handles win;
overlapping handles choose the nearest device centre, with stable local order breaking ties.

Down returns immediately with `{binding,capture,sequence,phase:"arming",selected,event,
revision,keys}`. The server then freezes playback at the **clicked timeline frame**, waits for
the worker acknowledgment and a matching successful native presentation, and sends
`window.state` with `phase:"armed"`. Worker acknowledgment or a queued texture alone cannot
arm or write history. Arming is bounded to 15 seconds; failure sends `phase:"refused"` and
`reason`. The same RPC connection remains free to send up, Esc, unbind, or transport commands
while this is pending. Legal external transport intent cancels arming before waiting for the
transport command turn, so a late freeze cannot rewind a newer seek.

`window.state` is sent only to the binding owner, on arming, armed, refused, or ended, with:

```json
{"binding":2,"capture":1,"sequence":5,"phase":"armed","reason":null,
 "selected":"w2","event":41,"revision":12,
 "keys":{"pan":0,"tilt":3,"clock":"source","frame":10},"selected_points":[]}
```

`selected_points` are the selected window's selected Curve anchors as that state is sent (empty
for any other shape; see Curve Window controls).

`selected` is the window actually hit, including when it differs from the originally selected
row. Accept that server selection in the list **without sending another bind**. Keep the same
binding across server-owned project revisions; do not rebind on every `project.changed` or
palette interaction notice. Filter notifications by binding/capture and increasing sequence.
`keys.size`, `keys.aspect`, `keys.pan`, `keys.tilt`, `keys.rotate` and `keys.softness`
are optional: an absent field did not participate, while null names a static parameter. Each
present field independently names its captured left key frame. Move retains the original
Pan/Tilt fields. Other controls report only their own participating field. `clock` is `source`, `clip`, or `timeline`; `frame` is the clicked frame in that clock.

If a project change invalidates the bound selection (for example, its grade becomes incomplete),
the server retires that binding and sends its original owner **`window.unbound`**:

```json
{"binding":2,"sequence":8,"revision":13,"reason":"project_changed"}
```

This notification is sent even without an active capture. If a capture exists, its ended
`window.state` comes first; the clear of the worker selection is queued before `window.unbound`.
The sequence shares the increasing Window sequence. Filter by the current binding and sequence,
discard the retired binding, and re-evaluate a project snapshot at least as new as `revision`
before binding again. This event may precede the corresponding `project.changed` notification.
It may also arrive before the matching `window.bind` reply. Retain it by binding/sequence while
that reply is pending, then apply it when the reply identifies the binding; a late bind reply
must not revive a retired binding.
This is necessary even if a refetch already shows a repaired, complete grade. Valid project
changes keep the binding and send no retirement notification; do not rebind on each revision.
Only the original owner receives it. Repeated changes with no selection send no duplicate;
explicit `window.unbind` and owner replacement do not add this notification.

Retirement clears the logical selection, but the desktop must retain its old binding id long
enough to call `window.unbind` and release that Window's native pointer. The server accepts a
retired binding for this cleanup only while its exact connection/binding still owns the Window
pointer claim. Cleanup consumes that claim. A successful new Window bind or an ordinary
timeline `view.pointer` commit replaces or revokes it, including when the same connection takes
the pointer for Image Wipe. Old cleanup must not disable that successor. Source-monitor pointer
changes do not revoke the timeline claim, and failed bind/pointer requests do not consume it.
If the binding retires while its native unbind is being prepared, cleanup still completes; it
does not restore pointer capture. A rejected stale cleanup must not fall back to a global
`view.pointer` off, which could disable a newer owner.

Native moves are consumed by the server, with consecutive pending moves coalesced. The last
held move is consumed before native up/lost capture, even if a client echo wins the session
lock before the background task. The desktop may echo `{binding,capture,kind:"move",event}`
to synchronize and obtain `phase:"arming"|"armed"`; the echo supplies no position. Moves
during arming wait and cannot write. After capture begins, movement continues beyond the
picture or comparison divider using the original native map and original values.

Down copies the original **entire windows array** and independently selects the participating
parameter left key indices: the last key at/before the clicked clock frame, or the first key before all keys.
For Move, each update writes that original array with `pan = original_pan + 100*(u-down_u)` and
`tilt = original_tilt - 100*(v-down_v)`, where `(u,v)` are normalized oriented-picture
coordinates before clip placement. Other windows, keys, interpolation and Bezier handles are
preserved. The outline/hit remains at the displayed interpolated mask; numeric controls edit
the chosen left key. There is no interpolation compensation or auto-key, so between two keys
the displayed movement may be partial.

Circular uses the model's local radius r=.25, and both shapes use half feather f=Soft1/400.
Circular Size handles lie at (+/-r,+/-r), Aspect handles on the four axes, Rotate at (0,-.75r),
and Soft1 on the outer boundary at a local 30-degree ray. Gradient uses its y=0 boundary,
y=+/-f feather lines, centre Move, bottom Rotate arrow at (0,r), and Soft1 handles at (-r,-f)
and (r,f). These positions and increments are project choices, not published Resolve algorithms.
Size0 has no invertible control geometry; numeric Size restores it. Soft0 handles remain
available; a collapsed inner feather circle at Soft100 is not drawn. The feather circles and
lines are display only, like the Linear/Polygon guides: a click on one hits only what lies
under it otherwise (the body, a handle, or nothing), never the ring itself; the Soft1 handles
are the softness controls.

Drag geometry uses the actual down-frame interpolated/clamped transform E; writes use the
original left-key value L plus the observed delta, clamped to the model bounds. Size scales
about the centre; Aspect changes by +/-50*log2(axis stretch) with a positive bounded stretch.
Soft1 changes by 400 times the local normal displacement (the captured side for Gradient).
Rotation removes clip placement, measures in picture-height units about the evaluated centre,
and accumulates adjacent observed angles **before native Move coalescing**. Its compact frame/map
proof runs retain event sequences: the existing arming boundary permits pre-arm playback,
while every post-arm contributing run must satisfy the original frozen-frame/source checks.
Passing through the centre (within one device pixel of it) holds the accumulated angle and
resets its next angular baseline.
No client coordinates or control/angle declarations are accepted by `window.input`.

The desktop's numeric controls show/edit left-key values through the shared palette gesture
queue. A Window field intention resolves to a whole-array wire edit only against the confirmed
predecessor; key ownership follows the actual array field, not just the `windows` container.
Esc snapshots that confirmed whole array. Window selection or a participating key change
permanently retires old callbacks. Grade completeness gates native binding, not numeric editing.

`{binding,capture,kind:"up"}` explicitly finishes without adding a final position. A repeated
up/cancel for the last ended capture is acknowledged. `{binding,capture,kind:"cancel"}` is
Esc: restore the original complete windows array through the same owned gesture, then finish;
an empty restored step disappears from undo. Native lost capture keeps the last successful
move. An external successful edit ends capture and never restores against a fresh revision;
a failed edit does not end it. A live native capture can remain held beyond the normal wire
gesture's 10-second idle limit; multiple moves remain one undo. A no-move click creates none.

Up, lost capture, disconnect, view destruction/replacement, map/placement/source changes,
tool/target changes, project changes and `{binding}` to **`window.unbind`** release capture.
`window.unbind` also clears the selection/overlay. Mouse-up stays paused; playback never
automatically resumes. Ordinary transaction replies and rendered/exported project pixels are
unchanged. Refusals use `-32001` with a narrow `data.reason` such as `binding`, `capture`,
`event`, `map`, `hit`, `selection`, `target` or `stale_revision`; invalid parameter schemas are
`-32602`. The desktop shares one native pointer owner with viewer navigation and Image Wipe.

### Linear and Polygon Window controls

The same binding, native down evidence, arming/freeze, owned gesture, left-key, Esc and undo rules
as Circular and Gradient apply. The worker draws enabled Linear and Polygon outlines through the
displayed (interpolated, clamped) local points, in their stored order; a self-intersecting outline
is filled even-odd and never reordered. Only the selected window draws white handles and grey
feather guides, exactly where its key becomes opaque inside and zero outside: the boundary of the
points on that side at least each edge's extent from that edge (Linear Soft *j*/400 on both
sides, Polygon Inside/Outside Softness/200, local units, so they follow rotation and aspect).
Edge offsets meet in mitres; about a vertex where the outline turns away they are arcs (drawn as
device chords within 1/16 pixel); pieces nearer another edge are dropped, so an inner guide never
leaves the window and disappears when the shape cannot hold its inside softness. Guides are
display only and never hit. Every other window is a grey outline. Size 0 draws nothing.

Handles (RM20 p. 3193): **Linear** has white corner handles, white side handles at each side's
middle, and one magenta softness handle per side on its outer guide, 3/4 of the way from that
side's first point. A **Polygon** has a white handle at each point and no softness handles; its
Inside/Outside Softness are numbers only. Within the selected window, an explicit handle wins,
then a Polygon edge (insert), then the body (move); overlapping candidates choose the nearest
device distance (to a handle's centre, or to an edge segment), then the lower local index.
Linear edge strokes and every unselected window's stroke and even-odd body move the window.

Down `keys` report the participating left keys. A corner or point reports
`"points":[{"point":i,"x":frame|null,"y":frame|null}]`; a Linear side handle reports its two
points; a Linear softness handle reports `"softness"` and `"edge":j` (Soft *j*+1, the side from
point *j* to the next). Points and side handles write `L + delta` to each participating x/y left
key, where delta is the pointer displacement through the captured displayed transform's inverse,
clamped to the coordinate bounds. A softness handle writes `L + 400 * (delta . n)`, n the side's
outward local unit normal at down, clamped to 0–100.

**Insertion.** A down on a selected Polygon's edge inserts one point into that edge for the whole
animation, at the share s (clamped to .001–.999) of the clicked spot along the displayed edge.
The point count is the same on every frame; every other Param is kept. The new x and y are the
exact sums `(1-s)*A + s*B` on the union of the two ends' existing key frames (Fixed, Linear,
Hold, and Bezier easings sharing one time parameterization are supported, including mixtures
that reduce to these), so the shape is unchanged at every instant. Otherwise the whole down is
refused with `data.reason:"insert"` and
`data.topology:{why,axis,points,frames,clock}`: `why` is `jump` (a held jump during a continuous
change), `timing` (different non-linear easings), `overshoot` (an original curve leaves the
coordinate range and is clipped), `range` (the new coordinate would), or `encoding` (the exact
split needs a key handle outside 0–1); `points` are 0-based point indices, `frames` the key
interval in `clock` frames. A Polygon at 128 points refuses with `{why:"maximum",most:128}`.
Nothing is baked, dropped or approximated. The successful down reports `"inserted":i` with the new
point's keys. The insertion is the capture's first owned step once armed, so a drag moves the new
point in the same undo step and Esc restores the original list. A click released before arming
completes still inserts once, as one step against the unchanged down revision, if the native left
button was actually released; otherwise nothing is written.

**Deletion.** A native middle-button down is retained apart from left captures, with its frame
snapshot, until its release. `{binding,kind:"delete",event}` names that down; it is claimed once
by its own connection and only after an unmoved release (`middle_moved` false and the same map).
The engine hit-tests the down's own frame and overlay: only a selected Polygon's point handle
deletes, removing that point (and its Params) from the whole animation as one plain step. A
deletion that would leave fewer than three points is refused entirely with
`data.reason:"delete"` and `data.topology:{why:"minimum",least:3,shape:"polygon"}`; the window
is not deleted.
A moved middle gesture is viewer pan (`data.reason:"moved"` if named), another hit is `hit`, and
an active capture is `capture`. These handle positions, increments, the insertion rule and the
point-count rule are project choices; Resolve instead makes an animated Polygon's points pop on
and off at keyframes (p. 3194).

### Curve Window controls

The same binding, native down evidence, arming/freeze, owned gesture, left-key, Esc and undo rules
as the other shapes apply. A new Curve is an empty open draft and shows no controls (RM20
p. 3193); clicks in the viewer draw it.

**Drawing.** The outline is the mask's own flattening of the displayed (interpolated, clamped)
anchors and handles: each edge a cubic `[A, A+A.out, B+B.in, B]`, cut until straight to a quarter
device pixel, never finer than the mask's own tessellation. An open draft has no closing edge
and no body. Only the selected Curve draws white anchor handles and each non-zero Bezier handle as
a display-only line to a smaller white end; a closed one also draws grey Inside/Outside guides
(the exact offsets of that outline at Inside/Outside Softness/200, as Polygon; display only, never
hit) and two magenta softness handles: the inside one on the inside guide at parameter 1/3 of
edge 0, the outside one on the outside guide at 2/3 (each on the side the even-odd mask reads
as inside or outside there, also on a lobe of a self-intersecting Curve). Every other window is
a grey outline; clicking an unselected Curve's outline or closed body selects and moves it.

**Hit priority** within the selected Curve: an anchor or handle end, then an edge, then the
closed body (move), nearest device distance first. A closed Curve's body also takes a down
within 4.5 device pixels of its drawn outline, selected or not ("drag anywhere within or just
outside a curve to move it", p. 3194): below that Curve's own controls, above the windows
underneath, independent of its softness guides. A draft has no body and so no such margin. While a draft is selected, a down that hits
none of its controls (even over another window) draws: it appends a static corner point at the
clicked spot, through the displayed transform's inverse, for the whole animation, and reports
`"inserted":i` with the point's handle keys. A binding may select no window (`selected:null`,
which the desktop uses when Esc leaves a draft): then a down on any window's body or a draft's
outline selects it and moves, and any other down is `hit`.

**Drafting and closing.** A drag that travels at least 4 device pixels (an anchor's radius, at
any viewer zoom) from the down pulls symmetric Bezier handles out of the new point (out = the
local displacement, in = its negative); a shorter wobble keeps a corner. With two or more points, the draft's first point closes it: a
click closes it with a corner there (its handles' left keys go to 0), a drag closes it with the
pulled symmetric handles; no point is added and `closed` becomes true. A one-point draft's first
point is an ordinary point. The draw or close is the capture's first owned step once armed, so
its drag belongs to the same undo step and Esc during it removes only that point or closing (the
rest of the draft stays); a click released before arming completes still draws or closes once.

**Editing** (p. 3194; open drafts too). An anchor drag writes `L + delta` to its x/y left keys
(its relative handles go with it). A handle drag writes the dragged handle's left keys, and turns
and scales the opposite handle by the same rotation and scale (their angle and length ratio
stay); with Alt the opposite handle stays where it is. A zero opposite handle stays zero; a
handle dragged from zero has no direction, so its opposite stays. Alt with a drag on a corner
(both handles zero as displayed) pulls a symmetric pair; Alt with a system double click on an
anchor writes its handles' left keys to 0 (a corner), then a drag moves it. Down `keys` report
`"points":[{"point":i,"x","y"}]` for an anchor and `{"point":i,"out_x","out_y","in_x","in_y"}`
for its handles. A softness handle writes `L + 200 * (delta . n)` to Inside or Outside Softness
(reported as `"inside"` or `"outside"`), n the local unit normal at down pointing away from the
outline on that handle's side, clamped to 0–100.

**Insertion.** A down on a selected Curve's edge splits that cubic at the parameter of the
nearest point of the cubic itself (clamped to .001–.999) for the whole animation, by de Casteljau:
the new anchor and its two handles, and the two neighbours' shortened facing handles, are exact
fixed combinations of A, A.out, B.in and B on the union of their key frames, so the curve is
unchanged at every instant. Refusals, `"inserted"`, the first owned step and the unarmed click
are as for Polygon (`topology.points` name anchors); a Curve has no point maximum.

**Deletion.** A middle tap on a selected Curve's anchor, including an open draft's first point,
removes it from the whole animation as one plain step. A closed Curve keeps at least two points:
otherwise the whole delete is refused with `data.topology:{why:"minimum",least:2,shape:"curve"}`
and the Curve stays closed; an open draft may become empty. Drawn points are static, and the pull
threshold, the margin just outside, handle placements and the linked rotation and scale are
project choices.

**Multi-point selection** (p. 3195; closed Curves and drafts, not Polygons). A down with Shift
held while a Curve is the selected window, anywhere on the picture, is a band: it hits no
control (`keys` has only `clock`/`frame`), draws a display-only white rectangle from the down to
the pointer while armed, and on release selects the anchors it encloses as that frame displayed
them, each counting within its 4 device pixel radius (so a Shift click on one anchor selects
it). The band replaces the selection; one enclosing none clears it; Esc during it keeps the
previous selection. Selection is not part of the project or its history and is never sent by a
client; the selected anchors are drawn amber. With two or more selected, a display-only white
box stands 8 device pixels off their bounding box (axis-aligned on screen) with blue corner and
side handles. **Hit priority** with that box: its handles (nearest first), then anywhere inside
it (`BoxMove`, over the Curve's own anchors, handles and edges), then a down outside it within
14 device pixels of a corner (rotation), then the Curve's own controls as above; a middle tap
looks through the box to the anchor under it. Dragging inside moves the selected anchors
(`L + delta` to their x/y left keys, handles kept); a corner scales them uniformly and a side
stretches them along its axis so that the handle follows the pointer (along the corner's down
offset from the box centre, or along the side's axis: in device pixels, the box's 8 pixels off
the anchors do not scale, and a down offset reaching less than one pixel past them counts as
one); the rotation turns them by the angle the pointer turns about the centre; all about the box
centre in canvas pixels (factors never below 0). Each map is applied in full to the displayed anchors and by its
linear part to their relative handles, writing each field's `L + delta`, clamped; `keys` reports
every participating anchor (`x`/`y`, and `in_*`/`out_*` for a transform). It is one owned
gesture: one undo step, Esc restores the whole list. A down whose frame does not show the
current point selection is refused `selection`. The selection lasts through its own drags and
undo/redo while that Curve keeps its anchor count; a changed count, another selected window, a
new bind, or any other down clears it. When a project change (an undo or redo, or another
client's edit) clears it, the binding owner is sent **`window.points`**
`{binding,sequence,revision,selected_points:[]}` (the sequence shared with `window.state`), so a
count kept from `window.state` follows. `window.input` `{binding,kind:"clear_points"}` (Esc)
clears it and answers `cleared`, the number it held (0 if none, for the desktop then to leave a
draft). `{binding,kind:"delete_points"}` (Backspace) removes the selected anchors from the whole
animation as one plain step, all or none: a closed Curve keeps at least two, otherwise
`data.topology:{why:"minimum",least:2,shape:"curve"}` and the selection stays; none selected is
`{reason:"points"}`; during a capture `{reason:"capture"}`. Box padding, the rotation reach, the
centre pivot and the anchor radius of a band are project choices.

## Window tracking

T24: the window tracker of the Color page (Resolve 20, chapter 140, pp. 3229–3255), the cloud
tracker and the point tracker of `columbina-track` (Q7). IntelliTrack is not offered; the
stabilizer is phase 4.

**What a window keeps.** A tracked window has

```json
"tracking": {"mode": "clip", "crosshairs": [[0.5, 0.5]],
  "data": {"anchor": [639.5, 359.5], "frame_size": [1280, 720],
    "components": {"pan": true, "tilt": true, "zoom": true, "rotate": true, "perspective": false},
    "curves": {"pan": {"12": 3.25}, "tilt": {"12": -1.5}, "zoom": {"12": 1.01},
               "rotate": {"12": 0.02}, "aspect": {"12": 1.0}, "shear": {"12": 0.0},
               "yaw": {"12": 0.0}, "pitch": {"12": 0.0}},
    "confidence": {"12": 0.97}, "points": [{"12": [600.0, 300.0]}],
    "keys": {"20": {"pan": 4.0, "tilt": 0.0, "zoom": 1.0, "rotate": 0.0, "aspect": 1.0,
                    "shear": 0.0, "yaw": 0.0, "pitch": 0.0}}}}
```

- `mode`: `clip` (the default) or `frame`, how a change made by hand applies (p. 3236), kept
  with the window so that a drag in the viewer does what the palette shows.
- `crosshairs`: the point tracker's crosshairs at the frame its next track starts from, in
  normalized picture coordinates (0 to 1, y down); at most 64; left out when empty.
- `data`: `curves` hold, per component, the motion from where the window was placed at each
  tracked frame (the tracker graph, p. 3236); a frame between samples takes the line between
  them, one before the first or after the last takes that sample (p. 3232). `pan`/`tilt` are
  pixels of `frame_size`, `rotate` radians clockwise, `zoom`/`aspect` factors, `shear`,
  `yaw`/`pitch` (per pixel) the rest of Perspective 3D. `confidence` is 0–1 per tracked frame;
  `points` are the crosshairs' paths; `keys` are Frame mode's keyframes. Frames are those of
  the grade's clock: source frames for a clip (T3c). Every number must be finite, zoom and
  aspect positive, frames within ±(2^53−1), `frame_size` at most 16384 a side and not 0 once
  anything is tracked; otherwise the edit is refused with `out_of_range` and
  `what: "windows.tracking.*"`.

**Where a tracked window is.** With `A(f)` the window's own Transform (the map above, local to
normalized picture), `T(f)` the tracked motion and `K(f)` Frame mode's keyframe about the
tracked anchor (both homographies in pixels of `frame_size`, see `Motion` in
`columbina-track`), and `S` from those pixels to normalized coordinates (pixel `(i, j)`'s centre
at `((i+0.5)/W, (j+0.5)/H)`):

`picture(x, f) = S · K(f) · T(f) · S⁻¹ · A(f) · x`

So the window's own Transform and its keyframes apply first, in the window's untracked place,
then the track, then Frame mode. The frames are tracked upright at the picture's aspect, so
this does not depend on the size the picture is shown at. A mask point is taken back through
the inverse homography; beyond the vanishing line of a Perspective 3D track (where its third
coordinate is not positive) none of the window is there, before Invert. This order is the
project's reading of the manual (pp. 3232, 3236); Resolve publishes no formula.

**On the viewer.** The window controls (above) are drawn, hit and dragged through the same
composition. In **Clip** mode a drag is read back through the track and changes the window's
own Transform (its left key, as before), so the window moves the same way along its whole
path. In **Frame** mode (p. 3236: "changes you make to the position or shape of a window
create a keyframe", for rotoscoping, pp. 3250–3252) a change of the whole window (Pan, Tilt,
Size, Aspect, Rotate) becomes a tracker keyframe at the playhead, the change as it is seen
there, and the Transform is kept; a drag of points of a Linear, Polygon or Curve window keys the
coordinates it changed at the playhead (a linear key at the value shown there plus the change,
other keys kept) and sets the tracker keyframe there at its motion, so the graph's ruler shows
it. A window never tracked is keyed too (p. 3250): its `data` is first set up on the frames a
track of its clip would have (below), anchored at its centre at the playhead; the window of a
grade that is not a clip's moving picture is changed as in Clip mode. Softness, and points added
or a Curve closed, apply to the whole clip. The controls are affine: a Perspective 3D track is
drawn by its derivative at the window's centre, exact there; the mask follows the full
homography.

**`track.start`** `{grade, node, window, revision, frame, direction, tracker?, components?}`:

- `grade` must be a clip's grade whose media has a moving picture: a timeline or group grade,
  a title or colour, or a still is refused with `reason: "untrackable"`, as is a window that is
  off or an unfinished Curve. `revision` must be the project's (`stale_revision`). `frame` is
  the playhead in the grade's clock (source frames) and must be within the frames the clip
  shows (`reason: "frame"`).
- `direction`: `forward` (Ctrl+T), `reverse` (Alt+T), `forward_and_back`, `one_forward`
  (Alt+→), `one_reverse` (Alt+←) (pp. 3231, 3236), on the timeline: a clip played backward is
  tracked backward in its source. A track ends at the first or last frame the clip shows.
- `tracker`: `cloud` (the default) or `point`; the point tracker needs at least one crosshair
  (`reason: "no_points"`). `components` are the boxes (p. 3235), default Pan, Tilt, Zoom and
  Rotate on and Perspective 3D off; a track writes all eight curves, those off at no motion.
- One track goes on in an engine at a time (`reason: "busy"`).

The answer comes at once: `{job, first, last, frame_size, direction}`. The track then runs on a
thread of its own: the clip's media (its proxy where usable) decoded on the processor, upright,
scaled to at most 720 lines at its own aspect (`frame_size`; pixels are square, as the engine
shows every picture: the project keeps no pixel aspect), as luma at a fixed scale. A
window tracked before keeps its data and the track continues it from the motion at `frame`
(p. 3232); the frames it reaches are overwritten (p. 3231). The track so far is written into the
window by `set_node` as a change of one native gesture of the connection (`project.changed` to
everyone, `track.progress` to it): the viewer shows the window following, and the whole track is
**one step to undo**. As each write carries the window's whole track, writes are 150 ms apart
while it is short and further apart as it grows: at least its size in bytes over 2 MB/s, and five
times as long as the last write held the engine, but never more than 1 s apart (a track of
10 000 frames writes about 1.75 MB, so about every 0.88 s).

`track.state` ends it: `done`; `stopped` with a `reason`; `failed` with `no_features`,
`no_points`, `missing_frame` or `cannot_read`, and a `message`. The reasons of `stopped`:

| `reason` | why |
|---|---|
| `stop` | `track.stop` (Ctrl+Alt+T), or the Stop button |
| `disconnected` | the connection that started it went |
| `stale_revision` | the project was changed by anyone (the track's next write was not on the revision it last wrote), or Undo or Redo |
| `stale_gesture` | the track's gesture was ended or replaced while the project stayed as the track last wrote it |
| `gesture_owner` | the gesture was not the connection's (not expected: a track writes only its own) |
| `project_changed` | its write was refused with no reason given |
| any other | its write was refused for that reason, as `window` when the window was deleted or `target` when its node or clip was |
Whatever the end, what was written stays as its own undo step (an Undo during a track takes it
away); a track that wrote nothing makes no step. Frames where the tracker lost the subject are
taken off the curves (a line bridges them) and counted in `lost`.

**`track.edit`** `{grade, node, window, revision?, edit}`, one step to undo each:

| `edit` | does (manual) |
|---|---|
| `{"kind":"clear","from":f,"to":g,"components"?:[...]}` | Clear Selected Track Data: the samples of those curves (all where left out) taken away, a line across (p. 3238) |
| `{"kind":"interpolate","from":f,"to":g,"components"?:[...]}` | the samples strictly between made the line between their values at `f` and `g` |
| `{"kind":"reset"}` | Reset Track Data on Active Window; Frame mode's keyframes stay (p. 3238) |
| `{"kind":"mode","mode":"clip" or "frame"}` | Clip/Frame (p. 3236) |
| `{"kind":"add_key","frame":f}`, `{"kind":"delete_key","frame":f}` | Add Keyframe: a Frame mode keyframe at the motion there, and a key at `f` of every coordinate of the window's points at its value there, so later changes interpolate from there (p. 3251); a window never tracked is first set up on its clip's frames, anchored at its centre at `f` (p. 3250), and one of a grade that is not a clip's moving picture is refused (`reason: "untracked"`). Delete Keyframe: the keyframe and the points' keys at `f` taken away, a coordinate left with none keeping its value there (pp. 3238, 3251) |
| `{"kind":"crosshairs","points":[[u,v],...]}` | the point tracker's crosshairs: Add Tracker puts one at `[0.5,0.5]`, Delete Tracker takes one away (p. 3238) |
| `{"kind":"paste","from":"w2"}` | Paste Track Data from another window of the node, as the same motion of the frame about this window's own anchor (p. 3233); `reason: "nothing_to_paste"` if it has none |

Copy Track Data is the client's: it remembers the window to paste from. A window preset never
stores `tracking` (T23). Not yet: Interactive Mode's point cloud editing, the crosshairs and
motion path drawn on the viewer (Show Track) and dragged there, the outline drawn through a
Perspective 3D homography, tracking across a transition's handles, timeline and group grades,
and the move of tracking frames when a grade is copied to another clip with its keyframes.

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
| `transport.scrub_audio` | `{"monitor":"timeline", "on":true \| false}` (phase 3, T18) | `{}` |

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
  `"off"` (not played now: paused, or, on the source monitor, a speed other than 1), or
  `"unavailable"` (there is no
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
 "source":"mixed", "proxies":true, "scrub_audio":true}
```

- `scrub_audio` (phase 3, T18): whether dragging the playhead plays the sound under it (Timeline ›
  Audio Scrubbing, Shift+S, on at first, p. 731), as `transport.scrub_audio` sets it. It is kept
  when another timeline is loaded. `transport.scrub_audio` is of the timeline monitor only: with
  `"monitor":"source"` it is refused with `-32602 {reason:"monitor"}` (the source monitor plays
  no sound while its playhead is dragged).

- Timeline positions use the timeline's frame rate; `last` is `max(duration - 1, 0)`.
  An empty timeline shows black at frame 0. Removing the loaded timeline unloads it and clears
  its view. A missing timeline is refused with `-32602 {reason:"missing", timeline:"..."}`.
- `shown` identifies the last composed timeline frame; individual layers may stand in with a
  key frame while scrubbing, or their track's previous picture when decoding is late. `exact`
  is true only when every media layer is exact. `late` counts playback frames for which at
  least one layer was unavailable, once per frame, and resets when playback starts from pause.
- `compared` (where a wipe or split screen is on): the frame of the timeline that each of its
  references shows for the frame shown (see Comparisons in the viewer).
- `recording: true` while automation is recorded as the timeline plays (`automation.record`; see
  Recording automation); left out where it is not. `all_read: true` while All Read is on
  (`automation.all_read`, T16b); left out where it is not.
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
  again and keeps the sound playing; and a change of what is heard never starts the sound
  again (phase 3, T5): a change of a setting that the mix takes as it plays (a fader, a pan, a
  trim, a send's level, a mute, a solo, a parameter of Dynamics, the equalizer or an effect, a
  clip's gain or pan where it is one number) is heard within some 35 ms, ramped over 10 ms; any
  other change of what is heard from where the mix is to the end (clips added, moved or
  trimmed, their keyframes, fades and speeds, effects added or turned on, the routing, the
  lanes) is heard once another mix, built in the background, takes the place of the one that
  plays at a sample some 150 ms ahead (further where the mix takes longer to build), the two
  crossfaded over 10 ms. Only a change of the rates of the timeline starts the sound again at
  the playhead. Where the mix of a play or a seek is not ready within 2 s (a drive that sleeps),
  the picture goes on by the clock of the system (`sound` is `"unavailable"`), and once it is
  ready the sound starts again half a second ahead of the picture and becomes the clock when it
  is heard (`sound` is `"on"` again).
- Each enabled video track reads ahead independently, including upcoming cut/transition
  handles. The shared decoder pool allows twelve decoders, and of one media and source kind
  two for each enabled track of picture that uses it (two at least), so that tracks reading one
  media at once do not take turns with its decoders. At most twelve decoders of the monitor and
  an export together use the graphics card; any more decode on the processor.
  Only unheld composition targets are reused, so a view never samples a target being rewritten.
- **Sound at every speed (phase 3, T18).** At every speed of `transport.play` (−8 to 8) the
  timeline's sound is played and is the clock of the picture, `sound` `"on"`: the mix plays the
  timeline at that speed, forward or backward, each clip resampled to its own speed times the
  speed of playing, so that the pitch follows the speed (2 plays a tone of 440 Hz at 880 Hz, −1
  plays the sound reversed), as a clip's speed without Pitch Correction does; at a speed other
  than 1, Pitch Correction and a clip's own pitch are not applied (in DaVinci Resolve only the
  slow motion of K+L is pitch-corrected, and only on macOS, p. 3745); back at 1 they are applied
  again, the sound as the export has it. Above 2 times (shuttling) the clips are read through a
  shorter filter (3 zero crossings on each side in place of 16), what would fold back staying
  some 44 dB down. Fades, keyframes and lanes are where the timeline is, and the effects run on the sound as it comes out (so that backward
  they hear it reversed, as a clip played in reverse is). Sample `n` heard after playing started
  at sample `f` of the timeline at speed `s` is sample `f + s·n` (at the timeline's rate), so
  the picture keeps to the sound at every speed. A change of speed while the sound plays does not
  start it again: the mix at the new speed is built in the background from where the sound will
  be some 150 ms ahead (further where it takes longer to build) and takes the place of the one
  that plays there, crossfaded over 10 ms; the playhead keeps its speed until then, and changes
  it at the sample where the sound does (`speed` in the state is the new one at once). Speeds
  below 1 (K+J, K+L) cannot be asked for (the speed is a whole number): they are later.
- **Scrubbing with sound (phase 3, T18).** While `scrub_audio` is on, each scrubbing seek
  (`"scrubbing": true`) of the timeline monitor plays a **grain**: 60 ms of the timeline from the
  start of the frame sought, forward at speed 1, through the whole mix (its effects too), faded
  in and out on straight lines over 5 ms at either end, as the export has it (Pitch Correction and
  a clip's pitch too). The first is heard once its mix is built (some 30–100 ms); each next takes
  the place of one that still sounds, crossfaded over its fade of 5 ms, some 20 ms after what is
  mixed (further where the grain before took longer to build), built again further ahead where it
  was not ready in time, so that its sound starts at the frame sought; seeks that come while a
  grain is built are taken together (only the last is played). The sound stops when the dragging
  stops (the last grain plays to its end) and ends a second after the last grain was asked for; a
  seek that is not scrubbing lets the last grain end as it is; pausing or turning `scrub_audio`
  off fades it out over 5 ms, and playing ends it at once. Changes of the project are heard from
  the next grain on, and a change of the rates of the timeline starts the grains again. While
  grains play, `sound` is not `"on"` (the picture is not played, and the sound is no clock).
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
  Every speed plays its sound (phase 3, above; the source monitor's speeds other than 1 are still
  silent). No enabled audio clips reports `"none"`; paused audio reports `"off"`.

**Test hook.** Only with `--test-hooks`, `transport.frame_digest {"monitor":m}` reads the current
rendered frame and returns `{monitor, frame, width, height, hash, pixels}`. `hash` is SHA-256 of
the entire RGBA frame, for regression comparisons; `pixels` contains the four top-left RGBA
pixels. It is not a playback operation or project identity. Without the flag, or headless,
the method returns `-32601`; before a picture is loaded it returns `-32004` `not_loaded`.
While timeline sound is playing, the hook also returns `audio_samples` (device-rate samples
heard since that output started), `audio_frame` (the corresponding timeline clock frame) and
`audio_sample` (where the mixer is in the timeline, in timeline samples: the sample it started
at plus those heard, at the timeline's `sample_rate`, each times the speed it was heard at, T18),
so a test can tell the sound from the
picture by the mixer itself rather than by the clock that the sound drives.
They are given while the timeline's sound is `"on"`; `audio_frame` or `audio_sample` is `null`
while the output has no clock yet or the mixer has not started. With them come
`audio_short_runs` (runs of the device short of sound since the output started),
`audio_underruns` (reads of a clip whose sound was not read ahead yet, since the monitor first
played), `audio_swaps` (graphs of the mix swapped in as the project changed, likewise),
`audio_late_swaps` (graphs that were ready after their sample and were built again),
`audio_mix_priority` (`"ProAudio"`, `"TimeCritical"` or `"Normal"`: what the thread of the mix
was given by the system) and `audio_capture_dropped` (frames that a `capture` output left out).
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
| `set_media_streams` | `id`, `layout` and `streams` of its `facts.audio` (each `null` or left out for not read; its inverse sets those before). Media without sound is `no_stream` (`kind: "audio"`). Nothing else is checked, as nothing is where a project is read: a channel of a clip that names a stream that the media has not plays silence. The engine sets them of media of an older project (see Streams read later); one that sets them to what they are changes nothing (alone in a transaction, `empty`) |
| `set_timeline` | `id`, `name?`, `rate?`, `width?`, `height?`, `sample_rate?` |
| `set_track` | `timeline`, `id`, `name?`, `color?` (`#rrggbbaa`, null restores the unspecified default), `enabled?`, `locked?`, `language?`, `mute?`, `solo?`, `solo_safe?` (the last three of a track of sound only; on another `wrong_track` with `what` of the setting) |
| `move_track` | `timeline`, `id`, `index`: zero-based index in the final complete track list, including video, audio and subtitle tracks; preserves the existing object and every ID/reference. Out-of-range or unchanged indexes are refused as `out_of_range` (`what: "track index"`). Like other basic track operations it can be undone while locked. |
| `set_clip` | `timeline`, `id`, `enabled?`, `speed?`, `reverse?`, `link?`, `transform?`, `gain_db?`, `pan?`, `fade_in?`, `fade_out?`, `audio_fade_in?`, `audio_fade_out?` (an `AudioFade` or `null` for a straight fade over the frames), `eq?`, `effects?`, `pitch_correction?`, `pitch?`, `channels?` (see Operations of sound), `source?` |
| `add_bin` | `bin`, `index?` |
| `remove_bin` | `id` |
| `set_bin` | `id`, `name?`, `parent?` |
| `set_media` | `id`, `name?`, `bin?`, `mark_in?`, `mark_out?`, `still?` |
| `add_transition` | `timeline`, `track`, `transition` |
| `remove_transition` | `timeline`, `id` |
| `set_transition` | `timeline`, `id`, `kind?`, `length?`, `align?`, `fade_in_level_db?`, `fade_out_level_db?` |
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

### Operations of sound

The operations on the sound of schema 4 (`crates/model/src/ops/sound.rs`), each taken back by
its inverse:

| op | fields |
|---|---|
| `set_track_audio` | `timeline`, `track` (a track of sound), `patch` |
| `add_bus` | `timeline`, `bus`, `index?` |
| `remove_bus` | `timeline`, `id` |
| `set_bus` | `timeline`, `id`, `patch` |
| `add_vca` | `timeline`, `vca`, `index?` |
| `remove_vca` | `timeline`, `id` |
| `set_vca` | `timeline`, `id`, `patch` |
| `add_track_group` | `timeline`, `group`, `index?` |
| `remove_track_group` | `timeline`, `id` |
| `set_track_group` | `timeline`, `id`, `patch` |
| `add_insert` | `timeline`, `of`, `insert`, `index?` |
| `set_insert` | `timeline`, `of`, `slot`, `insert` (an insert, or `null`, which must be written, to take the one at `slot` away) |
| `move_insert` | `timeline`, `of`, `from`, `to` |
| `set_timeline_audio` | `timeline`, `patch` |
| `set_lane` | `timeline`, `of`, `path`, `lane` (a lane, or `null`, which must be written, to take it away) |
| `splice_lane` | `timeline`, `of`, `path`, `at`, `remove`, `points` |

**Patches.** `set_track_audio`, `set_bus`, `set_vca`, `set_track_group` and
`set_timeline_audio` set their thing by a patch, as `set_color_settings` does: each object in it
is merged into the object of that name, and any other value (a number, a list, `null`) takes the
place of the one there, whole; so `{"strip": {"fader_db": -6, "pan": {"x": 0.25}}}` changes two
settings of a strip and `{"strip": {"sends": [...]}}` replaces all its sends. A patch of a track
names `format`, `channels`, `order`, `mute`, `solo`, `solo_safe`, `adc`, `automation_arm` and
`strip` (not `lanes`, which the operations of automation set); a `format` given without
`channels` sets the channels of the format (and keeps those of the track for `adaptive`), so
`{"format": "mono"}` is done as `{"format": "mono", "channels": 1}`. A patch of a bus or a VCA
names any of its fields but `id` and `lanes`; of a group any but `id`; of a timeline any field of
its `audio`. The inverse is the patch of what was there at the places that the patch names; the
operation as it was done (`done`) is the patch of what is there now at the places of its inverse,
as the engine writes it (`-6` as `-6.0`, a Ducker set where there was none whole, with its
defaults). A patch that is not an object, names a field that is not known or may not be set so,
or holds a value of the wrong kind is refused as `audio_settings` (`why`, serde's message).
Whatever a patch makes is checked as a project that is read is: a setting out of its range is
`out_of_range`, and the routing is checked again (`bus_cycle`, `bus_depth`, `unknown_bus`,
`unknown_vca`, `vca_cycle`, ...). `set_track_audio` of a track that is not of sound is
`wrong_track` (`what: "strip"`).

**Ids.** A bus added with an empty id is given `B` and one above the highest number of the ids of
that shape among the buses of its timeline (`B2` beside `B1`), a VCA `VCA` and such a number, a
group `TG` and one; they do not use `next_id`, so one taken away and added again without an id may
get the id it had; a bus also passes over the ids of the tracks of its timeline (`B3` beside a
track named `B2`). One given an id keeps it, unless another of its kind holds it (`taken`), a
track of its timeline holds it (a bus: `taken`, `what: "track"`) or it starts with `$`. Buses,
VCAs and groups have no names of a transaction; a track may be named by one where a group names
it as a member (`add_track_group`, and `members` of a patch of `set_track_group`) and where it
drives a sidechain or a Ducker (`{"track": "$name"}` anywhere in a patch of `set_track_audio`
or `set_bus`, in the strip of `add_bus`, and in the insert of `add_insert` and `set_insert`).

**Taking away.** `remove_bus` of a bus that a strip outputs or sends to, or whose sound drives a
sidechain or a Ducker, is refused as `in_use` (`what: "bus"`, `by` the track or bus); of the bus
that is monitored, `in_use` with `by` the timeline. `remove_vca` of a VCA that a strip or another
VCA is a member of is `in_use` (`what: "vca"`). A group is taken away whatever it holds; its
tracks stay. A bus, VCA or group that is not there is `unknown_bus`, `unknown_vca`,
`unknown_group`.

**Inserts.** `of` names a chain of inserts: `{"track": id}` (the strip of a track of sound),
`{"bus": id}` (the strip of a bus) or `{"clip": id}` (the effects of a clip, in order). `add_insert`
puts one at `index` (at the end where it is left out), and an insert of a strip with an empty id is
given the next `fx<n>` of its strip, above every id that the strip had before the operation, so
that an insert given no id is always new and never takes the id, and so the lanes, of one that it
replaces (the same for the inserts of a patch of `set_track_audio` or `set_bus`); one of a clip
keeps the id it is given, which may be none.
`set_insert` puts one in place of the one at `slot`, or with `insert: null` takes that one away,
those after it moving up; its inverse puts back what was replaced, or adds back what was taken
away (`add_insert` at `slot`). `move_insert` moves the one at `from` to `to`, those between moving
over; an insert keeps its id as it moves within its chain, and so do its lanes, while an insert
copied or moved to another strip is a new insert there, which takes the next id of that strip:
`copy_effects` gives it an empty id, and a client that moves an insert to another strip takes it
away from the one and adds it to the other with an empty id. A `slot`, `from` or `to` that is not
a place of the chain is `out_of_list`; a strip of more than 6 inserts is `too_many_inserts`; an
effect of a clip with a sidechain on is `out_of_range` (`sidechain`); an insert taken away or
replaced while a lane names its id leaves the lane naming what is not there, refused as
`out_of_range` (`lane`), and so is an insert that keeps its id with an effect of another kind
while a lane names it; so the lane is taken away first: the edits `set_insert`, `move_insert`,
`copy_effects`, `set_track_audio` and `set_bus` (below) do that in the same transaction.

A strip sends to a bus at most once: two sends to one bus are `out_of_range` (`sends`).

**Lanes** (`crates/model/src/ops/automation.rs`). `of` names what a lane is of: `{"track": id}`
(a track of sound), `{"bus": id}` or `{"vca": id}`; `path` names its setting (see Lane above).
`set_lane` puts `lane` in place of the lane of `path`, or adds it where there is none, or with
`lane: null` takes it away; its inverse puts back what was there (`null` where nothing was).
`splice_lane` puts `points` in place of the `remove` points of the lane from its point `at` (from
0) on, as JavaScript's `splice` does; its inverse is the `splice_lane` that puts back the points it
took away in place of those it put, so a change to a few points of a long lane is a short step of
undo. A lane must keep at least one point (`set_lane` with `null` takes one away), and what either
makes is checked as a lane that is read is (`out_of_range: lane`). A lane that is not there is
`missing` (`what: "lane"`, `id` its path) for `splice_lane` and for `set_lane` with `null`; an `at`
and `remove` that reach past the points is `out_of_list` (`at` their sum); a track that is not of
sound is `wrong_track` (`what: "lanes"`); a bus or VCA that is not there `unknown_bus`,
`unknown_vca`. A track may be named by a name of a transaction (`{"track": "$a"}`).

**Clips and crossfades.** `set_clip` also sets `eq` (whole, 4 bands), `effects` (whole),
`pitch_correction`, `pitch` and `channels` (`null` for the media's channels mixed as libav mixes
them; a list of as many as its track has, else `channel_count`). `set_transition` also sets
`fade_in_level_db` and `fade_out_level_db` (−3, 0 or 3) of a crossfade; of another transition,
anything but 0 is `out_of_range` (`level`).

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
| `replace` | same | Replace the source of the clips at `at`, keeping their positions, lengths and settings; explicit source `from` aligns to `at`. Each clip's grade goes to the new source with its keys aligned by source timecode (see Grades, keys copied to another clip). |
| `place_on_top` | `source`, `at` | Use an unlocked track above occupied tracks, creating one when needed. |
| `append` | `source`, `video_track?`, `audio_track?` | Place after the latest clip end on the target tracks. |
| `smart_insert` | `source`, `at`, `video_track?`, `audio_track?` | Insert at the nearest unlocked audio/video cut; ties choose the earlier cut. |
| `ripple_overwrite` | same | Replace the clip range at `at` and ripple by the difference in lengths. |
| `lift` | `clips`, `linked?`, `grouped?` | Remove selected clips and leave their gaps. |
| `ripple_delete` | `clips`, `linked?`, `grouped?` | Remove selected clips and close their occupied ranges on unlocked tracks. |
| `ripple_delete_range` | `from`, `to`, `tracks?`, `grouped?` | Cut out `[from,to)` and close it, on all unlocked tracks or the given tracks. |
| `close_gap` | `track`, `at`, `straddling?:"refuse"\|"stay"` | Close the gap at `at` and ripple unlocked tracks. With `straddling:"stay"`, clips of other tracks across the gap stay where they are instead of refusing (see below). |
| `blade` | `at`, `tracks?`, `linked?`, `grouped?` | Split crossing clips; original ids stay with first halves, second halves get new ids and link groups. |
| `move` | `clips`, `delta`, `to_track?`, `overwrite?`, `linked?`, `grouped?` | Move clips; default refuses occupied destinations, `overwrite:true` clears them. With `to_track`, every clip that moves, the linked ones too, goes as many tracks up or down among the tracks of its own kind as `to_track` is from the track of the first clip (see below). |
| `insert_move` | `clips`, `at`, `linked?`, `grouped?` | Lift and close original ranges, then insert at the original timeline coordinate. Noncontiguous selections pack their occupied interval union, not the gaps between it. Subtitles on unlocked tracks within a moved range go with it, keeping their ids and their place on the clips; one across an end of a range is cut there, and only its part inside goes along. |
| `place_copies` | `copies: [{clip, track, start}]` | Put a copy of each clip on `track` from frame `start` (a track of its kind, not locked: `wrong_kind`, `locked`), as an Alt-drag duplicate (p. 811) or a paste does: what is there is cleared as an overwrite clears it, after every copy is made of its clip as it was; each copy has all the settings of its clip but its id and transitions, copies of two or more clips of one link group are linked in a new group, and with Automation Follows Edit each takes a copy of the lanes over its clip (see below). |
| `trim` | `clip`, `edge:in\|out`, `delta`, `ripple?`, `linked?` | Move one edge; ripple defaults false. |
| `roll` | `left`, `right`, `delta`, `linked?` | Move a shared cut, keeping outer endpoints. |
| `slip` | `clip`, `delta`, `linked?` | Change source in/out while keeping timeline position and length. |
| `slide` | `clip`, `delta`, `linked?` | Move the clip while adjusting neighbors, keeping its source and length. A final clip may slide and change total duration. |
| `add_transition` | `left`, `right`, `length`, `kind?`, `align?`, `linked?` | Default dissolve on video or crossfade on audio, centered; with `linked`, a transition of picture brings the crossfade of its linked sound (see Edits of sound). |
| `remove_transition` | `id` | Remove the transition. |
| `set_transition` | `id`, `kind?`, `length?`, `align?`, `fade_in_level_db?`, `fade_out_level_db?` | Change given transition settings; the levels of a crossfade as the operation sets them. |
| `set_speed` | `clip`, `speed`, `reverse?`, `ripple?`, `linked?` | Preserve source span; optional ripple uses the linked group's change in maximum end, including J/L cuts. |
| `add_cue` | `track`, `start`, `length`, `text` | Add one subtitle. |
| `set_cue` | `id`, `start?`, `length?`, `text?` | Change given subtitle settings. |
| `remove_cue` | `id` | Remove one subtitle. |
| `cues_from` | `track`, `items:[{start,length,text},...]` | Replace all subtitles on that track. |
| `add_track` | `kind`, `index?`, `name?`, `language?` | Add an empty track; index is among tracks of its kind; name is its display label. A subtitle track is added disabled (hidden) while another subtitle track of the timeline is enabled, and enabled otherwise. |
| `remove_track` | `track` | Remove the track and its contents, all restored by undo; a track of sound leaves the groups of tracks that it is in, in the same transaction (a sidechain or a Ducker that it drives still refuses it as `in_use`). |

The edits of sound are below (Edits of sound).

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

**Groups of tracks that link Editing** (Reference Manual 20, p. 3917). `lift`, `ripple_delete`,
`move`, `insert_move`, `blade` and `ripple_delete_range` take `grouped` (false where it is left
out, and not written then). An editor's own action (a selection deleted, moved or cut by the
user, or by the agent for the user) passes `grouped: true`; edits that a client puts together to
do something else (clearing where copies go, the blades of Backspace over marks) leave it out.
Given `grouped`, where a track of sound is in an active group of tracks whose `linked` has
`editing`, `lift`, `ripple_delete`, `move` and `insert_move` do on each other track of the group,
not locked, what they do over the frames of each clip that they take (after the clips linked to
the named ones): the clips there are cut at the ends of those frames and what is between them
is taken, as `ripple_delete_range` takes a range, and nothing outside them (a music bed under 2 s
of dialogue lifted loses those 2 s only). A `move` with `to_track` takes no other track along.
`blade` and `ripple_delete_range` given `tracks` also cut the other tracks, not locked, of the
groups of the tracks given. It is one edit, so one step of undo. A locked member is left alone
and refuses nothing. Trims, rolls, slips, slides, changes of speed and the placing edits are not
taken along: the manual names jumping to cuts as what follows each track's own boundaries; the
rest is our choice.

**Automation Follows Edit** (pp. 3910–3911; while the timeline's `audio.follows_edit` is true,
as it is for a new one; `set_timeline_audio` sets it). The lanes of a track of sound go with
what the edits of clips do to its clips, as `splice_lane` (or `set_lane`, for a lane made)
operations of the same transaction, one for each lane that changed, after those of the clips:

- A clip taken away (`lift`, `ripple_delete`, an `overwrite`, `move` or copy over it, the frames
  that `ripple_delete_range` cuts out of it) leaves its lanes, from its start to its end, going
  straight from the value that came to its start to the value at its end (a switch holds the
  first): the points within it are taken away, and the values before and after it stay as they
  were.
- A clip moved (`move`, `insert_move`, `slide`) takes the lanes over it along, to where it is
  now, the points that were at its new place replaced; the lane jumps at each end of it from the
  value that the track had there to the clip's own. To another track, its lanes go to the lanes of
  the same settings there; where the track has no lane of a setting, one is made, holding the
  value that the setting is set to on that track outside the clip. A lane that the track cannot
  have (an insert or a send that it has not) is not taken, and stays behind as on a lift.
- `place_copies` puts copies of clips (an Alt-drag duplicate, a paste) and gives each a copy of
  the lanes over its clip, as a move to that place would, the clip's own lanes staying as they
  are.
- A ripple moves the points after its place on the tracks that it ripples, of tracks with no
  clips there too, and those after the last clip of a track: one that takes frames away takes
  away the points in them, the two sides meeting at the place with their values (a jump); one
  that puts frames in goes straight across them from the value that came to the place to the
  value that was there. (The reference keeps the automation of gaps where it is on a move; on a
  ripple, time itself is taken out or put in, so we move it.) Where a clip that does not move
  lies across the place, or starts after it (`close_gap` with `straddling: "stay"`, a ripple
  `trim` or `set_speed` of a split edit), the lanes are cut at its end, so that they stay with
  it. A ripple `trim` of a clip's start ripples the lanes of its track from the start: the
  points over the frames trimmed away are taken away and what the clip still plays keeps its
  automation (lengthened, the frames put in go straight across as in an insert).
- `trim` without a ripple, `roll`, `slip`, `blade`, `set_speed`, `replace` and the transitions
  change no lane over the clip: what a trim shortens is not taken away (it is there again if the
  clip is lengthened), and what it lengthens over holds the values that were there; a change of
  speed does not stretch the lanes. A ripple trim that shortens takes away the points of the
  frames that it takes out of the timeline. A clip placed from its media brings no automation.
  The lanes of buses and VCAs, which have no clips, stay where they are.

Frame `f` is at tick `⌊f · scale · den / num⌋` of a lane (as the graph places the first sample of
a clip), and the points over a clip move by as many ticks as its start does, so that they stay
on the same samples of the clip. At a rate of frames whose frames are not whole ticks (29.97 fps
at 48 kHz) two clips moved by one number of frames can move by ticks that differ by one: a lane
whose sloped line crosses the start of such a clip is held there by points of its values first
(at the tick before the start and at the start), and the last tick of a clip may take the value
of what follows it. A point that an edit adds and that lies on the straight line between the
points beside it is left out. The values at the ends of what is taken away or moved are within
those that the lane had; a lane made on another track also takes the value that its setting is
set to there.

Trims, rolls, slips and slides clamp to available handles and space, reporting their input
index in `clamped`; zero delta or an impossible movement is refused. Editing through a
transition keeps it where its clips still meet, shortens it to available handles/coverage, or
removes it if none fits. A repair shortened to zero removes the transition and is also reported as clamped. Transitions and same-end
fades are not silently substituted. All changes are still one reversible transaction.

### Edits of sound

| edit | fields besides timeline | behavior |
|---|---|---|
| `set_track_audio` | `track`, `patch`, `follow_groups?` | As the operation; with `follow_groups` (true where left out; `false` is the Ctrl+Shift clutch, p. 3920), the other tracks of every active group of tracks that `track` is in follow what the group links of what changed. |
| `clip_gain_keys` | `clip`, `action: add\|remove\|move\|set_interp`, `at`, `param?: gain_db\|pan`, `to?`, `value?`, `interp?` | A keyframe of the volume (or the pan) of a clip, named by its time `at`, a time of the clip's media (pp. 3838–3840). |
| `clip_gain_range` | `clip`, `from`, `to`, `delta_db` | Raise or lower the volume between two times of the clip's media (p. 3840). |
| `nudge_clip_gain` | `clips`, `delta_db`, `linked?` | Move the volume of clips of sound by `delta_db`, keys and all (±1 / ±3 dB, p. 969; a relative drag, p. 3837). |
| `paste_attributes` | `from`, `to: [clips]`, `items: [volume\|plugins\|equalizer]` | Paste the volume (keys and all), the effects or the equalizer of a clip onto others (p. 3842). |
| `remove_attributes` | `clips`, `items` | Put them back as a new clip has them: 0 dB with no keys, no effects, the equalizer off (p. 3804). |
| `fade_to_playhead` | `clips`, `side: in\|out`, `at` | Fade each clip in from its start to frame `at`, or out from it to its end (p. 3812), in one transaction. |
| `set_bus` | `id`, `patch` | As the operation; a bus soloed while the solo is exclusive takes the solo off the others (below). |
| `copy_effects` | `from`, `to: [chains]` | Put the inserts of one chain (`{"track"}`, `{"bus"}` or `{"clip"}`, as `of` of the operations) in place of all those of others (pp. 3855–3860): only the inserts, not Dynamics nor the equalizer. The lanes of the inserts of a strip that they replace are taken away first. |
| `set_insert` | `of`, `slot`, `insert` (an insert or `null`, which must be written) | As the operation, with the lanes of the insert that goes (`insert.<id>.…`) taken away first; an insert of a strip put in place of another is given a new id, one above every id of the strip and the one it replaces. |
| `move_insert` | `of`, `from`, `to`, `to_of?` | Within `of` (or `to_of` equal to it), as the operation: the insert keeps its id and its lanes. To another chain `to_of`: its lanes taken away, the insert taken from `of` and added to `to_of` at `to` with an empty id, so it is a new insert there. |
| `lane_points` | `of`, `path`, `action: add\|remove\|move`, `points?`, `at?`, `by?`, `delta?`, `grouped?` | Points of a lane (pp. 3898–3900, 3911): add `points` (`[{at, value?}]`), take away or move the points at the times `at`. |
| `lane_range` | `of`, `path`, `from`, `to`, `action: delete\|cut\|paste\|shift_value\|thin`, `points?`, `delta?`, `tolerance?`, `grouped?` | A range of a lane (pp. 3898–3899): taken away, pasted over, raised or lowered, or thinned. |
| `lane_draw` | `of`, `path`, `from`, `to`, `points`, `tolerance?`, `grouped?` | The Pencil (pp. 3900, 3911): the points drawn, thinned, in place of the range. |
| `lane_fill` | `lanes: [{of, path, value}]`, `from`, `to`, `glide?` | Fill Range or Glide Range (pp. 3908, 3913; T16b), as `automation.fill` writes it. |
| `automation_paste` | `at`, `to?`, `lanes: [{of, path, points?, value?}]` | Paste of automation (pp. 3913–3914; T16b): what `automation.copy` gave, at `at` or over a range. |
| `automation_erase` | `of: [owners]`, `from`, `to` | Erase of automation (p. 3914; T16b): the points of the range taken away. |
| `clear_lanes` | `of`, `paths?`, `then?: keep\|reset\|hold`, `at?` | Clear Track Automation (p. 3912): the lanes of `paths`, or all, taken away. |
| `record_pass` | `lanes: [{of, path, trim?, runs: [{points, to, then: return\|hold\|event\|cut, glide?}]}]` | What a pass of the recording of automation recorded (T16), as the engine writes it at the stop or a Punch In (see Recording automation). |

`add_transition` takes `linked?` (false where it is left out): a transition of picture with
`linked: true` brings, in the same transaction, a crossfade of the same length and alignment at
each cut where the clips of sound linked to its two clips meet on a track that is not locked and
has no transition, as DaVinci Resolve does with linked selection on (p. 973); one that would need
more of a clip's media than there is is made as long as fits and the edit is `clamped`, and one
that fits no frame is left out. A cut where a clip of sound fades (`fade_out` or `audio_fade_out`
of the one that ends there, `fade_in` or `audio_fade_in` of the one that starts there) gets no
crossfade, since a transition does not go where a fade is, and the edit is `clamped`; the
transition of picture is made all the same. It is off by default so that a client that adds the crossfade
itself, as the migration of old projects does, is not refused with `cut_taken`.

**Groups of tracks (Ch. 179, p. 3917).** `set_track_audio` with `follow_groups` changes, besides
`track`, each other track of sound that shares an active group with it whose `linked` has the
control that changed: `fader` moves the follower's `strip.fader_db` by as much as the track's
moved, up to +10 dB (a level at −∞, −100 dB or below, counts as −100 dB: the track brought up
from −120 to −90 dB moved by 10 dB; and a follower at −∞ stays there while the group goes down);
`panning` moves each number of its `strip.pan` by as much (within its range: `x`, `y` −1…1,
`rotate` ±180, `spread` 1…99, `divergence` 0…100, `lfe_db` as a fader) and sets its switches to
the track's; `sends` moves the level of its send to each bus that the track's send to that bus
moved by as much (as a fader moves), and sets its `on` alike (sends are not added or taken away); `mute`, `solo` and
`automation` (the automation arm) are set to the track's. Nothing else follows; inserts,
Dynamics and the equalizer never do. Each follower is set by its own `set_track_audio` of only
what changes of it, in the one transaction, so one undo takes all of them back. A track that is
locked is not changed: as `track` it is refused as `locked`, and as a follower it is left alone.
While the timeline's `audio.exclusive_solo` is on (p. 3885), a track that is soloed by the edit
`set_track_audio`, or a bus by the edit `set_bus`, takes the solo off every other track that is
not locked (but those soloed with it by a group), every other bus and every VCA, in the same
transaction; the Solo Safe of a track stays, and a track that is solo-safe and soloed loses its
solo like any other. The operations set only what they are given.

**Keyframes.** `clip_gain_keys` `add` puts a key at `at` with `value` (the value of the setting
there where it is left out) and `interp` (linear where left out); one already at `at` (counted
exactly, whatever its scale) is given them instead. `remove` takes the key at `at` away; the
last one taken away leaves its value, fixed. `move` moves the key at `at` to `to`, and to `value`
where it is given; a key at `to` already is `out_of_range` (`to`). `set_interp` sets its `interp`.
A key that is not at `at` is `missing` (`what: "key"`, `id` as `ticks/scale`).
`clip_gain_range` puts keys on a grid of the timeline's sample rate laid on the time of the
clip's media (the scale of the keys is the sample rate): at the points of that grid nearest to
`from` and `to`, a pair at each end one point apart, the outer one at the value that was there and
the inner one at that value plus `delta_db`; every key between them moves by `delta_db`, those
outside stay, and any between the two of a pair are taken away. The grid falls on the samples of
the timeline only where the clip plays at speed 1 from a `source_in` on that grid. So a clip at 0
dB lowered by 6 over [1 s, 2 s) has keys at 1 s − 1 sample (0), 1 s (−6), 2 s − 1 sample (−6) and
2 s (0). A key that stands where one of the four goes keeps its interpolation; an inner key is
linear; an outer key that splits a segment carries the ease of the segment at its side: the outer
key before the range eases in where the key after it eased in, and the one after the range eases
out where the key before it eased out. Values outside the range are the same as before where the
segments are straight lines; under an ease they are close to it but not the same. `from` and
`to` must be within the media that the clip plays (else `out_of_range`, `from` or `to`); a range
of no point of the grid, or `delta_db: 0`, is `empty_range`.
`nudge_clip_gain` with `linked` (true where left out) takes the clips linked to those named too,
passes over clips that are not on tracks of sound (refused as `out_of_range`, `clips`, where none
is), and `delta_db: 0` is `empty_range`. A gain that `clip_gain_range` or `nudge_clip_gain` would
take out of −96…24 dB is kept at that end, and the edit is `clamped`.
`paste_attributes` puts the keys of the volume of `from` as far from where each clip starts on
the timeline, in its media, as they were from where `from` starts: from where a clip begins in its
media, or where it ends there for one that plays backward. A key 1 s into `from` is 1 s into the
clip that it is pasted onto, whichever way each plays; onto a clip that plays the other way the
keys are mirrored, and an ease in becomes an ease out. Times are exact, refused as `too_large`
(`at`) where one would not fit. `remove_attributes` sets only what differs. Both take no linked
clips; an empty `items` or list of clips is `out_of_range` (`items`, `clips`).
`clip_gain_keys`, `clip_gain_range`, `paste_attributes` (from and onto) and `remove_attributes`
refuse a clip that is not on a track of sound as `wrong_track` (`what: "sound"`), while
`nudge_clip_gain` passes such clips over; a key added or moved outside the media that the clip
plays is `out_of_range` (`at` or `to`). `fade_to_playhead` sets `audio_fade_in` or
`audio_fade_out` of a clip of sound (its length the frames from the clip's start to `at`, or from
`at` to its end, as a time of the timeline; its curve and skew kept), and `fade_in` or `fade_out`
of a clip of picture, of each clip named, in one transaction; `at` must be after each clip's first
frame and before its end, else `nothing_at`; a fade that would meet the other fade of its clip
(the two together longer than the clip) is `out_of_range` (`fade`), and one at a cut where a
transition is is `fade_at_transition`. `copy_effects` with an empty `to` is `out_of_range` (`to`); a chain that is `from`
is passed over; the copies are new inserts, which a strip gives the next ids of its own (so a
lane of the target's old inserts names an insert that is not there, and is refused as
`out_of_range`, `lane`, until it is taken away).

**Automation (T14, Ch. 178).** The edits of lanes name a lane by `of` and `path`, as the
operations do, and work in times of the timeline (`{ticks, scale}`), each put on the nearest tick
of the lane's scale (that of its points; the timeline's sample rate for a lane that is not there
yet); a time that falls outside 0…2^53−1 ticks there is `out_of_range` (`at`, `from`, `to`,
`points`, `by`). A lane of a track that is locked is `locked`; a path of no lane is refused as the
operations refuse it (`lane`, `lane_integer`, `lane_not_wired`). Each is one transaction: a lane
made is one `set_lane`, one taken away one `set_lane` of `null`, and any other change one
`splice_lane` of the points from the first that changes to the last; an edit that would change
nothing (a point added where one of that value is, a lane already as thin as asked) is
`empty_range`. A value of a switch is on
(1) at 0.5 and above and off (0) below; a value of a number past the range of its setting is kept
at the end of the range, and the edit is `clamped`.
- `lane_points` `add` puts each of `points` at its time, after any there already, or, where one
  is there, sets the value of the last one there; a point given no `value` takes the value that
  the lane has at its time, so nothing is heard to change (p. 3898). Where the lane is not there,
  it is made, of the kind of its setting, from the value its setting has. `remove` takes away
  every point at each time of `at`; the last taken away takes the lane away, and the setting is
  heard again as it is set. `move` moves every point at the times of `at` by `by` in time (back
  where its ticks are below 0) and by `delta` in value: each run of points moved stays between the
  points beside it and within the timeline, as far as it can (`clamped` where it stops). A time of
  `at` with no point is `missing` (`what: "point"`, `id` the time as `ticks/scale`); a lane that is
  not there is `missing` (`what: "lane"`); `move` with no change is `empty_range`; `points` or `at`
  empty is `out_of_range` (`points`, `at`).
- `lane_range` works on the range from `from` to `to` (`from` before `to`, else `empty_range`).
  The value before `from` and the value from `to` on stay as they were (pp. 3898–3899): the points
  from `from` to `to` (both in) are put in place of by a point at `from` of the value that the lane
  came to it with, what the action makes, and a point at `to` of the value that was there. `delete`
  and `cut` (the same; a client keeps what it cut) leave a straight line between those two. `paste`
  puts `points` (times from `from`, in order, none past `to`; `out_of_range: points` otherwise) in
  between. `shift_value` moves the values of the range by `delta` (0 is `empty_range`): at each end
  a point of the value that was there and, 10 ms within (half the range where it is shorter than
  20 ms), one of the value moved, so it ramps in and out (C-A6); the points between the two within
  move by `delta`, and those within 10 ms of an end go. A switch jumps instead, at `from` and at
  `to`. `thin` keeps of the points from `from` to `to` those that Douglas–Peucker keeps within
  `tolerance` (see `lane_draw`). `delete`, `cut` and `thin` of a lane that is not there are
  `missing`; `paste` and `shift_value` make it, from the value of its setting.
- `lane_draw` puts `points` (times from `from` to `to`, in order; `out_of_range: points`
  otherwise), thinned, in place of the range from `from` to `to`, as `lane_range` `paste` does.
  Thinning (C-A4) keeps the first and the last point and both points of a jump (two at one time),
  and between them, by Douglas–Peucker, each point without which the lane would go further than
  `tolerance` from a point drawn, in value at its time as its scale measures it (in logarithms
  on a scale of logarithms): where `tolerance` is left out, 0.1 dB for a fader or the level of a
  send, 0.5 % of the range of a pan (`pan.*`, and the pan of a send: 0.01 of `x`), and 0.1 % of
  the range of any other setting as its control shows it (of the logarithms of its ends for a
  frequency or a Q); of a switch, the points where its value changes. The constants are
  `LANE_THIN_*` and `LANE_RAMP_PER_SECOND` of `crates/model/src/audio/automation.rs`. A negative
  or not finite `tolerance` is `out_of_range` (`tolerance`).
- `grouped: true` on `lane_points`, `lane_range` or `lane_draw` of a track (T16b, interaction
  reference §11.6) makes the same edit of the same lane of each other track, not locked, of an
  active group with it that links `automation` and the control of the setting (`fader_db`:
  `fader`; `mute`; `pan.*`: `panning`; `send.*`: `sends`; no other setting follows, a plugin's
  never, p. 3917), in the same transaction: a number moved relatively, each value given plus how
  far that track's setting (its lane, or as it is set) is from this one's at `from` (of
  `lane_points`, at the time of its first point) before the edit; a level (`fader_db`, any
  `*_db`) as a fader of a group follows another (§4.2): by the same change in decibels from this
  one's, −∞ (−100 dB and below) counted as −100, one at −∞ that would go down staying, no higher
  than +10 dB (the same while recording); a switch to the same state; `shift_value` by the same `delta`, and `delete`,
  `cut`, `thin`, `remove` and `move` as they are. A track that the edit cannot be made to (no
  such setting, lane or point, or nothing would change) is passed over. Not where it is left
  out.
- `clear_lanes` takes away the lanes of `paths` (each must be there, else `missing`; an empty
  list is `out_of_range: paths`), or every lane of `of` where it is left out, as one step of undo
  (Clear Track Automation, p. 3912). `then` says what each setting is left at: `keep` (where it is
  left out) as it is set; `reset` as a new strip has it (an insert as a new effect of its kind, a
  send as a new send to its bus; a VCA at 0 dB and not muted); `hold` at the value of its lane at
  `at` (required: `out_of_range: at`). The settings are set by one `set_track_audio`, `set_bus`
  or `set_vca` of what changes, after the lanes are gone.
- The edits `set_track_audio` and `set_bus` take away, first, the lanes that their patch would
  leave naming a setting that is no longer there or is of another kind: of a send, an output or
  an insert that a patch of `sends`, `outputs`, `inserts` or `track_fx` takes away, of a Ducker
  gone, or of an insert that keeps its id with an effect of another kind. A lane left out of its
  range by a patch is still refused.

### Recording automation

The timeline monitor records automation as it plays (T16, pp. 3900–3909), as the controls of
the mix are moved: what a pass writes is heard at once, and written into the lanes when the pass
ends, as one transaction and one step of undo (`record_pass`, made by the kind of client that
turned recording on; passes are written one after the other in the order they end). How it
records is the timeline's `audio.record` (set by `set_timeline_audio`): `mode`, `touch`,
`on_stop`, `categories`, `preview` and `active_range`, and the `automation_arm` of each track,
bus and VCA. The Glide Time (C-A1) is the client's, not the project's: `automation.record`
takes it. Loading a timeline in the monitor (or unloading it) turns recording off, drops a pass
under way and what Preview kept unwritten, and lets go of what is held.

- **`automation.record`** `{timeline, on, glide_ms?}` turns recording of the timeline loaded in
  the timeline monitor on or off (`-32004 {reason:"not_loaded"}` for another), its Glide Time
  `glide_ms` 0–10 000 (250 where it was never given; `-32602 {reason:"glide_ms"}` outside). The
  state says `recording: true` while it is on. On, a **pass** is recorded while the timeline
  plays forward at speed 1 only: it begins where it starts to play so (`transport.play`, or a
  seek while it plays), at once where it plays so already, and otherwise at the first touch once
  it plays at speed 1 again; it ends, and is written, where it stops (pause, the end of the
  timeline, a seek, a load), where its speed is changed (at the sample heard when the change is
  asked for, before the sound changes speed), where scrubbing plays a grain, or where recording
  is turned off. A seek while playing is so two passes, each a step of undo. At another speed a
  touch is heard and not recorded. A pass begins only where the sound plays (a session of the
  device); with no sound, nothing is recorded.
- **`automation.touch`** `{timeline, track|bus|vca, path, value, touching}` is a control of the
  mix touched (`touching: true`, sent as it moves, every 10–20 ms) or let go (`false`), at the
  sample of the timeline heard when the engine takes it (by the device's clock at the instant the
  request is read, not when the monitor gets to it); it is answered at once. `path` is a path
  of a lane of the owner (see Automation; `-32602 {reason:"path"}` otherwise, `missing` for an
  owner or a timeline that is not there, `owner` where not one of track, bus and vca is named,
  `value` where it is not a number), and `value` is in the units of the setting (a switch on at
  0.5 and above), kept within the range of the setting as it is heard and written (+120 dB is
  +10 dB; a frequency of 0 Hz the least of its band), but for a fader (`fader_db`) under Trim,
  where it is the decibels that the lane moves by (the fader at 0 dB is no change, p. 3901),
  kept within ±110 dB. Refused with
  `-32004 {reason:"not_recording"}` while recording is off.
- **Heard at once.** A control touched takes the place of its lane (or of its setting, where it
  has no lane) from the next block of the mix, within 10 ms, as it plays; under Trim the lane is
  moved by the value instead (−∞, −100 dB and below, stays; no higher than +10 dB). Let go under
  Latch it holds its last value; under Snap the lane takes its place back over the Glide Time
  (10 ms at least) on a straight line on the lane's own scale, as it is written (the logarithm of
  a frequency or a Q; a setting with no lane glides back to its value, a gain in the gain; a
  switch at once, and a parameter of Dynamics, the equalizer or an insert with no lane over
  10 ms). A value that moves ramps to its new value over 10 ms. The value is held by every graph
  swapped in as the project changes while it plays, at any speed. Nothing of the project
  changes until the pass ends, and no graph is built for a touch. When a pass ends, what is held
  is let go of over 10 ms (but under `preview`), as is what a Punch In writes. While the timeline
  does not play, a touch changes nothing, but under `preview`, where it is held for the next
  play; a touch of a timeline other than the one loaded is passed over.
- **What a pass writes**, of each setting touched where its category is on (Fader: `fader_db`;
  Mute; Pan: `pan.*`; EQ: `eq.*`; Comp: `dynamics.*` but Gate and Lim; Gate:
  `dynamics.expander.*`; Lim: `dynamics.limiter.*`; Sends: `send.*`; Plugins: `insert.*` and
  `track_fx.*`; Misc: the rest, the outputs and an insert on and off, Trim, the polarity) and,
  under Touch Off, of an armed track, bus or VCA only (`automation_arm`): the values recorded
  from where it was touched, at the samples heard, points a millisecond apart at least (a later
  one taking the value of the one before), thinned as a drawing is (C-A4). A value that comes
  after the control was let go, or more than 10 ms after the last (a value typed, a step of the
  wheel, a drag that paused), was heard as a step ramped over 10 ms, and is written so: the last
  value held to it, the new one 10 ms later (a switch steps).
  - `touch` (when): `off` as `latch`, on armed owners only; `latch`, from the touch to the end of
    the pass; `snap`, from the touch to its release, then back to the lane over the Glide Time,
    and again from the next touch; `snap_latch`, a fader (`fader_db`) as `snap`, the rest as
    `latch`.
  - `mode`: `write` writes the values; `trim` writes, for a fader, the lane as it was moved by
    the offsets recorded (its −∞ staying, kept to +10 dB), and for any other setting the values,
    as `write` does (interaction reference §11.3).
  - `on_stop` (after the last value, where it is still written at the end of the pass):
    `return` back to the lane over the Glide Time (C-A2); `hold` the last value to the end of the
    timeline, the lane after it taken away; `event` the last value to the next point of the lane
    after the end, the lane as it was from there (as `hold` where there is none).
  - `active_range` with `on`: only from `start` to `end` is written. A run touched before the
    start begins there at the value it had there; one that goes past the end is cut there, the
    lane given back at once; a glide or a Hold or Event that would go past it ends there. When the
    timeline passes the end, what is held is let go of over 10 ms (Punch Out, p. 3908), and what
    is touched after it is heard and not written.
  - `preview`: a pass is heard and not written at its end; what it recorded is kept, and what is
    held stays held across the stop and the next play. **`automation.punch`** `{timeline}`
    writes all that is kept of `timeline` as one transaction (Punch In, p. 3908) and answers how
    many lanes it gave to be written (0 where none of that timeline were kept); turning recording
    off, loading a timeline, or a pass of another timeline, lets go of what was kept. The
    offsets of a Trim kept by several passes are each of the lane as it was before all of them
    (as they were heard, nothing being written in between), the later taking the place of the
    earlier where it writes.
- **The Automation Arm** (`automation_arm` of a track, bus or VCA; T16b, interaction reference
  §11.4): armed (`true`) is recorded under Touch Off too; `"safe"` plays its lanes back but is
  never recorded, whatever the Touch, nor changed by `lane_fill`, `automation_paste` or
  `automation_erase` (skipped where they name it); `"off"` does not play its lanes back (the
  graph reads none of them: its settings stay as they are set) and is never recorded. A touch of
  either is still heard.
- **`automation.all_read`** `{timeline}` (All Read, p. 3913) ends, at the sample heard, every
  run being written (as at the end of the Active Range: the lane given back over 10 ms), lets go
  of what is held over 10 ms, and from then on what is touched is heard and not written, until
  `automation.record` turns recording on again, `automation.punch` or `automation.join`.
  `{"read": false}` where recording is off (nothing changes). Whether it is on is `read` in the
  answer of `automation.record` and `all_read` in the state of the transport.
- **`automation.join`** `{timeline}` (Join Mix, p. 3913): where the last pass that was written
  (not under Preview) stopped, and the last values of what it was still writing at its stop (not
  a Trim's offsets), are kept. Asked, the next pass, where the heard position reaches that stop
  (at once where it starts after it), touches each of those settings at its last value and
  writes it on as `latch` does whatever the `touch` (heard as held, written to the end of the
  pass, by its `on_stop`), once. Answers how many settings and the stop (`{"lanes": 0, "at":
  null}` where there is none of that timeline).
- **`automation.fill`** `{timeline, from, to, glide?}` (Fill Range, Glide Range; pp. 3908,
  3913), while `preview` is on: of the settings touched under it (their last values, a fader
  under Trim excepted), those that may be written (the setting still there, its track not locked,
  its owner neither Safe nor Off, its category on) are written over the range by the edit
  `lane_fill`, as one transaction (told by `automation.written`, its `lanes` those written, and
  its `failed` those that could not be), let go of over 10 ms as their lanes take their place,
  and dropped from what Preview kept to be punched in; the others stay held, to be filled later.
  A pass under way ends first (kept, under Preview). Answers how many lanes it gave to be
  written (0 where Preview is off or holds none). `from` not before `to` is refused (`-32602`,
  `reason: range`). **`lane_fill`** `{lanes: [{of, path, value}], from, to, glide?}`: each lane is written
  `value` from `from` to `to`; with `glide`, from the value the lane has at `from` on a straight
  line to `value` at `to` (a switch is filled). A number ramps in over the first 10 ms of the
  range and back to the lane over the 10 ms after `to`, as `record_pass` with a `cut` run
  writes it. A Safe or Off owner is skipped; nothing written is `empty_range`.
- **`automation.copy`** `{timeline, of: [{track|bus|vca}], from?, to?, at?}` (Copy, p. 3913)
  answers, of each owner named (a Safe one too), each lane of a category of recording that is
  on: with `from` and `to`, `points` of the range at times from `from` in the lane's scale (its
  value at `from` the first, the points within, its value at `to` the last); with `at` alone,
  `value`, the lane's value there (a snapshot), and also, of each fader, mute, pan and send of a
  category that is on and has no lane, its `value` as it is set, so that a static mix is copied.
  A range copies only lanes that are there. Given neither, or a range and `at`, it is refused
  (`-32602`, `out_of_range` `at`); one of `from` and `to` alone, `reason: range`; a `from` or `to`
  out of the timeline, `out_of_range` `from` or `to`. The clipboard is the client's: **`automation_paste`** `{at, to?, lanes: [{of, path,
  points?, value?}]}` puts each lane (any track, bus or VCA, and any setting: a fader's level to
  a send's and back, kept within its range) at `at`: with `to`, it fills the range, `points` as
  far as they go (cut at `to`) and their last value held to `to`, a `value` over all of it; with
  none, `points` whole from `at` (a lane whose points have no length is passed over), and a
  `value` is refused (`out_of_range` `to`: a snapshot fills a range). What was in the range is put in
  place of, the values before it and from its end on kept, as `lane_range` `paste` does. A Safe
  owner, and a lane of a category that is off, is skipped. **`automation_erase`** `{of: [owners],
  from, to}` (Erase, p. 3914) takes away the points from `from` to `to` of every lane of the
  categories that are on of each owner, not Safe, as `lane_range` `delete` does. Nothing changed
  is `empty_range`; a lane that is not there, or a point out of order, is `out_of_range`
  (`points`).
- **`automation.written`** (a notification to every connection) tells each pass written, and a
  Punch In (and a Fill Range or Glide Range, `punch: false`): `{"timeline", "punch": false, "lanes": n, "revision": r | null, "failed": [{"track" |
  "bus" | "vca": id, "path", "reason"}, ...]}`. A lane whose track, bus, VCA or setting is gone,
  or that the project refuses (a track locked since, say), is left out so that the rest of the
  pass is written, and is told in `failed` with the refusal's `reason`; a lane that would change
  nothing is left out silently; `revision` is that of the transaction, `null` where nothing was
  written. What a Punch In could not write of a setting that is still there is kept again, to be
  punched in once more.
- **`record_pass`** `{lanes: [{of, path, trim?, runs: [{points, to, then, glide?}]}]}`, the edit
  that a pass is written as, may also be sent by a client. For each lane (made from the value of
  its setting where it is not there), each run in the order of time puts `points` (times of the
  timeline, in order, from where the run starts, none after `to`; values kept within the range of
  the setting, a switch on or off; thinned by the default tolerance of `lane_draw`) in place of
  the lane from its first point, the value before it staying as it was, its last value held to
  `to`, and then: `return` back to the lane over `glide` (a time, 10 ms at least; none for a
  switch), as a straight line to the value the lane has there; `hold` nothing more of the lane;
  `event` the last value to the next point of the lane after `to`, into it over the last 10 ms;
  `cut` the lane as it was from `to` on, over 10 ms. A run of a number comes in from the value
  the lane had over 10 ms (its first value 10 ms after its first point, C-T1), as it was heard;
  a switch jumps. A later run takes the place of what an earlier one wrote from where it starts.
  With `trim` (a lane of decibels, `*_db`, only; `out_of_range: trim` otherwise), the points are
  offsets added to the lane as it was before the edit (a setting given twice: before the first),
  where it is above −100 dB, at its points and theirs; the offset ramps in from 0 over 10 ms,
  `return` takes it back to 0 over `glide`, `cut` over 10 ms. Refused as
  `out_of_range` with `lanes` (none), `runs` (none, or one starting before the one before it),
  `points` (none, out of order, or after `to`), `value`, `to`, `glide`, `trim`, and as the edits
  of lanes refuse a path or an owner.

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

A **node** is `{id, kind, label, enabled, locked, pos, params}`. `id` is unique within its grade
(not in the project: copying a grade keeps its ids), never `source` or `output`; the engine makes
`n` + `next_id`. `label` is at most 100 characters (an editor shows 12, p. 3313). A disabled node
passes its picture through. The engine always writes `locked` and `pos`, and reads a node
without them as `false` and `null`:

- `locked` is Lock Node (pp. 3297, 3382: "prevent any changes from being made to that track's
  corresponding node", "preventing you from accidentally making adjustments to it"). A locked
  node's `params` are not set (`node_locked`): not by `set_node` `params`, and so not by the
  edits that set them, `reset_node`, `paste_value`, `paste_node` onto it, or a mixer given an
  input by `add_parallel` / `add_layer`. Its label, `enabled` (the button beside the lock in the
  Keyframe Editor, p. 3382), `pos`, its links, and whether it is in the grade at all
  (`remove_node`, `extract_node`, a grade put in whole by `set_grade`, `copy_grade` or
  `collapse_group_grades`) stay free, as the manual names only adjustments. Copied from, it
  gives its settings as any node does.
- `pos` is `{x, y}`, where the node lies in the node editor: the top left corner of its card, in
  the editor's units (a pixel at zoom 1, `x` right, `y` down), each finite and within 10^6 of 0
  (`out_of_range` `pos`); or `null`, where the editor lays it out. DaVinci keeps the layout of a
  node graph with the grade, so that it is found as it was left by the colourist who comes back
  to it or another (p. 3321). It is kept on the node, not in a table of the grade, so that it
  goes wherever the node goes: `remove_node` and its inverse, `extract_node`, `copy_grade`,
  `set_grade`, groups. `copy_grade` with `preserve_nodes` gives a kept node the `pos` of the node
  of the copy whose place it takes, as the layout is the copy's. `append_node_graph` and
  `collapse_group_grades` move the nodes they put after others that have a `pos` right, so that
  the leftmost of them is 148 (a column of the studio's editor) right of the rightmost node
  before them, a node without a `pos` there counted as far right as the editor could lay it (at
  `102 + (n − 1) · 148` for `n` nodes); into a grade without nodes they keep theirs. A node
  without a `pos` stays without one. An editor sends a drag as one transaction of `set_node`
  `pos` (one step to undo), and Cleanup Node Graph (p. 3321) as one `set_grade` with every `pos`
  `null`.

`params` by `kind`:

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
| `blur` | at rest | the Blur palette, below (pp. 3454–3456) |
| `key.input` `{invert, matte, gain, offset}` | `false`, `true`, 1, 0 | gain 0…2, offset −1…1 (p. 3362) |
| `key.output` `{invert, gain, offset}` | `false`, 1, 0 | gain 0…2 (p. 3362), offset −1…1 |
| `key.qualifier` `{invert, matte, gain, offset}` | `false`, `true`, 1, 0 | as `key.input` (p. 3362) |
| `key.input_blur` `{radius, ratio}` | 0, 0.5 | radius 0…2000, ratio 0…1 (p. 3362) |
| `shot_match` | `null` | `null` or `{gamma, gain, offset, contrast, pivot}`, below |
| `qualifier` | `null` | `null` or the Qualifier palette, below |
| `windows` | `[]` | the ordered Power Window list, below |

An Outside node is a corrector with `key.input.invert: true` linked to the key of the node
before it (p. 3361); `add_outside` adds one. Within a corrector the settings apply in the order
of p. 3331. The engine writes `key.qualifier`, `key.input_blur`, `qualifier`, `windows` and `blur` always, and reads
a corrector without them as they are at first.

**`qualifier`** is the Qualifier palette (pp. 3157–3182): the node's own key, made of the
picture that comes into the node (R'G'B' of the working space, held within 0…1). `null`, as a
node is until a colour is picked, is no key of its own. As it is given first it is every pixel,
written whole:

```json
{"mode": "hsl", "invert": false,
 "hue": {"on": true, "center": 50, "width": 100, "soft": 0, "symmetry": 50},
 "sat": {"on": true, "low": 0, "high": 100, "low_soft": 0, "high_soft": 0},
 "lum": {...as sat}, "red": {...}, "green": {...}, "blue": {...},
 "finesse": {"pre_filter": 0, "clean_black": 0, "black_clip": 0, "clean_white": 0,
             "white_clip": 100, "blur_radius": 0, "in_out_ratio": 0,
             "morph": "shrink", "morph_radius": 0}}
```

| field | default | range | what it is |
|---|---|---|---|
| `mode` | `hsl` (p. 3167) | `hsl`, `rgb`, `lum` | which components key: hue, saturation and luma; red, green and blue; or luma alone (p. 3175: `lum` is `hsl` with hue and saturation off, and shares its `lum`). The 3D qualifier is not kept. |
| `invert` | `false` | | the palette's Invert (p. 3182), after Matte Finesse |
| `hue.on`, `sat.on`, …, `blue.on` | `true` | | whether the component limits the key (the toggles of p. 3173) |
| `hue.center` | 50 | 0…100 | a turn of hue: red 0, green 33.3, blue 66.7, going round |
| `hue.width` | 100 | 0…100 | the hues kept, about the centre; 100 every hue |
| `hue.soft` | 0 | 0…100 | the fall beyond them each side, in hundredths of a turn |
| `hue.symmetry` | 50 | 0…100 | lower, the right (higher hues) steeper; higher, the left (p. 3173): the left fall is `soft · (100 − symmetry)/50`, the right `soft · symmetry/50` |
| `sat`, `lum`, `red`, `green`, `blue` `.low` / `.high` | 0 / 100 | 0…100 | the range kept, black to white (saturation `(max − min)/max`, luma by the weights of the working space) |
| `….low_soft` / `.high_soft` | 0 | 0…100 | the fall below `low` and above `high` |
| `finesse.pre_filter` | 0 | 0…100 | a Gaussian of `0.05 ×` it pixels (of a frame 1080 high) over the picture before it is keyed (p. 3178) |
| `finesse.clean_black`, `clean_white` | 0 | 0…100 | specks out of the black, holes out of the white (pp. 3178–3179), below |
| `finesse.black_clip`, `white_clip` | 0, 100 (pp. 3179–3180) | 0…100 | the levels of the key from Black Clip to White Clip made 0 to 1 |
| `finesse.blur_radius` | 0 (p. 3180) | 0…2000 | a Gaussian of `0.1 ×` it pixels over the key |
| `finesse.in_out_ratio` | 0 | −100…100 | the blur both sides of the edge at 0, inside alone at −100, outside alone at 100 (p. 3180) |
| `finesse.morph` | `shrink` | `shrink`, `grow`, `opening`, `closing` | Morph Operation (p. 3181) |
| `finesse.morph_radius` | 0 | 0…100 | its radius, in pixels of a frame 1080 high |

Each number is a parameter (keyframes too); `mode`, `invert`, the toggles and `morph` are not. A
qualifier is set with `set_node`: given whole where the node has none (a part of one onto `null`
is `params`), patched in part where it has one, and taken away with `{"qualifier": null}`; out
of range is `out_of_range` with the path, such as `qualifier.finesse.blur_radius`. `reset_node`
takes it away.

**`windows`** keeps the Power Window palette (Resolve 20, pp. 3185–3199). Each corrector owns
an ordered list of complete windows; `[]`, including an older file without this field, has no
window key. A window's `id` is stable and unique only in that node; copying a grade or loading
a preset keeps these ids without using the project's id counter. The schema is:

```json
{"id": "w1", "label": "Subject", "enabled": true, "invert": false, "combine": "add",
 "opacity": 100,
 "transform": {"size": 50, "aspect": 50, "pan": 50, "tilt": 50, "rotate": 0},
 "shape": {"kind": "circular", "softness": 1}}
```

`combine` is `add` (Mask on, the default) or `subtract` (Mask off). This is the project's
explicit interpretation of conflicting Mask descriptions on pp. 3190 and 3197. `label` has
at most 100 characters; `id` is nonempty and cannot start with `$`. `enabled`, `invert`,
`combine`, ids, labels and Curve's `closed` are not animated. Every geometry, transform,
softness and opacity number is a `Param`: a fixed number or `{"keys": [...]}`. As with the
rest of the grade, clip window keys use source frames, timeline window keys timeline frames.
Keyframe-copy remapping includes points and Bezier handles. Tracking data is separate from
the manual window transform: an optional `tracking` field, left out when there is none (see
Window tracking).

| field | default | range |
|---|---|---|
| `transform.size` | 50 | 0…1000 |
| `transform.aspect` | 50 | 0…100 |
| `transform.pan`, `transform.tilt` | 50 | −1000…1000 |
| `transform.rotate` | 0 | −36000…36000 degrees |
| `opacity` | 100 | 0…100 |
| `shape.softness` (or each Linear element) | 1 | 0…100 |
| `shape.inside`, `shape.outside` | 0 | 0…100 |
| each point or handle `x`, `y` | shape-dependent | −16…16 image heights |

Only the Transform defaults and increasing Pan right, Tilt up and Rotate clockwise are
documented by the manual (pp. 3191–3192). **All ranges in this table, Opacity 100, Soft 1,
the initial geometry and the coordinate formulas below are project choices, not published
Resolve values.** They are centralized in `model::grade::window::bounds` for real-application
comparison. Transform values are clamped to their ranges after interpolation, including
Bezier overshoot. The stored parameters and key values themselves must be in range.

`Point` below means `{"x": Param, "y": Param}`. Shapes are tagged by `kind`:

| `kind` | additional fields | initial geometry and rules |
|---|---|---|
| `linear` | `points: [Point; 4]`, `softness: [Param; 4]` | corners top-left, top-right, bottom-right, bottom-left at `(±0.25, ±0.25)`; independent Soft 1–4 for edges 0→1, 1→2, 2→3, 3→0 |
| `circular` | `softness: Param` | a local circle of radius 0.25; Transform Aspect makes it elliptical |
| `polygon` | `points: Point[]`, `inside: Param`, `outside: Param` | starts as the same rectangle; 3…128 points (the upper limit is documented on p. 3194); insertion and deletion change the list |
| `curve` | `points: {position: Point, in: Point, out: Point}[]`, `closed: bool`, `inside: Param`, `outside: Param` | starts empty and open; handles are offsets from their anchor, both zero makes a corner; a closed path has at least 2 anchors, without the Polygon-specific 128-point limit |
| `gradient` | `softness: Param` | through the local origin, white on negative local y and black on positive local y; Transform sets its centre and direction |

Outline point order is preserved, with even-odd filling for self-intersections. An unfinished
Curve is saved as a draft and does not participate in the key, even if inverted; neither
does a disabled window. Shape topology and point count are not interpolated. A preset
applies a copy of the whole list; removing the user-level preset cannot change the project.

Local coordinates have origin at the shape's centre, x right, y down, and one unit equal to
the input picture's height. Let `A = picture width / height`, `s = size / 50`,
`r = 2^((aspect − 50) / 50)`, and `R` be clockwise rotation in y-down coordinates. The map is:
`q = R · (s·r·x, s/r·y)`, `u = pan/100 + q.x/A`, `v = 1 − tilt/100 + q.y`.
This keeps the default circle round in landscape and portrait images. Clip grades use the
source picture's dimensions; timeline grades use the canvas. Viewer controls then apply the
same clip-to-canvas placement, black bars, zoom and display scaling as the picture. None of
those viewer pixels is saved in the window. `WindowTransform.affine_at` and `inverse_at`
share six coefficients `[a,b,c,d,tx,ty]`: `u=a*x+c*y+tx`, `v=b*x+d*y+ty`.

Size 0 collapses the forward matrix to the centre; `inverse_at` returns `None`. Its raw mask
is 0, then Invert, Opacity and Add/Subtract apply in that order, so an inverted collapsed
window can cover the whole picture. This differs from an ignored draft or disabled window.
Invalid aspect/frame inputs and unrepresentable transforms also return `None` from the
helpers. No enabled complete windows leaves the node's previous key behavior unchanged.

**Window rendering (project formulas).** Masks use the shown input picture after metadata
rotation, before clip placement/scaling; timeline masks use the canvas. Generated Title and
Color clips use their complete transparent source canvas, not the visible glyph/texture
bounding box. Cropping a title for rasterization keeps its original canvas coordinates for
windows; neighbourhood lookup remains in the cropped texture. Its dimensions used by windows
provide the source aspect ratio, not an antialiasing pixel-size estimate. All feather distances
are local image-height units and therefore follow the Window Transform. Circular and Gradient
Soft `s` have total transition width `0.5*s/100`, centered on the boundary. Circular uses signed
radial distance, positive inside; Gradient uses `-local_y`, with negative y white at Soft 0.
Polygon and Curve use nearest-outline distance, positive for even-odd inside: their inside
and outside reaches are `0.5*inside/100` and `0.5*outside/100`. Each uses
`smoothstep(-outside_reach, inside_reach, signed_distance)`; zero total reach is a hard fill.
Linear uses its four segment distances independently with half of each edge's transition
width on either side: inside takes the minimum of the four inward ramps, outside the maximum
of the four outward ramps. This avoids nearest-edge switching seams. At corners a soft edge
can contribute beyond the adjacent hard edge's extension; this is the project's corner
blend, not a published Resolve algorithm.

Each enabled complete window's raw mask is inverted if requested, then multiplied by
`opacity/100`. Starting at 0, Add is `min(1, accumulated + mask)` and Subtract is
`max(0, accumulated - mask)`, in list order. The resulting window key meets the processed
qualifier key by `key.qualifier.matte`: product in Matte mode, saturated sum in Mask mode.
With only one of these internal keys, that key is used. The internal key next meets the
processed external key via `key.input.matte`, then `key.output` applies. These same keys feed
Highlight/BW, Key Mixers and Layer Mixers. Windows limit the correction's RGB contribution;
they neither change image alpha nor clamp HDR RGB. Later neighbourhood operations read the
preceding windowed result at each input-image position, including uniform colour sources.

Curve cubics use their relative handles and adaptive double-precision de Casteljau subdivision.
The control-hull-to-chord bound after the largest Window Transform scale is at most
`1/(8*16384)` image heights: at most 0.125 pixels for an image 16384 pixels high, scaling with
height beyond that. This is the tessellation bound; f32 packing and evaluation have additional
rounding (the extreme-control/40x-scale regression measured about 0.267 pixels at 16384 high).
A threaded bounding-box tree prunes even-odd crossing and distance queries; distances beyond
the feather support do not need searching. Window/Curve counts have no extra rendering cap.
Geometry, transforms, softness, opacity and point counts are parameter-buffer data, so animated
values and point-count edits do not compile pipelines. A node with no enabled complete windows
keeps the original corrector block and shader path.

The positionless CPU `evaluate`, `evaluate_with` and `evaluate_showing` APIs ignore spatial
window constraints; they do not invent a sample position. `evaluate_picture[_showing]` evaluates
positions throughout the graph, including neighbourhood sources. Generate LUT excludes an
entire enabled node with any enabled complete window, including Size 0 or Opacity 0. When
several exclusion reasons apply, `why` reports the first in the order `qualifier`, `windows`,
`midtone_detail`; a windowed node without a qualifier therefore reports `why: "windows"`.
Disabled windows and open Curve drafts do not cause exclusion.

`set_node` applies `{"windows": [complete windows...]}` as one atomic list replacement.
Adding, removing, reordering, editing a point or applying a preset all use that operation;
an object patch indexed into the array is not supported. `{"windows": []}` clears the list
and `reset_node` clears it with the other parameters. Undo restores the previous whole list;
a locked node refuses the change. Unknown fields or malformed shapes are `params`, invalid
numeric values or counts are `out_of_range`, duplicate ids are `taken` with `what: "window"`.

**The key of a corrector**, in this order (the engine's `compose::grade` has the formulas; the
manual gives none):

1. The qualifier keys each pixel of the node's input (blurred first by Pre-Filter): the product
   of how much each component that is on lies within its range, 1 within it, falling smoothly
   (a smooth step) to 0 over the softness beyond, at once where the softness is 0; hue by its
   distance round the circle from the centre.
2. Clean Black and Clean White weigh the key `q` against its mean `m` about the pixel (a
   Gaussian of 3 pixels of a frame 1080 high): `q · smoothstep(0, cb, m)`, then
   `1 − (1 − q) · smoothstep(0, cw, 1 − m)`, `cb` and `cw` over 100: a speck in the black and a
   hole in the white go, translucent edges are eroded or grown, and dark or light greys pushed
   to black or white.
3. Black Clip and White Clip: `clamp((q − bc)/(wc − bc))`.
4. Blur Radius and In/Out Ratio `t` (over 100): the key through its Gaussian `b`, then
   `clamp((b − lo)/(hi − lo))` with `lo = max(−t, 0)/2`, `hi = 1 − max(t, 0)/2`; with In/Out Ratio
   not 0 the Gaussian is at least 2 pixels, so that it fills small holes or takes small specks
   even without a blur (p. 3181).
5. Morph Operation: the mean `a` of the key in the box of Morph Radius about the pixel; grown,
   `max(q, clamp(a / 0.05))`, shrunk, `min(q, 1 − clamp((1 − a) / 0.05))`: the edge moves by the
   radius, about `0.05 · (2r + 1)` pixels soft. Opening shrinks then grows, Closing grows then
   shrinks.
6. The palette's `invert`; then the Key palette's `key.qualifier` (invert, then offset with white
   held, `k + o(1 − k)`, then gain, held within 0…1). `key.qualifier.matte` tells how it meets
   the node's windows by product in Matte mode or saturated sum in Mask mode (formulas above).
   With only one internal key, that key is used.
7. The key coming in at the node's key input (if one is linked), blurred by `key.input_blur` (a
   Gaussian of `0.1 × radius` pixels, `2 · ratio` of it across and `2 · (1 − ratio)` down),
   through `key.input`, and met with the internal qualifier/window key (`key.input.matte` true, Matte mode: the
   product) or added to it (Mask mode: the sum, held at 1) (p. 3362). Without an internal key, the
   key coming in is the key; without either, the key is full.
8. `key.output` (invert, offset, gain). The node gives `input + (made − input) · k` with this key,
   and it is what goes out of its key output along key links, into other nodes' key inputs and
   key mixers.

Every part of it is the identity at its default, to the bit: a node without a qualifier, or with
one as it is first, changes nothing that the rest of the node does not. Steps 1, 3 and 6–8 are of
the pixel alone; Pre-Filter, Clean, the blur and Morph read the pixels about each (the
neighbourhoods of `compose`), each only where its settings are ever other than their defaults
(a setting keyframed through its default reads them at every frame). The shape of the grade (a
qualifier or none, which of those neighbourhoods, one pass of Morph or two) makes the shader;
every number, the mode, the toggles and the inverts are parameters, so a drag or a keyframe
compiles nothing.

**`shot_match`** is the adjustment that Shot Match puts on the node that was selected, "applied
invisibly, as the very last adjustment to the node … similar to a LUT" (p. 3048): no palette
shows it, but it is in the model, undone, copied, pasted and reset with the node, and evaluated
after Soft Clip (and so after the LUT), inside the node's key mix. The engine always writes it
(`null` for none) and reads a corrector without it as `null`. Its shape, for a solver to fill:

| field | neutral | range |
|---|---|---|
| `gamma` `{r,g,b}` | 1 | 0.1…10, an exponent |
| `gain` `{r,g,b}` | 1 | 0…16 |
| `offset` `{r,g,b}` | 0 | −1…1, white being 1 |
| `contrast` | 1 | 0…4 |
| `pivot` | 0.435 | 0…1 |

Each channel `x` goes to `S(gain · x^gamma + offset)` (the power keeps the sign of `x`), with `S`
the S-curve of `contrast` `k` about `pivot` `p`: `p (v/p)^k` below `p`,
`1 − (1 − p)((1 − v)/(1 − p))^k` above, black, white and beyond kept. `S` is the S-curve
whatever the project's "Use S-curve for contrast" says. At its neutral values it is the
identity, to the bit. It is set with `set_node`: given whole where the node has none
(`{"shot_match": {...}}`, all five fields; a part of one onto `null` is `params`), patched in
part where it has one, and taken away with `{"shot_match": null}`. Out of range is
`out_of_range` with the path, such as `shot_match.gain.r`.

**`blur`** is the Blur palette (pp. 3454–3456): one set of settings that its three tabs show,
Blur, Sharpen and Mist (`mode`), each using a part of it; what one tab sets is kept, unused, in
another (the manual does not say; going from tab to tab and back so changes nothing). As it is
first, and as an old file without it reads:

```json
{"mode": "blur",
 "radius": {"r": 0.5, "g": 0.5, "b": 0.5}, "ratio": {"r": 0.5, "g": 0.5, "b": 0.5},
 "scaling": {"r": 0.25, "g": 0.25, "b": 0.25},
 "coring_softness": 0, "level": 0, "mix": 100,
 "linked": {"radius": true, "ratio": true, "scaling": true}}
```

| field | default | range | used by | what it is |
|---|---|---|---|---|
| `mode` | `blur` | `blur`, `sharpen`, `mist` | | the tab shown |
| `radius` `{r,g,b}` | 0.5 | 0…1 | all | 0.5 nothing; above, a Gaussian blur of `25 · t²` pixels (of a frame 1080 high), `t = (r − 0.5)/0.5`; below, an unsharp mask of a Gaussian of 1 pixel, `x + 3u · s/0.25 · w · D`, `u = (0.5 − r)/0.5`, the detail `D` being `x − blur` of both rounded to half floats (as the neighbourhood's texture keeps them), less 3 steps of half floats either way, so that a flat field or a slow gradient is not sharpened at all (no bands) |
| `ratio` `{r,g,b}` | 0.5 | 0…1 | all | H/V Ratio: the filter `2h` of itself across and `2(1 − h)` down |
| `scaling` `{r,g,b}` | 0.25 | 0…1 | `sharpen`, `mist` | `s` above, for a Radius below 0.5 (the Blur tab takes 0.25) |
| `level` | 0 | 0…1 | `sharpen` | `w` is 1 where the detail `max(abs(D))` of the channels sharpened is at least `0.2 · level`, and below it 0; at 0, 1 everywhere ("sharpen the entire image", p. 3455) |
| `coring_softness` | 0 | 0…1 | `sharpen` | `w` rises smoothly to 1 from `0.2 · (level − coring_softness)` (0 at least) up to the Level: it blends the border below the threshold, and does nothing at Level 0 |
| `mix` | 100 | 0…100 | `mist` | each channel mixed with a Gaussian of 12 pixels of the picture, `m + (y − m) · mix/100` |
| `linked.radius`, `.ratio`, `.scaling` | `true` | | | whether the three columns are ganged (p. 3454); it changes no picture |

The palette works on what the rest of the node made (after Soft Clip and Shot Match) and before
its key, so a qualifier, windows, the key coming in and Key Output Gain limit it as any setting of
the node (p. 3453), and a window draws nothing from outside it into its edge. The constants are
the engine's (the manual publishes none), kept in `compose/src/grade/blur.rs` to be set against
DaVinci. A node reads the pixels about each one (neighbourhoods, as Midtone Detail does) only
where a Radius is ever off 0.5 or, in `mist`, Mix ever below 100: one for each different
Radius and H/V Ratio of its channels (one when ganged), and one more for each H/V Ratio of Mist.
At rest in any tab, with any Scaling, Level, Coring Softness, gang or Mix that does nothing, it
is the node of before, to the bit, and makes no other shader; keyframes across 0.5 make none from
frame to frame. Set by `set_node` as any setting (`{"blur": {"radius": {"r": 0.3}}}`); out of
range is `out_of_range` with the path, such as `blur.radius.r`, and a `mode` not named, or a field
not of it, is `params`.

Every number setting above is a **parameter**: a number, or keyframes
`{"keys": [{"frame": 0, "value": 1.0, "interp": "linear"}, ...]}`. Frames rise strictly, are
within ±(2^53−1), and there is at least one key; every value is within the range. What a frame
counts (below, **frames of keys**) depends on the grade: of a clip of media, a frame of its
media; of a title, a colour or a still, a frame of the clip; of a timeline, a frame of the
timeline. Before the first key the value is the
first's, after the last the last's. `interp` says how the value goes to the next key: `"linear"`,
`"hold"` (a static keyframe, p. 3384) or `{"bezier": {"out": [t, dv], "in": [t, dv]}}`, a cubic
curve whose handles are each a share `t` (0…1) of the frames between the keys and a change of
value `dv` (any finite number): `out` from this key forward, `in` from the next key back
(`[0.33, 0]` for both eases out and in, p. 3385). `Param::value_at(frame)` evaluates it.

**Frames of keys.** DaVinci ties the keys of the Color page to the source of a clip: a grade is
linked to "a default one on the first frame of the master clip" (p. 3383), and a copied grade's
keys are "placed at matching frames that correspond to the source timecode" (p. 3393); the
Keyframe Editor's ruler only "mirrors the record timecode" of the clip (p. 3382). So
(`crates/model/src/keyframes.rs`):

- A key of a **clip's grade**, or of its group's pre-clip and post-clip grades as they apply to
  that clip, is at a frame of the clip's **media**, numbered at the media's own rate from its
  first frame (0), the number of the picture: key `n` is the setting at the moment that picture
  `n` is shown. The picture shown at frame `f` of the timeline is the one read there:
  `k = f − start` (backward, `k = length − 1 − k`); `tick = in.ticks + floor(k · speed.num ·
  rate.den · scale / (speed.den · rate.num))` with `rate` the timeline's (`span::source_tick`);
  then `n = ceil((tick + 1) · m.num / (scale · m.den)) − 1`, the tick clamped to
  `0 … duration.ticks − 1`, `m` the media's rate and `scale` that of its duration
  (`span::media_frame`). Frames are of the media, not the timeline, so a key names a picture
  whatever the timeline's rate; a source of frames at changing times is numbered on the grid of
  its rate, as its picture is read.
- So an edit that changes which pictures a clip shows leaves every key on its picture: a trim of
  either end, `slip`, `slide` and `roll` keep the keys of the pictures still shown where they
  were (pictures newly shown before the first key or after the last hold it); `blade` gives both
  halves the grade, each with the keys of its pictures, and the two play as the clip did (a
  through edit runs "with continuous timecode", p. 804); a change of **speed** or **direction**
  keeps each key on its picture, which is shown sooner or later: at 2× the keys come twice as
  close on the timeline, played backward they come last first, and a picture shown on several
  frames (slower than the media) has one setting on them all. The speed dialog's Maintain
  Timing / Stretch Keyframes is for the keys of the Edit page's Composite, Transform and
  Cropping only (p. 1073).
- A **title**, a **colour** and a **still** (one picture held) have no frames of their own: the
  keys of their grades count the frames of the clip from its first (0), and move with its head.
- The keys of a **timeline's grade** count the frames of the timeline; they are apart from those
  of clips (p. 3394).

**Keys copied to another clip** follow the Gallery's "Apply Grades Using" setting (pp. 3264,
3287), the edit's `keyframes`:

- `"source_timecode"` (Keyframes Aligning Source Timecode; the default where it is left out, as
  DaVinci places copied keys by source timecode, p. 3393): each key goes to the frame of the
  other clip's media at the same time from the start of the media (the frame of that rate that
  the key's time is in), so a grade copied to another clip of the same media keeps its keys on
  the same pictures. "If there is no source timecode overlap" (the stretches of media that the
  two clips show do not meet in time, or either clip has no media), the keys are aligned by
  start frames, as below.
- `"start_frame"` (Keyframes Aligning Start Frames): the frame at the start of the copied
  grade's clip goes to the frame at the start of the other clip, and every key keeps its time
  from it. A clip's start is the first picture of its media it shows, at its in point (shown
  last where it plays backward), or frame 0 of a title, a colour, a still or a timeline.
- Keys that come to one frame (from a higher rate to a lower) are one key there, the last. The
  grades of a group are of no one clip: copied to or from one, keys stay at their frames.
  `paste_node` and `paste_value` copy settings with their keys as they are. DaVinci's third
  choice, No Keyframes, takes the settings at the frame a still was saved at, and comes with
  stills.

### Operations

| op | fields | inverse |
|---|---|---|
| `add_node` | `grade`, `node` (empty `id` for the engine to make one), `index?`, `links?` | `remove_node` with `links` |
| `remove_node` | `grade`, `id`, `links?` | `add_node` with its links |
| `connect` | `grade`, `link` | `connect` of the link the input had, or `disconnect` |
| `disconnect` | `grade`, `to`, `kind`, `input` | `connect` |
| `set_node` | `grade`, `id`, `label?`, `enabled?`, `locked?`, `pos?` (`{x, y}` or `null`), `params?` | `set_node` with the values as they were |
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
  as the engine writes them (`60` comes back as `60.0`). `params` of a node that is locked
  before and after the change are `node_locked`: a node is unlocked and set in one `set_node`
  (`"locked": false` with `params`), or set and locked, so the inverse of a change is never
  refused for the lock. `pos` left out leaves where the node lies; `null` gives it back to the
  editor's layout.
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
| `add_serial` | `grade`, `node` (a node with an RGB output, or `source`), `windows?` (`[window, ...]`, default none) | Add a corrector after it, taking what its RGB output fed (Add Serial, Option-S, p. 3318). With `windows`, the corrector has those Power Windows (see Power Windows) from the start, checked as `set_node` checks `params.windows`, in the one `add_node` that adds it: Add Serial Node + CPW, LPW, PPW and PCW (Option-C, -Q, -G and -B, p. 3187) are one edit and one step to undo. An invalid window refuses the edit as `set_node` would. |
| `add_serial_before` | `grade`, `node` (a corrector, or `output`) | Add a corrector before it, taking what came into it (Add Before Current, Shift-S). |
| `append_node` | `grade` | `add_serial_before` the output (Append Node, Option-K): the first node of an ungraded clip. |
| `add_parallel` | `grade`, `node` (a corrector) | Add a corrector taking what the node takes; if the node's picture goes only into a parallel mixer, the mixer gets one more input for it, else a new parallel mixer takes the node at input 0, the new corrector at input 1, and the node's place (Option-P, pp. 3334–3335). |
| `add_layer` | `grade`, `node` (a corrector) | As `add_parallel`, with a layer mixer; the new corrector is at the lowest input, on top (Option-L, p. 3336). |
| `add_outside` | `grade`, `node` (a corrector) | Add an Outside node after it (Add Outside Node, Option-O, pp. 3318, 3341–3342): a corrector with `key.input.invert: true`, taking what the node's RGB output fed, with the node's picture linked into its RGB input and the node's key into its key input, so that it works where the node does not. Done as one `add_node`. |
| `extract_node` | `grade`, `node` | Take the node out of the tree, bridged as by `remove_node`, and keep it unlinked (E, p. 3319). |
| `reset_node` | `grade`, `node` | Set the node as a new one of its kind, keeping its label, `enabled`, `locked`, `pos` and inputs (`node_locked` where it is locked). |
| `copy_grade` | `from`, `to: [target, ...]`, `preserve_nodes?` (0…10, default 0), `keyframes?` (`"source_timecode"`, the default, or `"start_frame"`) | `set_grade` each `to` to the grade `from`, ids kept (pp. 3286, 3290), its keys put on the frames of each `to` as `keyframes` says (above; kept nodes keep theirs). With `preserve_nodes` N (Copy Grade: Preserve number of nodes, p. 3285), a `to` of at least N nodes keeps its first N: they take the places of the first N nodes of the copy, whose links are kept, so "the first N nodes of a copied grade are ignored"; a node of the copy after them whose id one of them holds gets a new id. A `to` of fewer than N nodes gets the copy whole. Each kept node must be of the kind of the node of the copy it replaces, with as many inputs (`wrong_node` naming the kept node), and the copy must have N nodes (`out_of_range` `preserve_nodes`, as is N over 10). |
| `append_node_graph` | `from`, `to: [target, ...]`, `keyframes?` (as `copy_grade`) | Add the nodes of `from` after the end of each `to`, with new ids, their keys put on the frames of each `to` as `keyframes` says: what fed the output feeds what the source fed, and what fed the output of `from` feeds the output (Append Node Graph, p. 3290). `append_grade`, its old name, is read as it. |
| `collapse_group_grades` | `timeline`, `clips: [clip id, ...]` | For each clip, in a group (`ungrouped` if not): put the nodes of the group's pre-clip grade before the clip's nodes and those of its post-clip grade after them, chained as `append_node_graph` chains (the picture goes through them in the order of p. 3330, keyframes at their frames), with new ids for the group's nodes, the clip's keeping theirs; then `set_clip_group` it to `null`. The group keeps its grades and other clips (Collapse Group Grades, pp. 3306–3307). A group grade without nodes adds none; one that is not complete is copied as it is, leaving the clip's grade not complete. The timeline grade is not touched. Done as `set_grade` (when there are nodes to add) and `set_clip_group`. |
| `paste_value` | `from`, `from_node`, `to`, `to_node`, `param` | Set one field of `params`, by its path with dots (`primaries.saturation`, `primaries.lift`), to its value in the other node (Paste Value, p. 3286). A path that either node lacks is `out_of_range` `param`. |
| `paste_node` | `from`, `from_node`, `to`, `to_node` | Set the `params` of `to_node` to those of `from_node`, of the same kind (p. 3320). |

A wrong node for an edit is `wrong_node`; an empty `to` is `out_of_range` `to`, empty `clips`
`out_of_range` `clips`.

### Refusals of grades

| reason | fields | meaning |
|---|---|---|
| `no_port` | `node`, `kind`, `input` (`null` for an output) | a link from an output, or into an input, that the node or end does not have |
| `input_taken` | `node`, `kind`, `input` | two links into one input |
| `loop` | `node` | the links would go round in a loop through `node` |
| `grouped` | `group`, `clip` | `remove_group` of a group that clip `clip` is in |
| `ungrouped` | `clip` | `collapse_group_grades` of a clip that is in no group |
| `params` | `node`, `why` | `set_node` params that are not those of the node's kind |
| `wrong_node` | `node`, `kind` (the node's kind, `source` or `output`) | an edit that cannot be done to or about that node |
| `node_locked` | `node` | the `params` of a locked node set, by `set_node` or an edit that sets them (Lock Node, p. 3382). Not `locked`, which is the refusal of a locked track (`track`) |

`missing` also names `group`, `node` and `link`; `taken` `node` and `group`; `out_of_range` also
`label`, `inputs`, `links`, `lut`, `param`, `to`, `clips`, `preserve_nodes`, `pos` and the paths
of corrector settings.

**The default node 01.** A clip new to DaVinci's Color page has one ungraded corrector, 01
(pp. 3309, 3312); a new grade of the engine has no node, which colours the picture as that one
does. An editor may show a grade without nodes, whose source feeds its output, with a node 01 of
the id `n` + `next_id`, and make it with the first change to it, in the transaction of the
change: `add_serial` after `source` first (in a grade without nodes the engine gives exactly that
id, as it passes over only the ids that the grade holds), then the change naming that id. Undone,
the grade has no node again. A change that leaves the node as it is (taking it away, resetting
it) sends nothing; a whole grade put in place (Reset All Grades and Nodes, Ctrl+Home, which
gives back the grade without nodes) is sent as it is.

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
- **Frames of keys**: a layer of a clip of media is graded at the frame of its media whose
  picture it shows (Frames of keys, above), worked out from the same place in the media that
  the picture is read at, so the picture and its grade are always of one frame, in the monitor
  and in an export alike, at any trim, slip, speed or direction, and within a transition, where
  a clip is seen beyond its ends. A title, a colour or a still is graded at the frame of the
  clip from its first. The timeline's grade is at the frame of the timeline.
- **Cost**: a grade is made into code once, when it changes (any of its settings, or the card);
  each frame takes only its parameters at that frame, so playing keyframes or dragging a setting
  compiles nothing, and a change of grades alone does not start the read-ahead again.
- **A new shape of a grade** (a node added, removed, enabled or linked otherwise; a LUT of
  another shape) is compiled on threads of the engine's own, a tenth of a second to a few
  seconds, while the timeline monitor goes on: until it is made, a layer is drawn with its
  grade as the frame before drew it, or ungraded where the layer at its place before was other
  media, and the timeline grade (and the output transform) as before. The monitor draws the
  frame again once it is made, playing or still. `transport.frame_digest` and
  `scopes.statistics` of the timeline monitor measure the frame drawn with the grade as it is
  (they wait for it); an export always waits for its grades.
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
  (its clips' on tracks of picture, their groups', its own), and of the project's Input, Output
  and Color Viewer LUTs (What colour management does to the picture), in the **timeline state**
  (left out where there is none, so a state without it has none missing; told again as soon as
  a change of the project's colour, made on a thread of its own, finds or loses one) and in each
  `render.progress` of an export of that timeline (left out likewise). The view of the timeline
  monitor also shows it, as "an overlay at the bottom right of the screen indicating the name of
  the LUT if a single LUT is missing, or an indicator that multiple LUTs are missing": the
  engine writes `Missing LUT: <name>` or `Multiple LUTs missing` (DaVinci's words, in English)
  into the picture that it hands the view, after the Color Viewer LUT, white letters outlined in
  black on a black plate, 2.8% of the frame high, 2.5% of its height from its lower right corner
  (a white of 0.9 in SDR; of PQ 0.58 and of HLG 0.75, about 203 nits); the page cannot draw over
  the native view. The scopes, `view.sample`, a still grabbed and an export see the picture
  without it. Words that do not fit on the frame are not written.
- **Highlight**: the timeline state has `highlight: {"mode", "grade", "node"}` while a Highlight
  mode is on (`viewer.highlight`, in Comparisons in the viewer), and none while it is off.
- **Bypass Color**: the timeline state has `bypass: true` while the monitor shows its picture
  without its grades (`viewer.bypass`, in Comparisons in the viewer), and none while it shows them.
- **Live Preview**: the timeline state has `preview: {"grade", "node", "lut", "frame"?}` while a
  LUT is previewed on a node (`viewer.preview`), and none while none is; `viewer_lut: false`
  while the view leaves the Color Viewer LUT out (`viewer.lut`), and none while it shows it
  (see Live Preview and the Color Viewer LUT).
- **Colour being made**: the timeline state has `color_pending: true` while a change of the
  project's colour management (its settings, or the colour of media) is being made on a thread
  of the engine's own, the monitor showing the colour it had until then, and none once the new
  one is shown (told at once both ways; up to a second and a half for an ODT of ACES 2.0; see
  What colour management does to the picture). `color_error: "<OCIO's message>"` where the
  colour management that the project asks for could not be made: the monitor goes on showing
  the colour it had, and the change is not tried again until the project's colour changes
  again; none where it was made.

### Generate LUT

`grade.export_lut` bakes a grade into a 3D LUT and writes it as a `.cube`, as Generate LUT of
DaVinci Resolve does from a clip's thumbnail (Generate 3D LUT (17/33/65 Point Cube), pp.
3413–3414). It answers once the file is written, and changes neither the project nor its
history; a headless engine has it too.

```json
{"grade":{"level":"clip","timeline":"t1","clip":"c3"},"size":33,"path":"looks/day.cube"}
```

- **`grade`** is any grade (`clip`, `pre_clip`, `post_clip`, `timeline`); **`node`**, where
  given, a corrector of it baked alone (between the source and the output, whether or not it is
  on). **`size`** is the points on an edge: 17, 33 or 65. **`frame`** is where its keys are
  read: a frame of the timeline for a clip's grade (turned into the frame of its keys as the
  monitor does; the clip's first frame where left out) and for a timeline's (0), the frame of
  its own keys for a group's (0).
- **What is baked** (p. 3413: Primaries and Custom Curves adjustments "mathematically combined",
  nodes with Qualifiers, Windows or filtering "completely ignored"): what the grade does to each
  colour on its own, by the engine's reference of the grades on the processor: the Primaries
  in both modes (wheels and Log), Contrast and Pivot as the project's S-curve setting says,
  Temperature, Tint, Color Boost, Shadows and Highlights, the custom and HSL curves, Soft Clip,
  Shot Match, and the LUT of a node (between its points as the project's
  `lut_interpolation` says), through each node's key settings and the mixers as the grade links
  them, with the luma of the timeline space. A node that reads the pixels about each one
  (Midtone Detail other than 0) is left out whole, as though it were off, and named in
  `ignored` (`{"node": "n2", "why": "midtone_detail"}`); asked for alone, it is refused.
  A node with a qualifier is left out so too (`"why": "qualifier"`), and one whose Blur palette
  changes the picture at some frame (`"why": "blur"`: a Radius ever off 0.50, or Mist with a Mix
  ever below 100), named after `qualifier`, `windows` and `midtone_detail` where several apply;
  a Blur palette at rest in any mode is baked as the rest of its node. The project's Input, Output
  and Viewer LUTs and the transforms of colour management are not of the grade, and are not
  baked.
- **The cube**: a `TITLE`, `LUT_3D_SIZE` and `size`³ rows, red fastest (Adobe's form, as
  `crates/lut` writes it, every value the shortest decimal that reads back as the same float);
  over 0–1 of the timeline space, its values as the grade gives them (beyond 0–1 too). A point
  of it is the grade's at that colour, to the float; between the points it is as near as a cube
  of its size is (the test: within 2e-2, 8e-3 and 4e-3 of a contrast, lift and saturation for
  17, 33 and 65).
- **`path`**: a name relative to the folder of the project, of plain names (no `..`, root,
  drive or `:`; through no link or junction; not in `cache`), its folders there already: the
  folder of LUTs of the project, so that `luts.list` lists it and a corrector's `lut` may name it
  by that name. Or an absolute path, which the user chose (a dialog of the desktop), as
  `render.start` takes one: its folder there already and not in the project's cache, and no
  file of the media of the project or its proxy. Either ends in `.cube` (any case). It is
  written beside its place and then moved there, so that nothing reads half of it; a file there
  is replaced only with `replace: true`, and only a file.
- **Answer**: `path` (the file, absolute), `name` (what a corrector's `lut` names it by: relative
  to the root of LUTs it lies in, with `/`, else the path), `size`, `nodes` (those baked, in the
  order of the grade) and `ignored`.
- **Refused** with `-32602` and `data.reason`: `size` (not 17, 33 or 65), `path` (with
  `message`), `missing` (with `timeline`, `clip`, `group` or `node`: no such grade or node),
  `not_a_corrector` (`node` is a mixer), `cannot_bake` (`node`, `why`). With `-32001`:
  `missing_luts` (`missing_luts: [{"name", "message"}]`: a LUT of a node that is baked cannot be
  read, so the cube would not be the grade), `exists` (`path`, without `replace`), `source`
  (`path`, `media`), `write` (`path`, `message`). `-32002`, where the luma of the timeline space
  cannot be found. Nothing is written when it is refused.

## Scopes

The video scopes of a monitor, as the Color page of DaVinci Resolve has them (pp. 3007–3022):
the waveform, the RGB parade, the vectorscope, the histogram and the CIE 1931 chromaticity
diagram. A client subscribes a scope to a monitor and is sent a picture of it for each frame
that the monitor shows; the agent reads the figures of the frame shown instead
(`scopes.statistics`). They are of an engine that is not `--headless` (one with monitors): in a
headless engine every `scopes.*` method is `-32601`, but `scopes.statistics` of a frame drawn off
screen (below).

**What is measured.** The frame as the monitor shows it, before the view maps it to its
display. For the timeline monitor, that is the frame composed with its grades: the signal that
an export writes, with the subtitles that the monitor draws over it. For the source monitor it
is the picture of the media alone, ungraded, composed as a timeline would show it: DaVinci's
scopes show the image of the viewer, on every page that has them (Media, Cut, Edit, Color and
Deliver, p. 692). R', G' and B' are signals from 0 to 1, 1 being the code 1023 of 10 bits
whatever the range of the file (DaVinci's data levels, p. 3008). A scope counts every pixel of
a frame of up to 4096 × 2160, one in two each way up to 8K, and one in four beyond.

**`scopes.subscribe`** `{"view", "settings"}` answers `{"scope_id", "size": {"width",
"height"}, "graticule", "settings"}`: the id of the scope (`"scope-<n>"`, of this connection
alone), the size of its pictures, its graticule, and the settings as they were taken, with every
field written. A connection may have 16 scopes at once (DaVinci's floating window shows 9).

**`scopes.set`** `{"scope_id", "settings"}` gives a scope of this connection other settings,
and answers as `scopes.subscribe`. Every frame sent after the answer is of the new settings.

**`scopes.unsubscribe`** `{"scope_id"}` answers `{}`, and no frame of the scope is sent after
the answer. The scopes of a connection are let go when it closes.

**Frames.** A scope is sent `scopes.frame` once it is subscribed or set, if its monitor has
something loaded (the monitor draws its frame again for it), and each time its monitor shows a
frame: playing, seeking or paused, and when a change of the grades changes what is seen. A
scope is sent at most **30 frames a second**. While its monitor shows frames faster, those in
between are passed over, and the last frame shown is always sent once its time comes, so a
scope rests on the frame that the monitor rests on. A scope is not sent another frame while its
last one has not been written to the client, so a client that reads slowly is sent fewer
frames, and is not closed for them. A paused monitor that shows nothing new sends nothing.

`scopes.frame` `{"scope_id", "view", "frame", "width", "height", "rgba"}` is sent to the
connection that subscribed the scope, and to no other. The protocol has one framing, lines of
JSON, and the picture is carried in it as thumbnails and waveforms are: `rgba` is the base64 of
`width × height × 4` bytes, RGBA of 8 bits, by rows from the top, opaque (the default waveform,
512 × 256, is 512 KiB, some 700 KB as base64). `frame` is the frame of the timeline, or of the
media for the source monitor. A scope is counted on the graphics card from the very texture
that the monitor shows, in the submission to the card's queue that follows the frame's own (the
compositor submits each frame by itself), and read back on a thread of the scopes, so that
neither monitor waits for it. When the card is lost, the scopes are made again on the next one;
a frame that cannot be counted or read is tried again a second later, or passed over for the
next frame shown.

**Settings.** `kind` and the kind's own fields side by side, then `signal` and `look`. Every
field but `kind` may be left out, and is then as this table says; a field that is not one of the
kind's is refused.

| field | kinds | default | values |
|---|---|---|---|
| `kind` | | (required) | `"waveform"`, `"parade"`, `"vectorscope"`, `"histogram"`, `"chromaticity"` |
| `width` | waveform | 512 | columns, 16–4096 |
| `width` | parade | 256 | columns of each of R, G and B, 16–1365; the picture is three times as wide |
| `width`, `height` | chromaticity | 512, 576 | 16–1024 each; x is 0–0.8 across and y 0–0.9 up, so 8 to 9 keeps their units alike |
| `levels` | waveform, parade | 256 | the height of the picture: levels of a signal from 0 to 1, 16–1024 |
| `channels` | waveform | `"luma"` | `"luma"`, or `"rgb"`: R, G and B over one another, white where they meet |
| `channels` | histogram | `"rgb"` | `"rgb"`, or `"yrgb"`: luma above them, each in a strip of its own |
| `scale` | waveform, parade, histogram | `"10-bit"` | `"10-bit"` (0–1023), `"percent"`, `"nits"`: on the curve of PQ, or of HLG on a display of 1000 nits where `signal.transfer` is `"hlg"` (p. 3009) |
| `size` | vectorscope | 256 | its side, even, 16–1024 |
| `zoom` | vectorscope | 1 | 1–8 (2 is DaVinci's 2X Zoom) |
| `tones` | vectorscope | `"all"` | `"all"`, `"low"` (luma below `low`), `"mid"`, `"high"` (luma above `high`) (p. 3020) |
| `low`, `high` | vectorscope | 0.30, 0.70 | 0–1, `low` not above `high` |
| `targets` | vectorscope | `"75"` | the boxes of colour bars: `"off"`, `"75"`, `"100"`, `"both"` (p. 3010) |
| `skin_tone` | vectorscope | `true` | whether the skin tone line is drawn, 123° from Cb |
| `bins` | histogram | 256 | 16–1024, the width of the picture |
| `height` | histogram | 256 | 16–4096, shared by its strips |
| `space` | chromaticity | `"rec709"` | the primaries of the signal: a gamut (`"rec709"`, `"rec2020"`, `"dci-p3"`, `"p3-d65"`, `"rec601-pal"`, `"rec601-ntsc"`), or `{"red": [x, y], "green", "blue", "white"}` |
| `gamuts` | chromaticity | `[{"name": "Rec.709", "primaries": "rec709"}]` | the triangles drawn, each `{"name", "primaries"}` with primaries written as `space` is; the first is the space's own as a rule, and the next one is to compare with (p. 3017) |
| `signal.project` | all | `true` where `signal` is left out | measure by the signal of the output space of the project's colour management, followed as it changes (the `luma` and `transfer` written are then Rec.709's, and not used); `false` to measure by `luma` and `transfer` |
| `signal.luma` | all | `"rec709"` | the luma coefficients: `"rec709"`, `"rec601"`, `"rec2020"`, or `[red, green, blue]`, red and blue above 0 and at most 0.5 |
| `signal.transfer` | all | `"rec709"` | a gamma of the model (`"rec709"`, `"gamma22"`, `"gamma26"`, `"gamma28"`, `"srgb"`, `"linear"`, `"st2084"`, `"hlg"`): what the chromaticity diagram makes the signal linear by, and the curve of a scale of nits |
| `look.mapping` | all | `"log"` | how a count is made a brightness: `"log"`, or `"gamma"` (2.2), which keeps more of the strong parts apart |
| `look.brightness` | all | 0.5 | 0–1; each 1/16 doubles the trace (DaVinci's slider of each scope) |
| `look.graticule` | all | 0.5 | 0–1: how brightly the graticule is drawn into the picture; 0 for none, the client then drawing `graticule` itself |

A scope whose settings leave `signal` out (or give `"project": true`) measures the timeline
monitor's frame by the luma coefficients and transfer of the project's output space (Rec.709 in
`DaVinci YRGB` and for a space of a camera; BT.2020 and PQ for `Rec.2100 ST2084`; of a bypassed
output, the timeline's), and is sent a frame by them as soon as a change of the colour
management changes them. The source monitor's are Rec.709. `scopes.statistics` measures luma
the same way; of a frame drawn off screen (`{timeline, frame}`), which is drawn through the same
input and output transforms as the monitor shows it, by its output space; of the picture at a
node, which is before the output transform, by the primaries of the timeline space.

**The graticule** is `{"width", "height", "lines": [{"from": [x, y], "to": [x, y], "colour":
[r, g, b]}], "labels": [{"at": [x, y], "text": "512", "colour": [r, g, b]}]}`, in pixels of the
picture from its top left corner; a label is written from `at`, its left at the middle of its
height. It holds the scale, the targets and the skin tone line, the spectral locus and the
triangles. It is given once for each settings, in the answer. The same lines are drawn into the
picture as brightly as `look.graticule` says, so that a picture reads alone (as when the agent
is shown it).

**`scopes.statistics`** `{"view", "bins"?}` (`bins` 16–1024, 256 unless given) measures every
pixel of the frame that the monitor shows now, on the card, and answers at once; it needs no
scope:

```
{"view": "timeline", "frame": 120, "width": 1920, "height": 1080, "samples": 2073600,
 "red": {"min", "max", "mean"}, "green": {...}, "blue": {...}, "luma": {...},
 "chroma": {"centroid": [cb, cr], "spread": [σcb, σcr], "radius", "saturation", "hue"},
 "percentiles": {"red": {"1", "50", "99"}, "green": {...}, "blue": {...}, "luma": {...}},
 "histogram": {"bins": 256, "red": [n, ...], "green": [...], "blue": [...], "luma": [...]}}
```

- Signals are from 0 to 1, or beyond where the frame is: `min` and `max` are its true extremes.
- Luma and chroma are made with Rec.709's coefficients until the colour settings of the project
  say otherwise.
- `chroma` is in Cb and Cr, from −0.5 to 0.5 within the gamut, as the vectorscope shows it
  unzoomed. `centroid` is their means (a cast where it is off the centre), `spread` their
  standard deviations, `radius` the root mean square distance from the centroid, `saturation`
  the mean distance from the centre, and `hue` the angle of the centroid in degrees
  counterclockwise from Cb (0–360; the skin tone line is at 123).
- A percentile is the signal of the bin below which that share of the pixels is.
- The same frame gives the same figures, to the last bit.

**Off screen, and at a node.** `scopes.statistics` `{"timeline": "t1", "frame": n, "bins"?}`
measures that frame of that timeline drawn off screen, as an export draws it (graded, without
subtitles; see Samples and the pointer, in The view): in a headless engine too, as an agent that
runs its own engine reads the figures of a shot. With `"grade": target, "node": "n1", "at"?:
"input" | "output" | "key"` (and `"clip"?` for a grade of a group), with `{"view": "timeline"}` (the
timeline and frame that the timeline monitor shows) or with `timeline` and `frame`, it measures
the picture that goes into the node or comes out of it, drawn as The picture at a node says (the
Auto Color of a node, and the histogram of the Curves palette, p. 3110). The answer is as above,
with `"timeline"` beside `frame`, `"node": {"grade", "node", "at"}` for a node, and `view` only
where a monitor was named. Refused with `-32602` for both or neither of `view` and `timeline`
with `frame`, the source monitor with a node (`reason: "node"`), `at` or `clip` without a node,
and the refusals of The picture at a node; with `-32004` `not_loaded` where the timeline monitor
shows no timeline.
**Refusals.** `-32602` for params that are not the method's; with `data` `{"reason":
"settings", "message"}` for settings that cannot be read or are out of range, `{"reason":
"missing", "scope_id"}` for a scope that is not this connection's, `{"reason": "too_many",
"most": 16}`, and `{"reason": "bins"}`. `scopes.statistics` is `-32004` with `{"reason":
"not_loaded"}` when the monitor shows nothing, and with `{"reason": "draw"}` when the frame could
not be measured (the card was lost: ask again).

## Comparisons in the viewer

The timeline monitor can show its frame compared with a reference, as the viewer of the Color
page of `DaVinci Resolve` does (Reference Manual p. 2999–3006, 3260–3261): a wipe of the frame
(A) against one reference (B), or a split screen of up to 16 pictures in a grid. It is of the
monitor alone: an export renders the timeline as it is, whatever is compared. The methods are of
an engine that is not `--headless` (`-32601` there). A wipe and a split screen are one at a time:
setting one turns the other off; turning one off leaves the other as it is. Each method answers
what is compared now, and the monitor draws its frame again at once where it is not playing.

**`viewer.wipe`** `{"on"?, "style"?, "position"?, "angle"?, "softness"?, "border"?,
"border_color"?, "invert"?, "gain"?, "reference"}` sets the whole wipe (what is left out takes its
default, not what was set before); `{"on": false}` turns it off.

| Field | Default | What it is |
|---|---|---|
| `style` | `"horizontal"` | `"horizontal"` (an upright border, A on the left), `"vertical"` (a level border, A above), `"diagonal"` (a border at `angle`), `"mix"`, `"difference"`, `"box"` (A in a box about the centre, B about it), `"venetian_blind"`, `"checkerboard"` |
| `position` | 0.5 | 0–1: where the border is across the canvas (1: all A, 0: all B, its softness included); the size of the box (1: all A); the share of B of a mix; the size of the strips (a pixel at 0, half the canvas at 1) and the width of the checks (a check is an eighth of the canvas high) |
| `angle` | 45 | −360–360: of `"diagonal"`, the degrees of its border from upright, turned clockwise |
| `softness` | 0 | 0–1: the width of a straight ramp across the border, as a share of the shorter side of the canvas (lines and box) |
| `border` | 0 | 0–64: the width of a line drawn on the border, in pixels (lines and box) |
| `border_color` | `[1, 1, 1]` | the R'G'B' of that line, each 0–1 |
| `invert` | false | the sides swapped (Invert Wipe): B where A was |
| `gain` | 1 | 0–100: of `"difference"`, which shows `\|A − B\| · gain` (black where they are alike), opaque |

A wipe at 0 or 1, and a mix at 0 or 1, show A or B exactly. The Alpha style is not done (it needs
the alpha output of the node graph). Softness, the border line and the gain are not in the manual
for the viewer and are off unless set.

**References** (`reference` of a wipe, each of `items` of a split screen):

- `{"kind": "still", "path"}`: a picture file (PNG, TIFF, OpenEXR; the first picture of the
  file), read when the method is called, shown as it is stored (a still holds its grade),
  fitted to the canvas. It is not graded again. `{"kind": "still", "still": "s3"}` names a still
  of the Gallery by its id instead (see The Gallery): its picture is read the same way, and the
  answer gives the reference with the `path` the engine found; a still not in the Gallery (or an
  id of no still, or both `still` and `path`) is `-32602` `{"reason": "still", "still"}`.
- `{"kind": "frame", "frame"}`: the timeline at that frame (held within it), graded as it is
  there. It stays where it is as the playhead moves.
- `{"kind": "clip", "clip", "frame"?: 0}`: the timeline at `frame` frames from the first frame of
  `clip` (held within the clip), as Wipe Timeline Clip shows another clip.
- `{"kind": "neighbour", "side": "previous" | "next", "steps"?: 1, "current"?: "c2"}`: the clip
  before or after the current one (`steps` 1–7 clips away, 1 where left out: 2 is the clip two
  before or after, as the Neighbor Clips split screen shows "the previous two clips", p. 3004), at
  the same offset from its first frame as the playhead is from the current clip's (held within
  it); it follows the playhead, as a timeline wipe ganged with the current clip does. The current
  clip is `current` where it is given (the clip that the client shows as current), else the one on
  top at the playhead; clips are in the order of their start, then of their track from V1 up.
  `steps` is written back only where it is not 1.
- `{"kind": "current"}` (a split screen only): the frame of the playhead.
- `{"kind": "black"}` (a split screen only): a black cell, as Neighbor Clips leaves one where
  there is no clip (before the first, after the last), since a neighbour that is not there is
  refused when it is set.

A reference of the timeline is drawn as the frame is (its media read, not proxies; each clip and
the timeline graded at that frame; no subtitles). One that is not there where the playhead has
moved (the clip before the first) is black. What was drawn is kept until the project changes, or
the frame that it is, so a wipe while playing costs a pass over the frame (under 0.4 ms at 4K on
an RTX 4080 Laptop).

**`viewer.split`** `{"on"?, "items", "selected"?, "outline"?: true}`: 1–16 items, laid in a grid
in reading order, as many columns as the square root of their number rounded up and as many rows
as they need; each fitted into its cell keeping its shape, black about it. The item at
`selected` is outlined in grey where `outline` is true. The modes of `DaVinci Resolve` (Selected
Clips, Selected Still Images, Neighbor Clips, Playheads, Current Group) are lists of items that
the client makes; those of grades, LUTs and versions applied to the current clip are not done.

**What it answers.** `{"mode": "off"}`, `{"mode": "wipe", "wipe": {every field, "reference"}}`
or `{"mode": "split", "split": {"on", "items", "selected"?, "outline"}}`. A reference of the
timeline is given with `"at"`: the frame of the timeline it shows now, or `null` where there is
none. That `at` is of the moment of the answer: as the playhead moves (playing), the **timeline
state** tells `compared`, the frame of the timeline that each reference shows for the frame shown
now, in their order (one of a wipe), `null` for one that is no frame of the timeline (a still,
the frame shown, black, a neighbour beyond the ends), told with the frame as the state is told
(at most 30 times a second while playing), and at once when a wipe or split screen is set; left
out where nothing is compared. A wipe of `"steps": 2` before frame 61 of the blue clip of green,
red and blue of 24 frames each: `"compared": [13]`.

**The scopes** (`scopes.*`) and `transport.frame_digest` measure what the viewer shows: the
comparison, where there is one.

### Highlight

**`viewer.highlight`** `{"mode": "off" | "highlight" | "bw", "grade"?: target, "node"?: "n1",
"monitor"?: "timeline"}` shows the key of a node on the timeline monitor in place of its picture,
as the Highlight modes of the viewer do (pp. 3170–3171): Shift-H is `highlight`, Alt-Shift-H
`bw`, and the page sends the grade and the node chosen in the node editor, again whenever another
is chosen. It answers `{"monitor": "timeline", "mode", "grade"?, "node"?}` (the mode now), and
the monitor draws its frame again at once where it is not playing. It is of the preview alone: an
export, a frame drawn off screen (`view.sample` and `scopes.statistics` of a timeline and frame)
and the references of a wipe are drawn as they are; the comparison of a wipe or a split screen,
and the scopes, are of the frame as it is shown, highlighted. A still grabbed is of the frame as
it is graded, not highlighted (see Grabbing). The timeline state tells the mode (`highlight`, see
Grades in the picture). It is of the monitor, which the pages share: the desktop shows it on the
Color page alone, and turns it off as that page is left. A `--headless` engine answers
`-32601`.

- **`highlight`**: the picture where the node's key is, and a flat grey (0.5) where it is not,
  "the selected portion of the image with the original colors, and the unselected portion of the
  image with a flat gray", mixed by the key; the picture is the frame shown, every grade and
  transform done, so the adjustments are seen as they are made. The grey is put in place in the
  node's grade: the grades after it (a group's post-clip grade, the timeline's) and the output
  transform do to it what they do to the picture, and with none of them, as in a project of
  `DaVinci YRGB` whose timeline is not graded, it is flat. Layers that are not of the grade (a
  clip's other layers, for a node of a clip's grade) are shown as they are. A node with no key of
  its own (a corrector without a qualifier or a key linked into it; a parallel or layer mixer) is
  shown alone instead: "if you turn on Highlight while you're selecting a node with no key, then
  only the currently selected node is output" (p. 3171), the node's picture in place of its
  grade's, the rest as it is.
- **`bw`**: the key itself, a "high-contrast" highlight, white where it is full and black where
  it is none. For a node of a clip's grade, or of its group's grades, only the layers of those
  clips are drawn (opaque), without the grades after it, the timeline's grade, the output
  transform or subtitles; for a node of the timeline's grade, every layer, the timeline's grade
  showing the key, and no output transform or subtitles.
- A node that its grade does not run through (no such node, a grade that is not complete, a grade
  of another timeline, a clip not shown) leaves the picture as it is: the mode stays on, and shows
  the node again once it is there.
- **Refusals**: `-32602` for params that are not the method's, with `data` `{"reason":
  "monitor"}` for the source monitor (it shows no grade) and `{"reason": "node"}` for a mode on
  without both `grade` and `node`.

### Bypass Color

**`viewer.bypass`** `{"on": true | false, "monitor"?: "timeline"}` shows the timeline monitor's
picture without its grades, or with them again: Bypass Color of the viewer's title bar (Bypass
Color and Fusion, p. 2987; View › Bypass Color and Fusion; Shift-D on the Cut and Edit pages, pp.
504, 550, which the desktop takes on the Color page too; this engine has no Fusion). Every
grade is left out (each clip's, its group's pre-clip and post-clip grades, Shot Match with them,
and the timeline's); the input and output transforms of colour management, the project's Input
and Output LUTs and the Color Viewer LUT stay, so the picture is the media as the project
manages its colour. While it is on, a Highlight mode shows the picture as it is. It answers
`{"monitor": "timeline", "on"}` (the setting now); the monitor draws its frame again at once
where it is not playing, and the timeline state tells it (`bypass`, see Grades in the picture).
It is of the preview alone, as Highlight is: an export, a frame drawn off screen, the
references of a wipe and a still grabbed (see Grabbing) are graded. It is of the monitor and is
kept from page to page, as `DaVinci Resolve`'s is on the Cut, Edit and Color pages (pp. 550,
672): the desktop shows it and turns it on each. A `--headless` engine answers `-32601`; `-32602` for params
that are not the method's, with `data` `{"reason": "monitor"}` for the source monitor.

**Refusals.** `-32602` for params that are not the method's, with `data` `{"reason": "range"}`
for a value out of its range (or no items, more than 16, or `selected` not one of them),
`{"reason": "reference", "reference"?}` for a wipe that is on with no reference, with `current`,
or with a reference not found in the timeline (no such clip, or none before the first or after
the last), and `{"reason": "still", "path", "message"}` for a still that cannot be read. A
reference of the timeline needs a timeline loaded: `-32004` with `{"reason": "not_loaded"}`
otherwise. A refused call changes nothing.

### Live Preview and the Color Viewer LUT

**`viewer.preview`** `{"grade": target, "node": "n1", "lut": "Film/Look.cube", "frame"?: n,
"on"?: true, "monitor"?: "timeline"}` shows on the timeline monitor what the LUT `lut` of the
library (named as `luts.list` names it) would do in place of the LUT of the node `node` of the
grade `grade`: Live Preview of the LUT browser, which "will display how the current clip will
appear with the LUT you select applied to the currently selected node of the current grade"
(p. 3411). `{"on": false}` ends it, and so does the end of the connection that turned it on last,
which no client would be left to end otherwise (another connection that turned it on before, or
one that never did, ends nothing as it ends). The frames are drawn from a copy of the project whose node has
that LUT, as `set_node` of `{"lut"}` would make it, and nothing else: the project, its revision
and history, and every other client are as they were, and an export, a frame drawn off screen
(`view.sample` and `scopes.statistics` of a timeline and frame, `luts.preview`,
`timeline.thumbnail`), the references of a wipe and a still grabbed (see Grabbing) are of the
project as it is. What the monitor shows (the comparison of a wipe, the scopes, `view.sample`
and `transport.frame_digest` of the monitor) is the frame as it is previewed. `frame`, where it
is given, is the frame of the timeline shown while the monitor is still (Hover Scrub Preview,
"the duration of the current clip in both the thumbnail and Viewer", p. 3259): `shown` of the
state is it, the playhead (`frame`) stays where it is, and playing shows the playhead's frames.
A node that cannot take the LUT (one that is not there, a node that is not a corrector, a locked
node) leaves the picture as it is, the preview staying on, as a Highlight mode does. It answers
`{"monitor": "timeline", "preview": {...} | null}`, the monitor draws its frame again at once
where it is not playing, and the timeline state tells it (`preview`). Bypass Color, which leaves
every grade out, leaves the previewed LUT out too. Refusals: `-32602` for params that are not
the method's, with `data` `{"reason": "monitor"}` for the source monitor, `{"reason": "lut"}`
for a preview on without its grade, node and LUT, and `{"reason": "name"}` for a LUT not named
relative to the folders of LUTs; a `--headless` engine answers `-32601`.

**`viewer.lut`** `{"on": true | false, "monitor"?: "timeline"}` puts the project's Color Viewer
LUT (`viewer_lut`, What colour management does to the picture) on the view of the timeline
monitor, or leaves it off: "The Color Viewer Lookup Table options … only affect the GUI Viewer in
the Color page. They do not affect the viewers in the Edit page" (p. 672). The engine does not
know which page its view is on: it shows the LUT until it is told not to, and the desktop turns
it off on every page but the Color page and on again there. It answers `{"monitor": "timeline",
"on"}`, the view shows the frame again at once where it is not playing, and the timeline state
says `viewer_lut: false` while it is off. Nothing but the view ever has the Color Viewer LUT.
Refusals as `viewer.bypass`'s.

## The Gallery

Stills of the timeline monitor, with the grades that made them, kept in albums, as the Gallery
of the Color page of `DaVinci Resolve` keeps them (Reference Manual ch. 141–142, pp. 3256–3297,
3393). Two galleries, neither in `project.json` and neither undone by `history.undo` (the
Gallery is apart from the project's history; a still deleted is restored by `still.restore`):

- **The project's**, in `<project>/gallery`: its still albums (ids `a1`, `a2`, …; stills `s1`,
  `s2`, …), "available only to the project in which they were saved" (p. 3266). It starts with
  one album, `Stills 1` (p. 3265).
- **The PowerGrades**, shared by every project of the user (p. 3266): in the folder that
  `COLUMBINA_POWERGRADES` names (absolute), else `%COLUMBINA_HOME%\powergrades`, else
  `%USERPROFILE%\.columbina\powergrades` (`$HOME` elsewhere). Its albums are `pa1`, …, its stills
  `ps1`, …; it starts with `PowerGrade 1`. Every engine of the user reads and writes it, one
  change at a time (a lock folder `.lock` in it, waited for at most 5 s, and taken over after
  30 s as left by an engine that ended).

Each holds `gallery.json` (`{"format": 1, "next": n, "albums": [{"id", "name", "stills": [ids in
order], "memories"?: true}], "recycled"?: [{"id", "album", "at"}]}`), and in `stills/` for each
still `<id>.json` (what it holds, below), its picture `<id>.png` or `<id>.exr`, and
`<id>.thumb.png` (at most 270 high, R'G'B'A of 8 bits); in `recycle/` the files of the stills
deleted that can be restored (see Albums and stills). Ids are never given twice in a gallery,
nor taken back from a still deleted. A still is named by its id only.

`format` 2 adds `recycled` and `"memories": true` to format 1, whose engines read no field they
do not know. An index is written as format 2 only while it holds one of them, else as format 1,
so that the PowerGrades, which every engine of the user shares, stay readable by an engine of
format 1 until a still is deleted from them. An engine reads formats 1 and 2, and refuses any
other (`-32002` `{"reason": "gallery", "path", "message"}`, the message naming the formats it
reads).

### What a still holds

`still.get` gives it whole; `gallery.list` and the other methods give it without `grades`,
`frozen`, `keyframes` and `color`, and with `source` cut to its ids, frames and timecodes, and
`grades` as `{"clip": bool, "timeline": bool}`.

| Field | What it is |
|---|---|
| `id`, `album` | its id, and the album it is in |
| `name` | its number, Track.Shot.Version (pp. 3258, 3261): the track of picture of its clip (V1 is 1), the clip's place in the order of the thumbnail timeline (by start, then track from V1 up; from 1), and 1 (versions are not done); `""` where it is of no clip or was imported |
| `label` | what it is labelled (Change Label, p. 3262), at most 100 characters: `""` at first (of an import, the name of its file) |
| `created` | when it was grabbed or imported, RFC 3339 in UTC |
| `image` | `{"file", "format": "png" \| "exr", "width", "height"}` |
| `level` | which grade of its clip `grades.clip` is: `clip`, or its group's `pre_clip` or `post_clip` (the Node Editor's mode when it was grabbed, p. 3306) |
| `grades` | `{"clip": grade \| null, "timeline": grade \| null}`: the grade of its clip at `level` (null where it is of no clip) and the timeline's (null where the timeline's has no node: "if one is applied", p. 3286), with their keys |
| `frozen` | the same grades without keys: each keyed setting at the value of its key at the frame grabbed or the nearest before it (the first, before them all), "whatever parameter values are contained by the next keyframe to the left of the position of the playhead" (p. 3393: halfway between keys of 50 and 0, 50) |
| `keyframes` | of each grade, `null` or `{"counts", "rate", "start", "last", "frames", "from_start"}`: what frames its keys count (`source`: of the clip's media, at the media's rate; `clip`: of a title, colour or still clip from its first; `group`: of whatever clip a group's grade is on; `timeline`), from what `start` (the frame of the source at the clip's in point, or 0) to what `last` (of `source` only), the frames of its keys, and the same counted from `start`: the keys "reference the source timecode, or start frame of the original clips" (p. 3393) |
| `source` | where it came from (Still Properties, p. 3258): `project` (`""`, kept for later), `timeline` `{"id", "name", "rate"}`, `frame` and `record_timecode` (`HH:MM:SS:FF`, not drop frame, of a whole number of frames a second, the rate rounded up), `clip` (the clip as the project had it, its grade left out), `media` (as the project had it, or null), `group`, `track`, `shot`, `source_frame` (the frame its keys count there) and `source_timecode` (of `source_frame` at the media's rate, from the start of the media, which has no timecode of its own; null of a clip of no media); null of an imported picture |
| `color` | the project's `color` when it was grabbed |
| `imported` | of an imported still, the path it was taken from |

**Its picture** is what the viewer shows of the frame, after the output transform, as half
floats: of image A, never the wipe or split screen it is compared with, and with no subtitles.
It is written as a PNG of R'G'B' of 16 bits (each value of 0..1 times 65535, rounded) where every
value is within 0..1, and as an OpenEXR of half floats (channels B, G and R, uncompressed,
`INCREASING_Y`), every value as it is, where one is not (an output of scene-linear light, values
above white); `format` `"png"` or `"exr"` chooses instead (a PNG clamps). An HDR output of PQ or
HLG codes is within 0..1, and is kept as a PNG. `DaVinci Resolve` saves a DPX (p. 3259), which is
not done. A wipe shows a still as it is stored (see Comparisons in the viewer).

### Grabbing

`still.grab` (Grab Still, Ctrl+Alt+G, p. 3257) takes the frame that the timeline monitor shows
into `album` (the project's first album where it is left out); of the clip `clip` (the one the
client shows as current), else the one on top at the playhead; its grade at `level` (the Node
Editor's mode; `pre_clip` and `post_clip` need the clip in a group, `-32602` `{"reason":
"ungrouped", "clip"}` otherwise). A frame of no clip is grabbed with no clip grade. It needs the
timeline monitor: `-32601` in a headless engine; `-32004` `{"reason": "not_loaded"}` where no
timeline is loaded, `{"reason": "draw"}` where the frame cannot be read back. A still keeps
"that clip's embedded grade" (p. 3256), and the manual says nothing of Bypass Color or the
Highlight modes at a grab: where either shows the frame otherwise, the frame is drawn again
aside, graded and not highlighted, for the still alone (no view or scope sees it), and the
monitor goes on showing it as it did. `view` is
`"timeline"` (`-32602` `{"reason": "view"}` otherwise). Grab All Stills, Grab Missing Stills and
One Still Per Scene are not done. Each grab is logged as `still_grabbed` `{"width", "height",
"format", "read_ms", "written_ms"}`: of a frame of 3840 × 2160 (66 MB of half floats), a release
build reads it back in some 30 ms and writes it in some 0.3 s as a PNG (deflated fast) or 0.2 s
as an OpenEXR (RTX 4080 Laptop).

### Applying a still's grade

`still.apply` sets the grades of `to` (targets as in Grades) from the still, as one transaction:
one `set_grade` of each target (each once), worked out while the project is locked, so one undo
step, answered and told as `transaction.apply` is (with `revision` as there). It is worked out
exactly as `copy_grade` and `append_node_graph` work out a grade of the project, from the still's
grade given whole. A timeline's grade
takes the still's timeline grade, any other its clip grade: which grade is applied depends on
the Node Editor's mode, and a group's grade takes the clip grade (p. 3287). A still without that
grade is refused (`-32001` `{"reason": "no_grade", "still", "grade"}`).

- `mode` `"apply"` (Apply Grade, p. 3290; the default): the grade in place of the target's, as
  `copy_grade` sets it: the ids of its nodes kept; with `preserve_nodes` N (0–10, Copy Grade:
  Preserve number of nodes, p. 3285) a target of at least N nodes keeps its first N in place of
  the first N of the still's, each of their kind and inputs (`wrong_node` otherwise; a still of
  fewer nodes is `out_of_range` `preserve_nodes`).
- `mode` `"append"` (Append Node Graph, p. 3290): its nodes after the target's, chained as
  `append_node_graph` chains them, with new ids, and those that lie somewhere of their own moved
  right of the target's nodes as `append_node_graph` moves them. `preserve_nodes` is of applying
  (`-32602` `{"reason": "preserve_nodes"}`).
- `keyframes` (Apply Grades Using, pp. 3264, 3287; `"none"` by default, as the Gallery's own
  setting): `"none"` sets `frozen` (No Keyframes); `"source_timecode"` and `"start_frame"` set
  `grades` with its keys put on the frames of the target as `copy_grade` puts them, from the clip
  the still came from (`source.clip`, `source.media`): to the same time of the source, or as far
  from the start; where the two show no time of the source alike (or one has no source), from
  the start. A grade of a group, and one applied to a group, keeps its keys where they are.

Applied, the ids of the still's nodes are kept where the project could have made them, as
`copy_grade` keeps them; one of the engine's shape that this project has not made yet (of a still
of another project, a PowerGrade) is given a new id of the engine's, in the order of the nodes.
Appended, every node is given a new id of the engine's, as `append_node_graph` gives them.

`grade.append_node` is Append Node to Selected Clips (Color menu, p. 3297): the node `node`, a
corrector, of the grade `from`, alone, after the end of each grade of `to`, chained and given a
new id as above, its keys put as `keyframes` says (`"source_timecode"` by default, as
`copy_grade`). With `"none"` (No Keyframes, p. 3393) each keyed setting of the node is the value of
its key at the frame `at` of the timeline (the playhead; for a clip's grade, the frame of its
source there) or the nearest before it, and of its first key where `at` is left out. `at` is
of No Keyframes alone: given with `"source_timecode"` or `"start_frame"` (or no `keyframes`), it
is refused (`-32602` `{"reason": "at"}`). One `set_grade` of each, one undo step.

Refusals are those of a transaction (`-32001`, the project as it was): `{"reason":
"out_of_range", "what": "to" | "preserve_nodes"}`, `{"reason": "missing", "what": "timeline" |
"clip" | "group" | "node", "id"}`, `{"reason": "wrong_node", "node", "kind"}`, `no_grade`, and
those of the `set_grade` worked out.

### Albums and stills

- `gallery.list`: the project's albums, then the PowerGrade albums at the bottom (p. 3265), each
  `{"id", "name", "kind": "stills" | "powergrade", "stills"}` with its stills in order, and apart
  from them the project's album of the memories (`memories`, `"kind": "memories"`, or null where
  there is none), which the Gallery does not show; the one album where it is named, the memories'
  too.
- `gallery.thumbnail`: the thumbnail of a still `height` high (1–270, 90 by default, at most its
  own), as `media.thumbnail` answers.
- `gallery.album.create`: an album of the project (`"stills"`, the default) or of the
  PowerGrades, named `name` (1–100 characters), else `Stills N` or `PowerGrade N`, N one more
  than the most of that name ("numbered incrementally", p. 3265). `"memories"` gives the
  project's album of the memories A–H (pp. 3271–3272), made the first time (named `name`, else
  `Memories`): one a project, kept as a still album is, not listed among the albums, and taken by
  a grab or an import only where it is named. `gallery.album.rename` renames one;
  `gallery.album.delete` removes one with its stills for good (Remove Current Album, p. 3265), but
  not the last of its gallery that the Gallery shows (`-32001` `{"reason": "last_album",
  "album"}`; the manual says so of the PowerGrades, and the project keeps one too, for a grab to
  go into).
- `still.label` (Change Label; at most 100 characters, may be empty); `still.delete` (Delete
  Selected, p. 3258), every id checked first: each taken out of its album, its files moved into
  the gallery's `recycle/`, and where it was kept, so that it can be undone; a gallery keeps so
  the last 64 stills deleted, and one deleted before them is deleted for good, but never one of
  the same call, so that a call is always restored whole, however many it deletes. The index is
  written before the files are moved: where a file cannot be moved (another program holds it
  open), those moved are put back, the index is as it was and the call is refused (`-32002`
  `{"reason": "gallery"}`), nothing deleted; where the engine ends between the two, the gallery
  is set right the next time it is opened (logged as `gallery_settled`). A still of the album of
  the memories is deleted for good and takes none of the 64 places: it goes when its memory is
  saved over or cleared ("the previous memory is overwritten", p. 3272), not as the user's
  deletion.
  `still.restore` undoes it (Ctrl+Z in the Gallery, the interaction reference §9.2): each still
  back in its album at its place (the last deleted first, so that stills deleted together go back
  as they were; at the end where the album has fewer now; in the first album shown where its
  album is gone), under its id, its index and files changed as a deletion changes them, every
  one of a gallery checked first (`missing`
  `"what": "deleted still"` for one not deleted, or restored already); `restored` gives the album
  of each. `still.move` to another album (a still dragged onto an album, p. 3265): within a
  gallery it keeps its id, into the other it is copied under a new id and taken out of the first;
  `moved` gives where each is now. `still.copy` copies each into `album` under a new id (a still
  dragged onto a memory, p. 3272), within its gallery or into the other; `copied` gives the id of
  each copy.
- `still.export` (p. 3271): for each still its picture and its JSON into `dir` (absolute, a
  folder that is there), as `<prefix>_<name>.png|exr` and `.json` (`<name>` its number, or its id
  where it has none; without `prefix`, the name alone). A file there is `-32001` `{"reason":
  "exists", "path"}` unless `replace`. No `.drx` is made: the JSON holds the grades instead.
  Export With Display LUT is not done.
- `still.import` (p. 3270): each file (absolute) a still, read as media reads a still (PNG,
  TIFF, OpenEXR; not Y'CbCr, as JPEG), kept as a PNG of 16 bits or an OpenEXR as above; with the
  grades, label and source of a still's JSON of the same name beside it (as an export leaves
  it), else with none, labelled by the name of its file. A file that cannot be read is `-32002`
  `{"path", "reason": "not_a_still", "message"}`, and nothing is imported.

Every method that changes the Gallery tells every client `gallery.changed` `{}` before it
answers, so that each can call `gallery.list` again. They work in a headless engine (an
agent's), but for `still.grab`.

**Refusals.** `-32602` for params that are not the method's; `{"reason": "missing", "what":
"still" | "album", "id"}` for an id of no still or album of the Gallery (of one of no shape of
theirs, as `../s1`, too); `{"reason": "out_of_range", "what": "name" | "label" | "height" |
"prefix"}`; `{"reason": "relative", "path"}` for a path that is not absolute; `{"reason":
"not_a_folder", "path"}`. `-32002` `{"reason": "gallery", "path", "message"}` where a folder or
file of the Gallery cannot be made, read or written; `{"reason": "link", "path"}` where
`gallery`, the folder of the PowerGrades or `stills` in one is a link or a junction (see
Security); `{"reason": "write", "path", "message"}` where an export cannot be written.

Memories (Alt+1…8, p. 3271) are not kept by the engine: a client keeps which still each holds.

## Window presets

Window presets (RM20 pp. 3198–3199) are windows saved to be recalled on any node. As the
PowerGrades, they are this user's and shared by every project (`%COLUMBINA_HOME%\window_presets`,
else `%USERPROFILE%\.columbina\window_presets`, or the folder that `COLUMBINA_WINDOW_PRESETS`
names: `presets.json` `{"format": 1, "next", "presets"}` and a lock folder `.lock`, written whole,
one change at a time of all the engines that share it; one of another format is refused). No
project names a preset, and no change of the library is a step of a project's history.

- **`window_presets.save`** (Save as New Preset) keeps every window of the corrector `node` of
  the grade `target` (see Grades), in the list's order, ids, labels, On/Off and all, each as it is on
  `frame` of the grade's keys (see Frames of keys; the playhead): every parameter interpolated
  and clamped there as the mask uses it, and no keyframes or tracking data. It goes after the
  others, with a new id `wp<n>` (never given twice). `name` is 1 to 100 characters, trimmed.
- **`window_presets.update`** (Update Preset) puts the node's windows, kept so, in place of the
  preset's; its id, name and place stay.
- **`window_presets.delete`** (Delete Preset) removes it at once and for good, "no warning" and
  no undo (p. 3199). Projects whose nodes it was recalled on keep their windows.
- **`window_presets.apply`** (choosing a preset in the option menu) "overwrite[s] whatever other
  windows were set up in that node" (p. 3199): it is worked out, while the project is locked,
  into one `set_node` of the node's whole `windows`, the preset's in full, one transaction and
  one undo step, answered as `transaction.apply`. The windows can then be animated or tracked.
- There is no Rename Preset: the manual lists none.

Every method that changes the library tells every client `window_presets.changed` `{}` before it
answers. They all work in a headless engine.

**Refusals.** `-32602` for params that are not the method's; `{"reason": "missing", "what":
"preset", "id"}` for no such preset; `{"reason": "out_of_range", "what": "name"}`. `-32001`
`{"reason": "name_taken", "name"}` for a name a preset has already; `{"reason": "no_windows",
"node"}` to save a node without windows; `{"reason": "missing", "what": "timeline" | "clip" |
"group" | "node", "id"}`; `{"reason": "wrong_node", "node"}` for a node that is not a
corrector; and an apply refused as a transaction is (`stale_revision`, a locked node, windows of
a hand-edited library that are not valid). `-32002` `{"reason": "window_presets", "path",
"message"}` where the library cannot be made, read or written, or is of another format.

## The library of LUTs

**`luts.list`** (no params, or `{"folder"}`) lists the folders that a corrector's `lut` is
looked for in (see Grades in the picture), and the LUTs in them by the names that a node gives
them:

```
{"roots": [{"place": "project", "path": "C:\\…\\studio", "exists": true},
           {"place": "library", "path": "C:\\Users\\…\\.columbina\\luts", "exists": false}],
 "luts": [{"name": "LUTs/Film.cube", "root": 0, "size": 1234,
           "modified_us": 1790000000000000, "shadowed": false, "kind": "3d", "status": "ok"},
          {"name": "Log.cube", "root": 0, "size": 40, "modified_us": 1790000000000000,
           "shadowed": false, "kind": "1d", "status": "error",
           "message": "line 3: the file ends after 1 of the 2 rows of its tables"}],
 "truncated": false}
```

- `roots` are in the order a name is looked for in them: the folder of the project, then each
  folder of the library (`COLUMBINA_LUTS`, else `luts` in `COLUMBINA_HOME`, else in
  `.columbina` in the user's home). A folder of the library that is not there is listed with
  `"exists": false`.
- `luts` are the `.cube` files (the extension in any case) under each root, by root, then by
  name (code points). `name` is the path from the root, with `/` between folders, which is what
  a corrector's `lut` names it by; `root` is the index of its root; `size` is in bytes; and
  `modified_us` is when it was last written, in microseconds since 1970 (UTC), or `null`.
  `shadowed` is `true` where a root before it has a file of the same name (compared as the
  system compares names, without case on Windows): a node of that name reads that file instead.
- `kind` is what tables the file has: `"1d"`, `"3d"`, or `"shaper+3d"` (a 1D shaper before the
  cube, as Resolve writes them); `null` where it cannot be told. `status` is whether it parses
  as the engine reads it when it grades: `"ok"`; `"error"`, with `message` saying why (`line N:
  ...`, as `missing_luts` would; such a file cannot be used, and a node that names it grades as
  though it named none); or `"unchecked"`, not parsed by this list, its `kind` read from the
  keywords before its first row alone. `.3dl` and other formats are not listed.
- How a file is checked, and what it costs: each is parsed whole once, and what came of it is
  kept by its path, size and time of writing until one of them changes; the tables are not
  kept. A list parses only the files it does not know yet, four at a time, and begins none once
  2 s have passed since the list began, nor one longer than 64 MiB: those are `"unchecked"`,
  and a later list goes on with them. A file longer than any LUT the engine reads (1 GiB) is an
  `"error"` without being read. (A library of 50 cubes of 33³ and 65³, 107 MB, is parsed in
  about 0.2 s the first time, and listed in microseconds after.)
- `folder` lists only the LUTs under that folder of each root, still named from the root. It is
  a relative path of plain names, with `/` or `\`. One with `..`, a root or a drive is refused
  with `-32602`, `data: {"reason": "outside_roots", "folder": "<as sent>"}`; a root that does not
  have the folder lists nothing.
- At most 10000 files are listed, 100000 entries of folders looked at, and folders 16 deep; past
  that, `truncated` is `true` and what was found is given.

### Previews of LUTs

**`luts.preview`** draws small pictures of what LUTs do, for the thumbnails of the LUT browser
(pp. 2970, 3410–3411). It changes neither the project nor its history; a headless engine has it
too.

- `luts`: 1 to 64 LUTs, each named as `luts.list` names it, relative to the folders of LUTs
  (not empty, not absolute, never going up: `-32602` `{"reason": "name"}`; none or more than 64:
  `{"reason": "luts"}`). `height`: 1 to 270 pixels (`{"reason": "height"}`), 72 where left out.
- **On the test image** (no `frame`): "By default, all LUTs appear with a test thumbnail that
  give a preview of that LUT's effect" (p. 3410). The test image is the engine's own, 16:9: a
  sweep of hues across its top, full at the top and paler to its middle; a row of six patches
  (skin, sky, foliage, shadow, highlight, a grey of 0.5); and a ramp of grey from black to white
  along its bottom fifth, in R'G'B' read as shown. Each LUT is done to it on the processor by the
  tables and the reading between points that grade on the card, as `lut_interpolation` says, and
  its values clipped to 0..1 and made 8 bits.
- **On a frame** (`frame`: `{"timeline", "frame", "clip"?, "grade", "node"}`): how the current
  clip looks with the LUT on the node chosen (p. 3410: "a live preview of how the current clip
  looks with that LUT"; p. 3411), as Hover Scrub Preview shows it in the thumbnail (p. 3259): the
  frame drawn as `timeline.thumbnail` draws it (of the clip's layers alone, by its own grades,
  where `clip` is named), from a copy of the project whose node has the LUT in place of its own.
  A timeline, clip or frame refused as `timeline.thumbnail` refuses them; a node that cannot
  take the LUT (`set_node` would refuse it) is `-32602` `{"reason": "node"}`.
- It answers `{"previews": [...]}`, one for each LUT in the order asked: `{"lut", "width",
  "height", "png"}` (an opaque PNG of 8 bits, in base64), or `{"lut", "error"}` with why for a
  LUT that cannot be read (not there, or not parsed: the message of `luts.list`), the others
  drawn.

## Colour management

The colour management of a project is its `color` object, and the colour of media set by hand
the `color_override` of each media item (`crates/model/src/color_management.rs`), as DaVinci
Resolve's Project Settings › Color Management and its Media Pool menu set them (Reference Manual
20, pp. 149–153, 220–252). These are settings: what they do to the picture is done by the
compositor (see Grades in the picture), not by the model.

Both are always written; any field of them, or the whole object, may be left out where a
project is read, and is read as its default. A schema 2 project is upgraded with the defaults:
not managed, no media set.

### Spaces

A colour space is named as the engine's catalogue names it, in the style of Resolve's menus, and
is kept by that name, never by OCIO's. The catalogue has 68 spaces, and every one may be chosen
wherever a space is:

- the 48 spaces of media: each gamut of Rec.709, Rec.601 (PAL), Rec.601 (NTSC), Rec.2020,
  DCI-P3 and P3-D65 with each gamma of Gamma 2.4, Gamma 2.2, Gamma 2.6, Gamma 2.8, sRGB,
  Linear, ST2084 and HLG, named `<gamut> <gamma>` (`Rec.709 Gamma 2.4`, `P3-D65 ST2084`), but
  `sRGB` (Rec.709 sRGB), `Rec.2100 ST2084` and `Rec.2100 HLG` (Rec.2020 with PQ or HLG);
- the 20 spaces of cameras and of grading: `Rec.709 (Scene)`, `Apple Log`, `Apple Log 2`,
  `DJI D-Gamut/D-Log`, `Sony S-Gamut3/S-Log3`, `Sony S-Gamut3.Cine/S-Log3`,
  `Panasonic V-Gamut/V-Log`, `Canon Cinema Gamut/Canon Log 2`, `Canon Cinema Gamut/Canon Log 3`,
  `ARRI Wide Gamut 3/LogC3`, `ARRI Wide Gamut 4/LogC4`, `FujiFilm F-Gamut/F-Log2`,
  `Blackmagic Design Wide Gamut Gen 5/Film Gen 5`, `RED Wide Gamut RGB/Log3G10`,
  `DaVinci Wide Gamut/DaVinci Intermediate`, `DaVinci Wide Gamut/Linear`, `ACEScct`, `ACEScc`,
  `ACEScg`, `ACES (AP0) Linear`.

The input and the output space, and the input of media, may also be `bypass` (Bypass, p. 245).
The model's list and the catalogue of the OCIO crate are checked to be the same, one for one
(`crates/ocio/tests/names.rs`).

### The project's `color`

A field applies only where it says (see Refusals): **all** = always; **RCM** = `science` is
`yrgb_managed`; **manual** = RCM with `automatic` off; **Custom** = manual with `preset`
`custom`; **ACES** = `science` is `acescct` or `acescc`. Defaults that the manual does not give
are ours (marked *ours*), to be checked against Resolve.

| field | values | default | applies | page |
|---|---|---|---|---|
| `science` | `yrgb` (DaVinci YRGB), `yrgb_managed` (DaVinci YRGB Color Managed), `acescct`, `acescc` | `yrgb` | all | 149, 224–226 |
| `automatic` | Automatic color management | `false` *ours* | RCM | 230 |
| `processing_mode` | `sdr`, `hdr` (Color processing mode) | `sdr` *ours* | RCM with `automatic` on | 230 |
| `preset` | `sdr_rec709`, `sdr_p3_broadcast`, `sdr_p3_cinema`, `sdr_rec2020`, `davinci_wide_gamut`, `hdr_p3_broadcast`, `hdr_rec2020`, `custom` | `sdr_rec709` | manual | 230–232 |
| `output` | a space, or `bypass` | `Rec.709 Gamma 2.4` *ours* | RCM | 227, 232 |
| `separate_gamma` | Use Separate Color Space and Gamma (how the menus show a space; it is kept by one name) | `false` | Custom | 150, 234–235 |
| `input` | a space, or `bypass`: of media that says none | `Rec.709 Gamma 2.4` | Custom | 230, 235 |
| `timeline` | a space | `Rec.709 Gamma 2.4` | Custom | 226, 236 |
| `working_luminance` | `sdr_100`, `hdr_500`, `hdr_1000`, `hdr_2000`, `hdr_4000`, `hdr_er_1000_2000`, `custom` | `sdr_100` *ours* | Custom | 238 |
| `custom_luminance` | nits, 1–10000 | `100` *ours* | Custom, `working_luminance` `custom` | 238 |
| `hdr_203_nits` | Use 203 nits reference for Rec.2100 HDR (a client shows it only for an HDR output and an SDR timeline) | `false` | Custom | 238–239 |
| `limit_gamut` | `null` (None) or a gamut: `rec709`, `rec601-pal`, `rec601-ntsc`, `rec2020`, `dci-p3`, `p3-d65` | `null` | Custom | 239 |
| `input_drt` | `none`, `simple`, `luminance`, `davinci`, `saturation_preserving` | `none` *ours* | Custom | 239–240 |
| `input_sat_rolloff` | `{start, limit}` nits, 1–10000, `start < limit` | `{100, 1000}` *ours* | Custom, `input_drt` `saturation_preserving` | 240 |
| `output_drt` | as `input_drt` | `none` *ours* | Custom | 240–242 |
| `output_sat_rolloff` | as `input_sat_rolloff` | `{100, 1000}` *ours* | Custom, `output_drt` `saturation_preserving` | 242 |
| `inverse_drt` | Use inverse DRT for SDR to HDR conversion | `false` *ours* | Custom | 242 |
| `white_point_adaptation` | Use white point adaptation | `true` *ours* | Custom | 242 |
| `color_space_aware_tools` | Use color space aware grading tools | `true` | Custom | 243 |
| `resize_in` | `timeline`, `log`, `linear`, `linear_mapped`, `gamma`, `gamma_mapped` | `linear` | Custom | 243 |
| `graphics_white` | Graphics white level, nits, 1–10000 | `100` *ours* | RCM | 243 |
| `aces_version` | `1.3`, `2.0` | `2.0` *ours* | ACES | 150, 247 |
| `idt` | a space (ACES Input Device Transform) | `Rec.709 Gamma 2.4` *ours* | ACES | 247–248 |
| `odt` | an ODT of `aces_version` (below) | `Rec.709` *ours* | ACES | 248–250, 252 |
| `gamut_compress` | Apply ACES reference gamut compress | `false` *ours* | ACES | 251 |
| `node_luts_in` | `timeline` (ACEScc AP1 Timeline Space), `ap0_linear` (ACES AP0 Linear) | `timeline` | ACES | 250 |
| `lut_interpolation` | `trilinear`, `tetrahedral` (3D Lookup Table Interpolation) | `trilinear` | all | 152–153 |
| `input_lut`, `output_lut`, `viewer_lut` | `{"1d": name or null, "3d": name or null}`: Input, Output and Color Viewer Lookup Table; the 1D is done before the 3D | none | all | 151–152 |
| `s_curve_contrast` | Use S-curve for contrast (General Options) | `true` | all | 156 |

- A LUT is named relative to the folders of LUTs, as `luts.list` names it: not empty, at most
  4096 characters, not absolute (no leading `/` or `\`), with no `:`, NUL or `..` part.
- A **preset** other than Custom sets what is in force for the timeline, the working luminance,
  the DRTs and the gamut limit, and keeps the input at `Rec.709 Gamma 2.4` (p. 230); the stored
  Custom fields stay as they are and come back into force with Custom. The manual names each
  preset's gamut and range but not its settings, so the table is ours
  (`Preset::settings`, one place to change):

  | preset | timeline | luminance | DRT in / out | limit | output chosen with it |
  |---|---|---|---|---|---|
  | `sdr_rec709` | Rec.709 Gamma 2.4 | SDR 100 | none / none | – | Rec.709 Gamma 2.4 |
  | `sdr_p3_broadcast` | P3-D65 Gamma 2.4 | SDR 100 | none / none | – | the same |
  | `sdr_p3_cinema` | DCI-P3 Gamma 2.6 (no P3-D60 in the catalogue) | SDR 100 | none / none | – | the same |
  | `sdr_rec2020` | Rec.2020 Gamma 2.4 | SDR 100 | none / none | – | the same |
  | `davinci_wide_gamut` | DaVinci Wide Gamut/DaVinci Intermediate | HDR ER 1000/2000 | davinci / davinci | – | Rec.709 Gamma 2.4 |
  | `hdr_p3_broadcast` | P3-D65 ST2084 | HDR 1000 | davinci / none | P3-D65 | the same |
  | `hdr_rec2020` | Rec.2100 ST2084 | HDR 1000 | davinci / none | – | the same |

  Choosing a preset does not set `output`: a client that wants the output to follow it sends
  both in one patch. Automatic colour management is taken as `sdr_rec709` (`processing_mode`
  `sdr`) or `hdr_rec2020` (`hdr`) (ours; p. 230 only says it chooses for the media).
- **Tone mapping**: `none` is a plain transform, clipped out of gamut. Resolve publishes the
  mathematics of none of the others; `simple`, `luminance`, `davinci` and
  `saturation_preserving` are the engine's own, made to behave as pp. 239–242 describe, and are
  approximations of Resolve's. RED IPP2 (Output DRT) is not offered. The ODTs of ACES are OCIO's
  own, exact.
- **ACES versions and ODTs** are those of OCIO's built-in studio configs:
  `ocio://studio-config-v2.2.0_aces-v1.3_ocio-v2.4` for `1.3` and
  `ocio://studio-config-v5.0.0_aces-v2.1_ocio-v2.6` for `2.0` (Resolve's 1.0.3, 1.1 and 1.2 are
  not offered). Each ODT is a display and a view of that config (every tone-mapped view of
  every display), the spaces `ACEScct`, `ACEScc`, `ACEScg`, or `No Output Device Transform`
  (ACES2065-1, p. 252). ADX and DCDM are not in the configs and not offered.
  - `1.3`: `Rec.709`, `Rec.709 (D60 sim.)`, `sRGB`, `sRGB (D60 sim.)`, `Display P3`,
    `Display P3 (D60 sim.)`, `P3-D60`, `P3-D65`, `P3-D65 (D60 sim.)`, `P3-D65 (Rec.709 Limited)`,
    `P3-DCI (D60 sim.)`, `P3-DCI (D65 sim.)`, `Rec.2020`, `Rec.2020 (P3-D65 Limited)`,
    `Rec.2020 (Rec.709 Limited)`, `Rec.2020 HLG (1000 nits)`, `Rec.2020 ST2084 (1000 nits)`,
    `(2000 nits)`, `(4000 nits)`, `P3-D65 ST2084 (108 nits)`, `(1000 nits)`, `(2000 nits)`,
    `(4000 nits)`.
  - `2.0`: `Rec.709`, `Rec.709 (Gamma 2.2)`, `sRGB`, `Display P3`, `Display P3 HDR (1000 nits)`,
    `P3-D65`, `P3-D65 (Rec.709 Limited)`, `P3-D65 ST2084 (108 nits)`, `(500 nits)`,
    `(1000 nits)`, `(2000 nits)`, `(4000 nits)`, `Rec.2020 ST2084 (500 nits)`, `(1000 nits)`,
    `(2000 nits)`, `(4000 nits)`, the same four `…, P3-D65 Limited)`,
    `Rec.2020 HLG (1000 nits, P3-D65 Limited)`, `Display P3 HDR (100 nits, Rec.709 Limited)`,
    `Rec.2020 ST2084 (100 nits, Rec.709 Limited)`, `P3-D65 ST2084 (100 nits, Rec.709 Limited)`.

  The display and view of each are in `AcesVersion::odts` of the model.
- Not modelled: Dolby Vision, HDR10+, HDR Vivid, HDR mastering, Broadcast Safe, ACES AMF, the
  Video Monitor and Scopes LUTs, Display HDR On Viewers (a preference of the viewer, T12).

### Media's `color_override`

`{"input": null, "levels": "auto", "lut": {"1d": null, "3d": null}, "inverse_drt": true}`:

- `input`: a space, `bypass` (Bypass Color Management, p. 245) or `null` (the project's default,
  the menu's Project Default). It decides the input space over the file's metadata and the
  project's default, in the order of p. 228: camera raw (`prores_raw`, `braw`, `r3d`, `arriraw`)
  is its maker's and cannot be set (`camera_raw`); a space set by hand; the file's metadata; the
  project's `input` (or `Rec.709 Gamma 2.4` under a preset), or `idt` in ACES.
- `levels`: `auto`, `video`, `full` (Data Levels, pp. 221–222).
- `lut`: the LUTs of the source clip (Applying LUTs to Source Clips, pp. 3409–3410: the 1D LUT ▸
  and 3D LUT ▸ menus of the media pool and of the Color page's thumbnails), each named relative
  to the folders of LUTs as the project's are (else `out_of_range`, `what: "media_lut"`), or
  `null`. They are the media's, not a grade's (`ColorTrace` "cannot" copy them); see What colour
  management does to the picture.
- `inverse_drt`: Inverse DRT for SDR to HDR Conversion of the clip (p. 233): `true`, the default,
  lets the project's `inverse_drt` map its picture up; `false` keeps it from being mapped up
  where the project's would ("you can disable this behavior on a clip by clip basis in the Media
  Pool clip contextual menu, or the Thumbnail Timeline contextual menu").
- Media without a picture keeps the defaults (`no_stream`).
- Each field is written; one left out, as by a project written before, is read as its default.

### Operations

| op | fields | inverse |
|---|---|---|
| `set_color_settings` | `settings`: a patch of `color` | `set_color_settings` with the values as they were |
| `set_media_color` | `id`, `input?` (a space, `bypass`, or `null`), `levels?`, `lut?` (`{"1d", "3d"}`, both, each a name or `null`), `inverse_drt?` | `set_media_color` with the given fields as they were |

- `settings` is merged as `set_node` merges `params`: each object in it (a LUT pair, a rolloff)
  is put into the object of that name, and any other value takes the place of the one there,
  whole. `{"output_lut": {"3d": "Film/Kodak 2383.cube"}}` keeps the 1D LUT. What it makes is
  checked whole, every field whether it applies or not.
- Its `done` and inverse carry the values at the patch's paths as the engine writes them (`500`
  comes back as `500.0`). Every field stays as it was set when the science or preset changes,
  as Resolve keeps them; switching back puts them in force again.
- A `set_color_settings` or `set_media_color` that sets every given field to the value it has
  already changes nothing: alone in a transaction it is refused as `empty` and makes no step.
- `set_media_color` may name media by a `$name` given in the same transaction.

### What colour management does to the picture

The timeline monitor and an export draw a project as its `color` says
(`crates/server/src/managed.rs`; the transforms are OCIO's, the tone mappings ours):

- **`yrgb`**: nothing. Decoded R'G'B' is held to 0..1, as before; an export is tagged BT.709:
  a project that is not managed is drawn to the bit as before colour management.
- **`yrgb_managed`** (RCM): each picture from its input space (`media_input`: by hand, else as
  its file says, else the project's) into the timeline space, never clipped; through the Input
  DRT where it maps (from the most of the space, 100 nits for SDR, 1000 for HLG, the mastering
  display's or 10000 for PQ, 10000 for linear light and the spaces of cameras, to the working
  luminance; up only with `inverse_drt`, and the media's own `inverse_drt` on); titles and colours from Rec.709 Gamma 2.4, their white
  at `graphics_white`. Grades are done in the timeline space, whose primaries give their luma.
  Then into the output space: the Output DRT (from what the timeline holds to the most of the
  output), the reference of 203 nits, `limit_gamut` (clipped below 0 in the smaller gamut) and,
  where `white_point_adaptation` is off, the adaptation of white undone; then clipped to 0..1. A
  preset other than Custom maps between SDR and HDR by the rule of pp. 232–233 where its output
  is of the other range. `bypass` is no transform.
- **`acescct`, `acescc`**: each picture by its IDT into the timeline (with the reference gamut
  compression of ACES 1.3 where `gamut_compress`), titles as Rec.709, grades in AP1, and out
  through the ODT, a view of the built-in config of `aces_version`, clipped.

The DRTs (`simple`, `luminance`, `davinci`, `saturation_preserving`) are ours: the curve of the
rule of pp. 232–233 (no change below half the target, then cubics of Hermite through its knots:
for 1000 to 100 nits, 50 → 50, 100 → 90, 1000 → 100), on each channel (`simple`, from 55 times
the target, as "5500 nits to 100", p. 2089) or on luminance (the others), `davinci` greying the
brightest and darkest, `saturation_preserving` by its Sat. Rolloff (`crates/compose/src/tone.rs`).

**The project's LUTs** (`input_lut`, `output_lut`, `viewer_lut`, the Lookup Tables of pp.
151–152) are done in every science, `yrgb` too, each level's `1d` before its `3d` (p. 152), a
cube read between its points as `lut_interpolation` says, at the places of the overall order of
p. 3330:

- the **Input LUT** on each picture of media after its input transform (RCM's into the timeline
  space, or the IDT of ACES) and before its grades, "before every other image processing
  operation" of the Color page; not on titles and colours, which are no source;
- the **LUTs of a source clip** (`color_override.lut`, its `1d` before its `3d`) on the pictures of
  its media alone, after the Input LUT and before their grades, in every science, read as the
  project's are; one that cannot be read is told likewise;
- the **Output LUT** on the frame after the timeline grade and the subtitles, before the output
  transform (RCM's, or the ODT of ACES): "after the operations applied in the Color page, but
  before" the display LUT;
- the **Color Viewer LUT** over the frame as the view of the timeline monitor shows it, after
  any comparison of the viewer (wipe or split screen), and before the view's own mapping to the
  display (HDR or SDR): on that view only. An export never has it, nor the frame that the scopes
  measure ("unbiased", p. 152: there is no Scopes LUT), `view.sample` reads, a still is grabbed of
  and `transport.frame_digest` hashes; nor the source monitor, which shows the media as they are.

A cube clips what is beyond its domain (0–1 where the file says none, p. 225), so a level whose
LUT is of display-referred values clips a timeline of scene-referred ones, as in Resolve. A LUT
that cannot be read is left out of its level, and told as `missing_luts` (Grades in the
picture); a project whose LUT could not be read makes its colour again at the next change of the
project, so that a file put in its place is found. With none of the three named, nor any of a
source clip, `yrgb` is drawn to the bit as before. The view leaves the Color Viewer LUT out while
`viewer.lut` says so (Live Preview and the Color Viewer LUT).

A change of the colour management, while a timeline is loaded in the monitor, is made ready on a
thread of the engine's own and its shaders compiled there; the monitor shows the colour it had
until then (up to a second and a half for an ODT of ACES 2.0), and then draws again; the
timeline state says `color_pending: true` meanwhile, and `color_error` where it cannot be made
(Grades in the picture). A load, and an export, wait for it. The timeline monitor's view is told to show HDR (PQ or HLG, and its most
nits) where the output space is of HDR; the source monitor's where the media's file says PQ or
HLG.

An export is tagged with its output space: its primaries, transfer and matrix (BT.2020
non-constant luminance for Rec.2020 and P3, the Y'CbCr made by that matrix), limited range; and
of PQ, the mastering display (the primaries of `limit_gamut` or of the output, D65, 0.0001 to
the most nits of the output) and the light of its content, `MaxCLL` and `MaxFALL`, measured of
every frame it writes. An output of HDR is written in 10 bits (HEVC Main 10, or ProRes); H.264,
which is written in 8, is refused for it (`render.start`, `-32602` with `{"reason":
"hdr_codec"}`). A space of a camera or of grading as the output is tagged unspecified.

### Refusals of colour

| reason | fields | meaning |
|---|---|---|
| `unknown_space` | `setting`, `name` | `input`, `timeline`, `output` or `idt` names no space of the catalogue (or `bypass` where it may not be); `odt` names no ODT of `aces_version` (so does a change of `aces_version` alone, whose ODT the other version lacks: set both); media's `input` names no space |
| `inapplicable` | `setting` | a field of the patch applies neither to the settings before nor to those after it (an `odt` while not in ACES, a `timeline` under a preset, `custom_luminance` while the working luminance is not `custom`). Before *or* after, so that the inverse of any change, which sets back what it set, is never refused |
| `color_settings` | `why` | `settings` is not an object, or what it makes is not colour settings: an unknown field (`{"gamma": 2.4}`), a value of the wrong kind or out of its list (`"science": "legacy"`, `"aces_version": "1.2"`, `"input_drt": "red_ipp2"`), a LUT pair that is not an object; serde's message |
| `camera_raw` | `media` | `set_media_color` of a space on camera raw (`bypass` and `null` are allowed) |

`out_of_range` also names `custom_luminance` and `graphics_white` (not 1–10000 nits),
`input_sat_rolloff` and `output_sat_rolloff` (not 1–10000, or not `start < limit`),
`limit_gamut` (`camera-native`), and `input_lut`, `output_lut`, `viewer_lut` (a name that is not
relative to the folders of LUTs). `missing` `media` and `no_stream` (media without a picture set
to anything but the defaults) as elsewhere. A project read with colour settings or a colour of
media that the operations would refuse is refused as invalid.

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
| `project.changed` | `{"revision": n, "base"?: b, "by": "<kind>", "made_by": "<kind>", "done": [op, ...], "next_id": k}` (`base` where several changes of a gesture are told together, see Gestures) |
| `engine.error` | `{"message": "...", "kind": "save"}` |
| `transport.state` | the state of the transport (see The transport) |
| `engine.yielding` | `{"to": "desktop"}`: the engine is about to end, to yield the project to the desktop's engine (see `engine.yield`) |
| `media.proxy` | `{"media": "m1", "state": "queued" \| "running" \| "done" \| "failed", "progress": 0.5, "message"?: "..."}` (see Proxies) |
| `media.waveform` | `{"media": "m1"}`, or `{"media": "m1", "stream": 1}` for the waveform of a stream (background analysis finished; query again for data or error) |
| `render.progress` | `{"job", "state": "running" \| "done" \| "failed" \| "cancelled", "frame", "frames", "fps", "path", "encoder"?, "message"?, "missing_luts"?, "measuring"?, "normalization"?}` (see Rendering a timeline, Grades in the picture, and The sound of an export) |
| `scopes.frame` | `{"scope_id": "scope-1", "view": "timeline", "frame": n, "width": w, "height": h, "rgba": "<base64>"}`: a picture of a scope, to the connection that subscribed it only (see Scopes) |
| `gallery.changed` | `{}`: the Gallery was changed; call `gallery.list` again (see The Gallery) |
| `window_presets.changed` | `{}`: the window presets were changed; call `window_presets.list` again (see Window presets) |
| `view.pointer` | `{"monitor", "kind": "down" \| "move" \| "wheel" \| "up" \| "cancel", "button", "buttons", "delta", "clicks", "middle_moved", "x", "y", "inside", "window": {"x", "y"}, "shift", "ctrl", "alt"}`: what the pointer did on a view that takes it, to the connection that asked only (see Samples and the pointer) |
| `window.state` | `{binding,capture,sequence,phase,reason,selected,event,revision,keys,selected_points}`: owner-only arming, armed, refused and ended states (see Circular and Gradient Window controls) |
| `window.unbound` | `{binding,sequence,revision,reason:"project_changed"}`: original-owner-only retirement when a project change invalidates the binding, even without capture |
| `window.points` | `{binding,sequence,revision,selected_points:[]}`: owner-only, when a project change drops the selected Curve's point selection (see Curve Window controls) |
| `track.progress` | `{job,frame,timeline_frame,done,total,lost,revision}`: to the connection that started the track, each time the track so far is written (see Window tracking) |
| `track.state` | `{job,phase,reason,frames,lost,frame,revision,tracked,message?}`: how the track ended (`done`, `stopped`, `failed`), to the connection that started it |
| `automation.written` | `{"timeline", "punch", "lanes", "revision", "failed": [...]}`: a pass of the recording of automation, or a Punch In, written, to every connection (see Recording automation) |
| `audio.analysis` | `{job, state: "running", done, total}` (seconds of sound measured, at most 4 a second), then `{job, state: "done", method, ...}`, `{job, state: "failed", message}` or `{job, state: "cancelled"}`: how an `audio.analyze`, `audio.normalize` or `audio.level_match` goes and ends, to every connection (see Meters, loudness and the analysis of sound) |

- `by` is the kind of the client that asked (for an undo or a redo, the one that asked for
  that); `made_by` is the kind of the client that made the transaction (for an undo or a redo,
  the one that made the step taken back or done again). Both are `"engine"` for a change that
  the engine made itself (a proxy set on its media), which is no step of the history.
- **Keeping a copy.** Call `project.get` after `hello`; then for each `project.changed`: ignore
  it if `revision` is not above yours; if it applies to yours (its `base`, or `revision − 1`
  where it has none), set your `next_id` to its `next_id` and apply `done` in order; if it
  applies to another revision (which the engine never sends on one connection), call
  `project.get` again. A client that does not read `base` stays right: it fetches the project
  again where a gesture's changes were told together.
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
- **The library of LUTs is listed within its roots.** `luts.list` reads the names of folders
  and files only under the folder of the project and the folders of the library, and the
  contents of the `.cube` files listed there (to tell their kind, never written). It does not go through a link or a junction, neither one that the `folder` asked for
  passes through nor one found below (nor into `cache` of the project, the engine's own), and it
  refuses a `folder` that is not a path of plain names within them. A corrector's `lut` itself
  may still name any absolute path, which the engine reads as a `.cube` when it grades (see
  Grades in the picture).
- **A LUT is written where it is asked for.** `grade.export_lut` writes one `.cube`: by a name
  within the folder of the project (plain names, through no link or junction, not into
  `cache`), or at an absolute path that the client names, as an export of a timeline is, its
  folder there already and not the project's cache, and never over a file of the project's
  media or proxies, a folder or a link; over another file only with `replace`.
- **The Gallery is the engine's own.** Its folders are `<project>/gallery` and the folder of the
  PowerGrades (see The Gallery), which the engine makes; it does not use one that is a link or a
  junction (nor `stills` in it), and reads, writes and deletes only `gallery.json` and the files
  `stills/<id>.json|png|exr|thumb.png` of ids of its own shape, never through a link. A client
  names a still or an album by its id only. `still.export` writes into the folder a client
  names, and `still.import` reads the files it names, as an export and an import of media do.
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
| `-32601` | no such method; `view.*` (but `view.sample` of a frame drawn off screen), `transport.*` and `scopes.*` (but `scopes.statistics` of a frame drawn off screen) in a `--headless` engine; `media.make_proxy` in an engine that makes no proxies; `engine.yield` in an engine that is not headless | |
| `-32602` | params are not what the method takes (or not an object); an operation that cannot be read; a bad `$name`; a speed of `transport.play` out of −8…8 or 0; `transport.scrub_audio` of the source monitor (`{"reason": "monitor"}`); `transport.load` of media or a timeline not in the project; `timeline.duration` of a timeline not there; `media.import` or `media.inspect` with no paths or a relative path; `media.thumbnail`, `media.waveform` or `media.make_proxy` of media not there; `media.waveform` of a `stream` that is not a stream of sound of the media; `media.thumbnail` of a frame outside the media, a height outside 1…270 or a width over 16384; a bad parent or rectangle of `view.*`; `render.start` with a bad path, codec, quality, timeline, range (an empty timeline has none, so it cannot be exported), `subtitle_tracks` or `audio` (see The sound of an export), and `render.cancel` of a job that is not there; `scopes.*` with bad settings, a scope that is not the connection's, too many scopes or bins out of range; `luts.list` with a `folder` outside the roots; `grade.export_lut` of a size, path, grade or node that it does not take; `transaction.apply` with a `gesture` id that is empty or longer than 64 characters; `view.sample` off the frame or of an even size, and `view.sample` or `scopes.statistics` naming a node that cannot be drawn | `{"index": i, "reason": "invalid_op"}` for an operation that cannot be read; see names above; `{"reason": "speed"}`; `{"index": i, "path": "...", "reason": "relative"}`; `{"reason": "parent"}` or `{"reason": "rect"}` (`view.*`); `{"reason": "missing", "media": "..."}` or `{"reason": "missing", "timeline": "..."}` (`render.start` too); `{"reason": "stream", "stream": n}` (`media.waveform`); `{"reason": "out_of_range", "frame": n}`; `{"reason": "subtitle_track", "track": "..."}` for a `subtitle_tracks` entry that is not a subtitle track of the timeline, or is named twice; `{"reason": "bus" \| "rate" \| "audio_codec" \| "bitrate" \| "standard" \| "target", ...}` (the `audio` of `render.start`); `{"reason": "settings" \| "missing" \| "too_many" \| "bins", ...}` (`scopes.*`, see Scopes); `{"reason": "outside_roots", "folder": "..."}` (`luts.list`); `{"reason": "size" \| "path" \| "missing" \| "not_a_corrector" \| "cannot_bake", ...}` (`grade.export_lut`, see Generate LUT); `{"reason": "gesture"}`; `{"reason": "outside" \| "size" \| "monitor" \| "node" \| "grade" \| "timeline" \| "clip"}` (see Samples and the pointer) |
| `-32001` | a transaction, undo or redo was refused; the project is exactly as it was; or `render.start` was refused before anything was written or taken away (with `{"reason":"stopping"}` while the engine is stopping; with no `data` when the export could not be started) | the refusal, below |
| `-32002` | `media.import` or `media.inspect`: a file cannot be read; nothing was added. `transport.load`, `media.thumbnail`, `media.waveform`: the file of the media cannot be opened or decoded. The analysis of the engine stopped (no `data`) | `{"path": "<as sent>", "reason": "...", "message": "..."}`, below |
| `-32003` | `view.*`: the view cannot be made, there is none, or its window is gone | `{"reason": "..."}`, see The view |
| `-32004` | `transport.*`: no media is loaded, the transport has stopped, or the test hook could not draw; `scopes.statistics`: the monitor shows nothing, or its frame could not be measured | `{"reason": "..."}`, see The transport; `not_loaded` or `draw` (see Scopes) |

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
| `missing` | `what` (`media`, `bin`, `timeline`, `track`, `clip`, `transition`, `cue`, `named output`, `key`), `id` | the thing named is not there (a `key` of `clip_gain_keys` is named `ticks/scale`) |
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
| `in_use` | `what`, `id`, `by` | media (`what: "media"`) still used by clip `by`; a track of sound (`what: "track"`) that group of tracks, track or bus `by` names (as a member, or as the source of a sidechain or a Ducker); a bus (`what: "bus"`) that track or bus `by` outputs or sends to or drives a sidechain or Ducker of, or that timeline `by` monitors; a VCA (`what: "vca"`) that track, bus or VCA `by` is a member of |
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

### Refusals of sound (schema 4)

| reason | fields | meaning |
|---|---|---|
| `bus_cycle` | `id` | the sound would go round in a loop through track or bus `id`, by outputs, sends, sidechains or duckers |
| `bus_depth` | `id` | a path of buses, each feeding the next, would be more than 6 buses long, ending at bus `id` |
| `too_many_inserts` | | a strip with more than 6 inserts |
| `too_many_sends` | | a strip with more than 10 sends |
| `too_many_outputs` | | a strip with more than 10 outputs |
| `channel_count` | `track` | a track's `channels` do not fit its format (an adaptive one has 1–24), or a clip on it names another number of channels |
| `unknown_bus` | `id` | an output, a send, a sidechain, a ducker or the monitor names a bus that is not there |
| `unknown_vca` | `id` | a strip or a VCA names a VCA that is not there |
| `order` | | a strip's `order` is not `fx`, `dynamics` and `eq` each once |
| `vca_cycle` | `id` | VCAs would be members of each other in a loop |
| `vca_depth` | `id` | a chain of VCAs would be more than 6 long |
| `unknown_group` | `id` | an operation names a group of tracks that is not there |
| `audio_settings` | `why` | a patch of sound (`set_track_audio`, `set_bus`, `set_vca`, `set_track_group`, `set_timeline_audio`) is not an object, names a field that is not known or may not be set so (`id`, `lanes`), or holds a value of the wrong kind; `why` is the reason |

Of automation (T14): `missing` with `what: "lane"` (`id` its path) or `what: "point"` (`id`
`ticks/scale`); `out_of_range` with `what` `lane` (a lane of no setting, of another kind or scale,
of no point, out of the range of its setting, or a switch that is neither 0 nor 1; an insert that
keeps its id with an effect of another kind while a lane names it), `lane_integer` (of a whole
number), `lane_not_wired` (of a setting that the graph does not move as a lane would), `points`, `at`,
`from`, `to`, `by`, `value`, `delta`, `tolerance` or `paths` (see Automation, above);
`wrong_track` with `what: "lanes"`; `empty_range` of a range or a move of nothing. Of
`record_pass` (T16): `out_of_range` with `lanes`, `runs`, `points`, `value`, `to`, `glide` or
`trim` (see Recording automation). Of `lane_fill`, `automation_paste` and `automation_erase`
(T16b): `out_of_range` with `lanes`, `of`, `value`, `points`, `at` or `to`; `empty_range` where
nothing would change (every owner Safe, every category off).

A track that a group, a sidechain or a ducker names is refused as `missing` (`what: "track"`)
where it is not there; `remove_track` of it while it is named is refused as `in_use`. `out_of_range` also
names (schema 4): of a clip `gain_db`, `pan` (also their keys), `fade`, `eq`, `pitch`,
`channels` (a level above 24; a stream or channel that the media has not is silence, never
refused), `sidechain`; of a transition `level` (a level other than 0 dB on one that is not a
crossfade); of a strip `trim_db`, `track_fx` (on a bus), `fader_db`, `pan`, `send`, `outputs`
(two to one bus), `inserts` (two of one id), `vcas` (one twice, or one and a VCA that it is a
member of), `eq`, `dynamics`, `ducker`, the kind of an effect, `lane`; of a VCA `vcas` (as of a
strip); of a timeline `buses`, `vcas` (more than 64), `audio` (its settings), `label` (a bus name
over the longest label); of a group `members`, `linked` (one twice), `label` (a name of a VCA or a
group over the longest label); of the edits of sound `items`, `clips`, `to` (a move onto a key, or
`copy_effects` to none), `at` (a time of scale 0), `interp` (`set_interp` without one) and
`gain_db` (a `delta_db` that is not finite). `set_track_audio` and the operations on the inserts
of a track that is not of sound are `wrong_track` (`what: "strip"`); a `slot`, `from` or `to`
outside a chain of inserts is `out_of_list`. A bus named as a track of
its timeline is refused as `taken` (`what: "track"`). A project read with one of these is
refused with `what` of `bus`, `vca` or `track_group` where the thing at fault is one.

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
scene compositor as preview, and the same graph of the sound (see The sound of an export). It
never uses proxies.
Edits, undo, or deletion of that timeline after start do not change the export. Titles are
composited into the picture. Subtitle tracks are **not burned into the picture**.
The GPU is shared with preview: a waiting preview composition takes the next turn; export
submits and reads back one frame at a time. If the graphics device is lost during an export
(a driver reset), the export waits up to 30 s for a new device, draws the frame it was at
again and goes on: no frame was written for it yet, so the file is whole; the final `message`
says the device was made again. If none can be made, or it is lost three times on one frame,
the export fails, saying so. An encoder that fails after it opened (a frame, the sound or
finishing the file) fails the export with a message naming the encoder and the frame; the
ProRes fallback is only ever taken when no encoder opens, never in the middle of a file. The
sound is AAC at 192 kb/s by default, at the rate of the timeline, in the channels of the bus
monitored (see The sound of an export); a film of a timeline with no enabled clip on an enabled
audio track has no sound. H.264 uses 8-bit picture; HEVC uses
10 bits when the project contains media of more than 8 bits or its output is of HDR; ProRes always
10 bits. The picture is tagged with the colours of the project's output space (Colour management,
What colour management does to the picture). If no hardware
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

### The sound of an export

Plan T6 of phase 3. The export runs the same graph of the sound as playback (the timeline's
tracks, buses, VCAs, effects and lanes), built for export from the snapshot: a Listen of a
sidechain is never exported, every clip is read by a reader that waits for it (a clip that is
heard and cannot be read fails the export), and the graph starts at the first sample of the
range with the preroll that playback from the same sample has (what the strips need to settle,
and what each clip under way there needs, its equalizer and effects and the strips after them,
but no further back than where its effects start; at most 10 s). So the sound exported from a sample is, to the bit, the sound that playback from that
sample gives the device (the null test: a playback captured with `--sound capture=<path>` less an
export as `pcm_f32le` is nothing but zeros), where:

- the file is at the rate of the device (another rate is made by another filter);
- the bus is stereo, or is compared as the device hears it (another format is folded into two
  sides by the device);
- no sample goes beyond ±1, or the file is of floats (the device's copy, and AAC and PCM of 24
  bits, are clipped);
- the playback started where it was asked: a playback whose graph is not built and prerolled
  within 2 s goes on by the clock of the system and starts its sound again half a second ahead of
  the picture (see The transport), at a sample of its own and not at the requested frame. The
  session that a playback starts as the device is first made ready starts where the clock is by
  then, a sample or so after the frame; the digest's `audio_sample` less `audio_samples` is where
  each session started.

An export of a range and one of the whole timeline come within −90 dBFS of each other after the
preroll where the graph settles within 10 s.

`audio` is optional, every field of it too; unknown fields are refused:

- `bus`: the bus rendered, a main output (a bus that outputs to no bus) or the bus monitored; by
  default the bus monitored, else the first main output. The file has the channels of its format
  (mono, stereo, 3.0, 4.0, 5.0, 5.1, 7.0, 7.1), laid on the speakers of the file in the order of
  libav's channels: 5.0 and 5.1 with the surrounds at the back (5.1 is `FL FR FC LFE BL BR`, as
  WAV and AAC have it), 7.0 and 7.1 with the surrounds at the sides and the rears at the back
  (`FL FR FC LFE BL BR SL SR`), the S of 4.0 at the back centre.
- `rate`: samples a second of the file, 7350 to 192000; by default the timeline's
  (`sample_rate`). Another rate is made by the windowed sinc that makes the device's rate. AAC
  takes only the rates of the table of MPEG-4 audio: 7350, 8000, 11025, 12000, 16000, 22050,
  24000, 32000, 44100, 48000, 64000, 88200 and 96000.
- `codec`: `aac` (in `.mp4`, `.mov`, `.m4a`), `pcm_s24le` or `pcm_f32le` (in `.mov`, `.wav`); by
  default `pcm_s24le` in `.wav` and `aac` else. AAC and PCM of 24 bits are clipped to −1…1, the
  one place where the sound of an export is clipped; PCM of floats is written as the graph gives
  it, over 1 too.
- `bitrate`: of AAC only, 128000, 192000 (the default), 256000 or 320000.
- `normalize`: Audio Normalization of the Deliver page (p. 4055), of this export only, never of
  the project: `{"standard", "target"?, "mode": "normalize" | "optimize"}`. `standard` is an id
  of the table of loudness standards (`ebu-r128`, `spotify`, `apple-podcasts`, …); `target`, in
  LUFS from −60 to 0, is the standard's target by default (the timeline's Target Loudness for a
  standard without one). The bus is measured first over the whole range (BS.1770, its true peak
  by 8 times oversampling), then: `normalize` adds the gain that brings it to whichever of the
  target and the standard's highest true peak it meets first; `optimize` adds the gain that brings
  it to the target and, where its true peak would go over the highest, a limiter of true peaks
  after it (the Limiter effect, its ceiling at the highest true peak, release 100 ms), and
  measures again: the loudness that the limiter took is given back once where it is more than
  0.1 LU, and the ceiling is lowered by what the true peak still goes over, twice at the most.

Without `audio`, a film of a timeline that has no clip of sound that is on has no sound and
needs no bus. Audio Normalization asked for of a film without sound is not done, and `message`
says so.

While Audio Normalization measures, before the first frame is written, `render.progress` carries
`measuring`: `{"pass", "passes", "frame", "frames"}` (pass `pass` of at most `passes`, `frame` of
`frames` samples of it measured); each pass is told as it starts. `fps` counts from the end of
the measuring.

With `"video": null` the sound is exported alone, without `codec` or `quality`, into a `.wav` or
an `.m4a`; the range is still in frames of the timeline, and the progress counts them. A bus,
rate, codec, rate of bits, standard or target that cannot be is refused with `-32602` and
`{"reason": "bus" | "rate" | "audio_codec" | "bitrate" | "standard" | "target"}` (`bus`,
`rate`, `bitrate` or `standard` beside it) before anything is written.

`render.progress` carries, once Audio Normalization has measured, `normalization`:
`{"standard", "mode", "target", "true_peak_max", "gain_db", "ceiling_db" | null,
"before": {"integrated", "true_peak"}, "after": {"integrated", "true_peak"}, "met"}`
(LUFS and dBTP; `after` measured again where a limiter was put in, else `before` raised by the
gain; `null` for silence). Where it could not meet the standard (`met` false: a sound of no
loudness, left as it is, or one that came as near as it could), `message` says so too.

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

## Meters, loudness and the analysis of sound

Plan T9b of phase 3 (p. 3688, p. 3804–3805, p. 3840–3841, p. 3887–3888, p. 3979–3987). None of
these changes the project but `audio.normalize` and `audio.level_match` with `apply`. Levels are
in dBFS, loudness in LUFS and LU, true peaks in dBTP, to a hundredth; silence (−∞), and a measure
that there is none of, is `null`.

### The meters of the timeline as it plays

While the timeline monitor plays, the engine reads the levels of every track and bus before and
after its fader, the sum of the channels of the bus that is monitored (the Control Room meter,
p. 3983) and the loudness of that bus (BS.1770-4: Momentary, Short Term, their maxima, Integrated,
Loudness Range, the true peak by 8 times oversampling; the LFE not counted), in cells of 10 ms,
30 times a second, so that nothing is missed however seldom a client asks. What the meters show
(the bars, the peak indicators, the loudness) goes on across the graphs of the sound that a change
of the project, a seek or a change of speed swaps in, and falls as on silence once nothing has
played for 0.1 s. What is metered is what goes to the device, before it is clipped; the loudness
hears only what plays. The meter type (IEC 60268-18, Digital VU, or Custom with its detector,
scale, decay and peak indicator), the standard and target of the loudness and Lock Metering to
Transport are the timeline's (`audio.meter`, `audio.custom_meter`, `audio.loudness`), read at
each request.

`audio.meters {timeline, reset?}` answers what they show now; a client polls it (the desktop at
the rate it draws, 30 a second at the most). `reset: true` first lets the meters of the tracks and
buses go of their peaks (a click on the meters: the only end of Infinite hold).

- `timeline`; `playing`: whether a graph of it gave sound in the last 0.1 s; `rate`: 30, how often
  the engine reads the meters; `scale`: `iec` (−60 to −20 dB in the lower half of a meter, −20 to
  0 dB in the upper) or `linear` (Quasi Linear).
- `strips`: one for each track that is on and each bus, tracks first in the order of the
  timeline: `{"owner": {"track": "A1"} | {"bus": "B1"}, "pre": [channel, ...], "post": [...]}`, a
  channel being `{"bar", "peak", "bar_position", "peak_position"}`: the bar of the meter type (the
  sample peak falling 20 dB in 1.7 s, the VU, or the RMS of 300 ms) and the peak indicator (`null`
  where it is Off), in dB, and where they stand on the scale, from 0 to 1. `pre` is after the
  strip's processing and before its mute and solo, `post` after its mute, solo, fader and VCAs
  (Pre fader metering on tracks picks which one a track shows, p. 162). A timeline that does not
  play, or another than the one that played last, has none.
- `control_room`: a channel as above, of the sum of the channels of the bus that is monitored;
  `null` where none is.
- `loudness`: as `audio.loudness` answers.
- `lost`: cells written over before they were read since the engine started (0 unless the engine
  was held up for a third of a second).

`audio.loudness {timeline, action?}`: `start` (or resume), `pause` (what plays is not heard, what
was measured is kept), `reset` (nothing measured); without `action`, only what it reads. Only
playing switches the meters to a timeline: for another timeline than the one that played last,
a read is of nothing measured, and an action is `-32001` with `{"reason": "not_metered",
"timeline"}`. With Lock
Metering to Transport on (the default), each start of playing where the playhead was put (a play,
a seek) resets it as it is first heard. It answers `{"running", "standard", "target",
"momentary", "short_term", "momentary_max", "short_term_max", "integrated", "range",
"true_peak", "sample_peak", "seconds", "verdict"}`: `standard` its id (the table of loudness
standards, see The sound of an export), `target` its target or the timeline's Target Loudness,
`integrated` as the standard measures it (BS.1770-1 ungated, the others gated), `verdict` `low`,
`within` or `high` against the target and the standard's tolerance (±1 LU where it gives none),
by the integrated loudness to a tenth, `null` before there is one.

### Analyses

`audio.standards` reads the 15 rows of `columbina_audio::standards::STANDARDS`, in menu order,
without changing the project or starting a job. Each row has `id`, `name`, `target` (LUFS or
`null` for the timeline's Target Loudness), `tolerance` (LU or `null`), `true_peak` (dBTP or
`null`), `measure` (`gated` or `ungated`), `dialogue`, `resolve`, `official`, `verified`, and
`source`. A `null` target is not an official numeric target. Unofficial or unverified rows
keep that provenance; clients must not replace it with an official claim.

`audio.analyze`, `audio.normalize` and `audio.level_match` check their params against the project
as it is, answer `{"job": "audio-<n>"}` at once, and measure on a thread of their own, as fast as
the clips can be read (never through the device). Every connection is told `audio.analysis`:
`{job, state: "running", done, total}` (seconds of sound measured, of all the job measures) at
most 4 times a second, and once how it ended. `audio.cancel {job}` stops one (it ends
`cancelled`); a job that is not running is `-32602` with `{"reason": "missing", "job"}`. At most
4 run at once; a fifth is `-32001` with `{"reason": "busy"}`. A clip that cannot be read ends the
job `failed`, the reason in `message`.

What is measured is what an export would carry (a Listen never):

- a track or a bus: after its fader, mute, solo and VCAs, over `range` (frames of the timeline,
  `to` not included) or the whole timeline;
- a clip: its sound after its Volume (keys and all) and before its pan, as its track takes it (its
  channels configured, at its speed, through its equalizer and its effects, which come before its
  Volume), without its fades and crossfades, and without the track's Trim, polarity, Ducker,
  inserts, Dynamics, equalizer and lanes, over the part of it on the timeline (`range` is not
  read for it). So the gain that Normalize and Level Matcher find is what the Volume is moved
  by, and a clip normalized again is moved by nothing (a gain under 0.001 dB is not applied).

`audio.analyze {timeline, targets, range?, standard?, revision?}`: `targets` from 1 to 256, each
`{"clip"}`, `{"track"}` (a track of sound that is on) or `{"bus"}`, each once (the tracks and buses are measured
in one run of the mix, each clip in a run of its own); `standard` an id of the table,
the timeline's by default. Done: `{job, state: "done", method: "audio.analyze", standard, level,
results: [{target, integrated, range, momentary_max, short_term_max, true_peak, sample_peak,
seconds, verdict}, ...]}` in the order of `targets`, `level` the target of the standard. Of the
bus that is exported, the integrated loudness and the true peak are what `ebur128` of ffmpeg reads
of the exported file (within 0.1 LU and 0.2 dB).
When `revision` is supplied, it is checked under the snapshot lock before a job is started;
a mismatch returns `-32001`, `data.reason: "stale_revision"`, with `expected` and `actual`,
without a job or history change. Omitting it retains the existing unbound behavior.

`audio.normalize {timeline, clips, level?, target?, mode?, apply?, revision?}` (Normalize Audio
Levels, p. 3840–3841): `clips` from 1 to 256 clips of sound, each once; `level` `sample_peak` (the
default), `true_peak`, or the id of a standard (its integrated loudness, as the standard measures
it); `target` from −60 to 0, dBFS for a peak (−9 by default) and LUFS for a loudness (the
standard's target by default); `mode` `relative` (the default: the clips as one programme, heard
one after another in the order of the timeline, the highest peak among them for a peak, one gain
for all) or `independent` (each clip by its own level). Done: `{job, state: "done", method:
"audio.normalize", level, target, mode, measured, clips: [{clip, gain_db, clamped, measured},
...], applied}`: `measured` the level of the clips together (`relative`; `null` for
`independent`) and of each clip; `gain_db` what is added to the clip's Volume (the target less the level measured after it),
`null` for a clip that has no level (silence, or shorter than 400 ms for a loudness), which is left as it is;
`clamped` where the Volume, or a key of it, moved by the gain would go out of −96…24 dB, so that
the clip does not come to the target.

`audio.level_match {timeline, reference, clips, apply?, revision?}` (Level Matcher,
p. 3804–3805): the gain that brings each clip to the loudness of `reference`, the difference of
their integrated loudness, each its ungated mean where it has none (shorter than 400 ms, or under
the gates). Done: `{job, state: "done", method: "audio.level_match", reference: {clip, measured},
clips: [{clip, gain_db, clamped, measured}, ...], applied}`.

With `apply: true`, once they are found, the gains are applied in one transaction made by the
client that asked, one step to undo: a `nudge_clip_gain` of each clip by its gain (`linked:
false`; the whole curve of a clip with keys moved, clamped to −96…24 dB), none where no clip has
a gain. `applied` is what `transaction.apply` answers, `null` where nothing was applied.
`revision`, where it is given with `apply: true`, is checked before the snapshot is taken and
again when applying the result. An initial mismatch refuses the request without a job; a change
during measurement is reported in `refused` when the job finishes. It binds the commit as it
binds `transaction.apply`: where the project changed
since, nothing is applied and `refused` holds the refusal (`stale_revision`); without it, the
gains are applied to the project as it is when they are found. A transaction refused otherwise (a
clip removed meanwhile) is told in `refused` too.

The classic agent tool `video_loudness` currently exposes `measure` for clip/track/bus/main and
`apply` for `clip_gain` Normalize only (phase 3 T24a). It waits for the terminal event and reports
`status: measured | applied | unchanged | refused`, preserving the engine's nulls, clamping,
transaction receipt and refusal. Its `measured_revision` identifies the checked snapshot;
`session` identifies the Python connection generation. Main resolves a bus with no outputs;
multiple main outputs require an explicit bus id. Range is accepted only for track/bus/main,
since clip analysis currently measures the whole clip. T24 is not complete: track/main apply,
candidate analysis, clip-range analysis, `video_duck`, audio info and shared range/neutral-value
validation remain separate increments. No main limiter or ducking guarantee is made by T24a.

Params that cannot be are `-32602` with `data.reason`: `missing` (`timeline`; or `what`: `clip`,
`track` or `bus`, and `id`), `disabled` (`track`), `standard` (`standard`), `twice` (`clip`, or `target`),
`out_of_range` (`what`: `targets`, `clips`, `range` or `target`).
