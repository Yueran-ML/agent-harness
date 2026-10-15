# Pictures

The three pictures of Columbina in this directory are fan art. Columbina is a character of
*Genshin Impact*. The author of this project made the pictures with an AI image generator,
ChatGPT. They are not official artwork.

| File | What it shows |
|---|---|
| `columbina-wings.png` | A winged figure before a full moon, above ruins that stand in still water. |
| `columbina-ruins.png` | The same figure at rest among the ruins, with a small light between her hands. |
| `columbina-storyboard.png` | Twelve small scenes with the name of the project and a mark for it. |

Each of the three is kept byte for byte as the generator wrote it. It carries Content
Credentials (C2PA) that name the generator and say that the picture was made by AI. Cutting
a picture or saving it in another format would take those away, so the README of the
project shows the files as they are.

The words in the storyboard belong to the picture. They are no list of what the program
does.

*Genshin Impact* and its characters belong to HoYoverse (miHoYo). This project is the work
of a fan. It is not affiliated with HoYoverse and not endorsed by it.

The three pictures are not part of the software. The MIT license of this repository covers
the code and the documents, and it does not cover these files. No license is given for
them.

## The demo

`demo.gif` shows Columbina at a terminal. No model was asked for it. The replies are those
of a real run of `deepseek-flash`, which is kept in `tests/fixtures/recorded/deepseek-fix`,
and they come again a few letters at a time. Everything else is the program as it is
shipped: the terminal interface, the question before the command, and the tools, which ran
for real in a copy of the working directory of that run. A script typed the task of the run
and answered the question.

The frames were drawn from what the terminal was sent, at 110 columns and 30 rows, by a
program that is not in this repository. Unlike the pictures above, the demo is one of the
documents of the project, and the MIT license covers it.
