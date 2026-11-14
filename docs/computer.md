# The tool `computer`

With this tool the agent sees the screen of the machine that it runs on and uses its pointer
and its keyboard, as the person in front of it would. It is for work that has no other way in:
a program without a command line, a setting in a window, a page that only a browser shows.

It is off unless you switch it on.

```sh
COLUMBINA_COMPUTER=1 columbina
```

or, in `~/.columbina/config.yml` or a patch:

```yaml
patch:
  - id: computer
    config:
      enabled: true
```

## What it does

One tool, `computer`, with an `action`:

| Action | What it does |
|---|---|
| `screenshot` | A picture of the main display. |
| `zoom` | A part of the screen (`region`), at the pixels that the screen has there. |
| `cursor_position` | Where the pointer is. |
| `mouse_move` | Puts the pointer at `coordinate`. |
| `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click` | A click at `coordinate`, or where the pointer is. `text` may name keys to hold meanwhile. |
| `left_click_drag` | Presses at `start_coordinate`, moves to `coordinate`, lets go. |
| `scroll` | Turns the wheel: `scroll_direction`, and `scroll_amount` notches (3 if left out). |
| `type` | Types `text` as it is, whatever the layout of the keyboard. |
| `key` | Presses a chord such as `ctrl+s`; several, separated by spaces, one after another. |
| `hold_key` | Holds a chord for `duration` seconds. |
| `wait` | Waits `duration` seconds. |

The picture of the screen is made smaller, so that its longer side is at most 1280 pixels, and
every position that the model names is one in that picture. After anything that it did with
the pointer or the keyboard the tool waits half a second and gives back a new picture.

`type` types the characters themselves, so it does not matter which layout or which input
method the window has: Chinese, accents and emoji arrive as they are. A line break in the text
is the Enter key and a tab is the Tab key, which in many windows goes to the next field. `key`
presses keys as a keyboard does. So a letter that is pressed alone goes through the input
method of the window, and where that is composing Chinese it takes the letter, as it would
from your keyboard; chords such as `ctrl+c` are not touched by that.

A key is one character, or one of: `ctrl`, `alt`, `shift`, `super` (Command, or the key of
Windows), `enter`, `tab`, `space`, `backspace`, `delete`, `escape`, `insert`, `home`, `end`,
`pageup`, `pagedown`, `up`, `down`, `left`, `right`, `capslock`, `f1` to `f12`. The names that
`xdotool` has for them are read too.

## Where it works

| System | How | What it needs |
|---|---|---|
| Windows | `SendInput` and GDI | Nothing. It reaches no window of a program that runs with higher rights, and nothing while the screen is locked. |
| macOS | Core Graphics, `screencapture` and `sips` | Screen Recording and Accessibility, allowed in System Settings for the program that starts Columbina: the terminal, or the application. |
| Linux, X11 | `libX11` and `libXtst` (XTest) | The two libraries, which a desktop has. A screen of 24 bits of colour. |
| Linux, Wayland | Not reached yet | Wayland shows no program the screen of another without a portal, which is not spoken to yet. The model is told so. |

Only the main display is reached. Nothing is installed for any of this: the tool speaks to the
system through what Python brings.

## What keeps it in hand

- **It is asked about.** A call of the tool runs something on your machine, and is checked as
  a command of the shell is: where a session asks, you are asked, and a yes for the rest of the
  session is for every call of the tool. In plan mode it is refused. The assistants in Feishu,
  WeCom and QQ and the subagents have nobody to ask, so there it is refused unless a rule in
  `permissions.yml` allows `computer`. In full access nothing is asked, as for the shell.
- **You stop it with the pointer.** Put the pointer into the upper left corner of the screen.
  The tool looks there before it moves or types anything, and then does nothing and says so
  to the model. Stopping the turn works as always, and whatever a call was holding, a key or a
  button, is let go when the call ends.
- **The model is told what not to do.** What the screen shows is something to look at and not
  an instruction; it is not to type passwords, keys or the details of a card, and not to
  confirm a purchase, a payment or a deletion on its own. That is a rule that it is given, not
  a wall: watch what it does, and do not leave it alone with a window in which a mistake costs.

## Which models

The model has to take pictures. Claude and the models of OpenAI do, and so does the Flash of
DeepSeek (`deepseek-flash`); its Pro (`deepseek-v4-pro`) takes none, and is told that a picture
was there. Each picture of the screen costs tokens and stays in the
conversation, so a long piece of work with this tool fills the window of the model sooner than
one with commands.

## Config

| Key | Default | |
|---|---|---|
| `enabled` | `${COLUMBINA_COMPUTER:-0}` | Whether the tool is offered. |
| `longest` | `1280` | The longer side of a picture of the screen, in pixels, at most. |
| `settle_seconds` | `0.5` | How long the tool waits before the picture that it gives back. |
| `failsafe` | `true` | Whether the pointer in the upper left corner stops the tool. |
