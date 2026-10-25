# Pets

The desktop application can show a pet: a small figure in a window of its own, over the other
windows, that follows the task that is open. It thinks while the model thinks, reads while a tool
runs, waits with you while the agent asks for a yes, waves when a turn is done, and shows a
failure when one went wrong. A click on it brings Columbina back, a drag moves it, and its menu
hides it. The settings show it, choose it, and tell other programs what the task is doing.

Three pets come with the application: Little Moon, Little Cat and Little Star. They are pixel art
that `desktop/scripts/draw-pets.mjs` draws from shapes, so that they can be drawn again.

Anybody can make a pet in two ways. A **package** is a pet that Columbina shows itself. The **state
file** lets a program of your own, such as a desktop pet that you already have, follow the task.

## A package

A package is a folder with `pet.json` and a sheet of frames. Put it in the folder for your own
pets, which the settings open: `%APPDATA%\Columbina\pets\` on Windows,
`~/Library/Application Support/Columbina/pets/` on macOS and `~/.config/Columbina/pets/` on Linux.
It can be chosen in the settings then.

```text
pets/
  mochi/
    pet.json
    spritesheet.png
```

`pet.json`:

```json
{
  "id": "mochi",
  "displayName": "Mochi",
  "description": "A round friend.",
  "spritesheetPath": "spritesheet.png",
  "animations": {
    "idle": { "row": 0, "frames": 6, "fps": 5 },
    "wave": { "row": 3, "frames": 4, "fps": 6 }
  }
}
```

| Field | |
|---|---|
| `spritesheetPath` | Required. The sheet, a PNG or WebP of 16 MB at most, in the folder of the package. |
| `id` | The name of the pet. Without it, the name of the folder. |
| `displayName` | What the settings call it, 64 characters at most. Without it, `id`. |
| `description` | Shown where the pointer rests on its name, 200 characters at most. |
| `animations` | The rows of the sheet, each with `row`, `frames` (1 to 8) and `fps` (1 to 30). Optional. |

The sheet is laid out as the pets of Codex are, so that a pet made for Codex, or found on
Petdex, can be used as it is. Its cells are 192 by 208 pixels, eight to a row, so the sheet is
1536 pixels wide. It has nine rows at least, and may have more, which are left alone:

| Row | Name | Shown |
|---|---|---|
| 0 | `idle` | at rest |
| 1 | `runRight` | not yet |
| 2 | `runLeft` | not yet |
| 3 | `wave` | for a while after a turn ended well |
| 4 | `jump` | not yet |
| 5 | `failed` | for a while after a turn failed, was blocked or was cut off |
| 6 | `waiting` | while the agent waits for your yes |
| 7 | `thinking` | while the model thinks or writes |
| 8 | `review` | while a tool runs |

A row of `animations` takes the place of the row of the table. A row that `animations` does not
name has as many frames as there are cells with something in them before its first empty cell,
and goes at a speed of its own. Frames are drawn at three quarters of their size, so a sheet
that is drawn four times larger than its art stays sharp.

A package that does not hold together is not offered: a `pet.json` that is no object or larger
than 64 KB, a sheet outside the folder, of another size, or that is no picture, or `animations`
that name a row the sheet does not have.

## The state file

Turn on **Tell other pet programs what the task is doing** in the settings, and Columbina writes
what the task that is open is doing to `pet-state.json` in the same folder as `pets`, whenever
that changes:

```json
{ "version": 1, "state": "waiting", "label": "shell", "at": 1792917913946, "pid": 47860 }
```

| Field | |
|---|---|
| `version` | 1. A reader takes no other version. |
| `state` | `idle`, `thinking`, `review`, `waiting`, `failed` or `done`. |
| `label` | The tool that runs or waits for a yes, or the code of a failure. Often empty. |
| `at` | When it was written, in milliseconds since 1970. |
| `pid` | The process of the application. |

`done` and `failed` are written when a turn ends, and `idle` 15 seconds later. Nothing of the
conversation is written, and no file content. The file is written beside its place and moved
there, so a reader never finds half of it. When the application ends it writes `idle`, and when
the option is turned off the file is deleted.

The file is written when the state changes, and not otherwise: a model may think for minutes,
and the file says `thinking` all that while. A reader should take the task as at rest where the
process of `pid` is no longer there, as after a crash. Watch the file, and look at it every few
seconds as well: a watch can miss a change.
