# The engine protocol (version 1)

`columbina-engine` is the only program that writes a project. The desktop app and the agent
connect to the same running engine, send it changes, and are told of every change in one order.
This is what a client is written against.

## Starting and finding an engine

```
columbina-engine --project <DIR> [--headless] [--idle-seconds <N>]
```

- The directory is made if it is not there. The engine takes the lock of the project, opens
  `project.json` (a new, empty project if there is none), and listens.
- Its first line on **stdout** is one JSON object, written once it listens:
  `{"listening": "<address>", "pid": 1234, "protocol": 1}`. Nothing else is ever written to
  stdout. Logs go to **stderr**, one JSON object a line (`{"event": "kicked", ...}`).
- **Address.** Windows: the named pipe `\\.\pipe\columbina-engine-<hash>`, where `<hash>` is the
  first 16 lowercase hex digits of the SHA-256 of the UTF-8 of the project directory as a full
  path (`std::fs::canonicalize`, written without `\\?\`). Unix: the socket
  `<project>/cache/engine.sock`. The address is opened as it is given: Node
  `net.connect(address)`, Python `asyncio.open_unix_connection(address)` or, on Windows, the
  proactor loop's `create_pipe_connection` (wrap it in a timeout: it retries a busy pipe for ever).
- **Ending.** A `--headless` engine ignores stdin and ends once no connection has been open for
  `--idle-seconds` (30 unless given), counted from its start and from each time the last
  connection closes; a connection made in that time keeps it. An engine that is not headless
  belongs to whoever started it (the desktop) and ends when its **stdin is closed** (the owner
  closes it, or dies); it does not end for lack of connections. Both end on Ctrl+C, and on
  SIGTERM on Unix. As it ends, the engine stops listening, saves the project, and removes
  `engine.lock`.
- **Exit codes.**

  | code | meaning | stderr |
  |---|---|---|
  | 0 | ended as asked (idle, stdin closed, Ctrl+C) | |
  | 1 | stopped by an error after it listened (taking connections failed 20 times in a row, or the last save failed) | `{"event": ...}` lines |
  | 2 | could not start (bad arguments, a `project.json` that cannot be read, the address taken by another program) | `{"error": "..."}` |
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
- Requests carry `"jsonrpc": "2.0"`, a `method`, an `id` (string, number or `null`) and, where
  the method takes them, `params` (an object). A message without an `id` is a notification; the
  engine defines none from clients and ignores them. Batches (arrays) are refused (`-32600`).
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

**A client that does not read** fills its queue (4096 messages); the engine then closes its
connection rather than drop a single message (a line on stderr says so). A client whose
connection was closed reconnects, says hello and calls `project.get`.

## Methods

Before `hello`, every other request is refused with `-32600`.

| method | params | result |
|---|---|---|
| `hello` | `{"client": "desktop" \| "agent" \| "cli", "protocol": 1}` | `{"engine": "0.1.0", "protocol": 1, "revision": n, "project": "<dir>", "pid": 1234}` |
| `project.get` | none | `{"revision": n, "next_id": k, "project": {...}}` |
| `transaction.apply` | `{"ops": [op, ...]}` | `{"revision": n, "done": [op, ...], "next_id": k, "names": {"$a": "c7", ...}}` |
| `history.undo` | none | `{"revision": n, "done": [op, ...], "next_id": k}`, or `{"revision": n, "done": null}` if there is nothing to undo |
| `history.redo` | none | as `history.undo` |

- `hello` says once per connection what the client is; a second `hello` is refused (`-32600`).
  From then on the connection is sent every notification. `protocol` other than 1 is refused
  (`-32602`, `data: {"protocol": 1}`). `project` is the directory as the engine writes it (see
  Address); `pid` is the engine's.
- "None" params: leave them out, or send `{}`, `[]` or `null`; anything else is `-32602`.
- `transaction.apply` applies its operations in order, all or none; an empty list is refused.
  `done` is the operations as they were done (see below). `names` maps each `$name` of the
  transaction (see below) to the id the engine gave it; `{}` if there were none.
- `history.undo` takes back the latest step **whoever made it** (desktop and agent share one
  line of undo); `history.redo` does again the step undone last, with the same ids. A new
  transaction clears what could be redone. 1000 steps are kept.

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
`next_id`, never reused; tracks `V<n>`/`A<n>` by place). The rules are in
`crates/model/src/ops.rs`.

**`done`** lists the operations as they were applied, each with every id the engine gave and the
place each thing was put at (the `index` of an added thing is filled in): applied in order to a copy of the
project at revision `n - 1` whose `next_id` was first set to the `next_id` sent with them, they
give the project at revision `n`. Without that `next_id`, a copy would refuse the engine's new
ids as `reserved`.

## Names of a transaction (`$name`)

Within one `transaction.apply`, a thing that an operation adds may be given an id of the form
`$name`: `$` and 1–32 of `A–Z a–z 0–9 _`. The thing is added as if its id were empty, and given
one by the engine. Every **later** operation of the **same** transaction that names `$name` where
it names a thing (`timeline`, `track`, `id`, `media`, and `media` of a clip) is made to name that
id before it is applied. The ids are answered in `names`; `done` and `project.changed` carry only
real ids.

```json
{"ops": [
  {"op": "add_media", "media": {"id": "$m", ...}},
  {"op": "add_timeline", "timeline": {"id": "$t", "name": "Cut", "rate": {"num": 25, "den": 1}, "tracks": []}},
  {"op": "add_track", "timeline": "$t", "track": {"id": "$v", "kind": "video", "clips": []}},
  {"op": "insert_clip", "timeline": "$t", "track": "$v", "clip": {"id": "$c", "media": "$m", ...}}
]}
```
→ `"names": {"$c": "c3", "$m": "m1", "$t": "t2", "$v": "V1"}`.

- Names may be given to media, timelines, tracks, and clips (also those inside an added timeline
  or track). A name lives only for its transaction.
- An id that starts with `$` is always a name: one of another form is refused (`invalid_name`),
  one named before an operation gives it is refused (`unknown_name`), and one given twice is
  refused (`duplicate_name`). These are `-32602` with `data: {"index": i, "name": "$x",
  "reason": "..."}`, and nothing is applied.

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
| `-32600` | not a request object, `jsonrpc` is not `"2.0"`, no `method`, a bad `id`, a request before `hello`, a second `hello`, a line over 16 MiB | |
| `-32601` | no such method | |
| `-32602` | params are not what the method takes; an operation that cannot be read; a bad `$name` | `{"index": i, "reason": "invalid_op"}` for an operation that cannot be read; see names above |
| `-32001` | a transaction, undo or redo was refused; the project is exactly as it was | the refusal, below |

The connection stays usable after every error.

**Refusals (`-32001`).** `data` is `{"reason": "...", "message": "...", ...fields}`, with
`"index": i` (the place of the refused operation in `ops`) when an operation was refused, and
`{"reason": "empty", "message": ...}` for a transaction with no operations. An undo or a redo is
refused only through a bug of the engine. Each `reason` and its fields:

| reason | fields | meaning |
|---|---|---|
| `missing` | `what` (`media`, `timeline`, `track`, `clip`), `id` | the thing named is not there |
| `taken` | `what`, `id` | the id of an added thing is held by another of its kind |
| `reserved` | `id` | an id of the engine's own form (`m`/`t`/`c` + number) not made yet |
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
