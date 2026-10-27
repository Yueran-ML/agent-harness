# Artwork, audio and typeface notices

Columbina is an **unofficial fan project**, with no affiliation with or endorsement
by HoYoverse or miHoYo. Genshin Impact, Columbina and Nod-Krai references belong to
their respective rights holders. The visual assets below are **AI-generated**.
They are **outside this repository's MIT license**; the code license does not grant
rights to these images, video, character designs, or any music.

## Visual sources

| Build filename | Source and treatment |
| --- | --- |
| `columbina-intro.mp4` | User-supplied Jimeng AI opening video, dated 2026-10-09. Original source only; kept outside the repository and never copied into the application. |
| `columbina-intro-extended.mp4` | Derived from that opening video. Final three seconds slowed to half speed, original audio removed. `AIGC`, `UserComment` and `LvMetaInfo` provenance tags are preserved and checked by the conversion script. |
| `moonlit-columbina.png` | User-supplied AI fan illustration of Columbina, used as the opening poster. |
| `columbina-icon.png`, `electron/icon.ico`, `electron/icon.icns` | AI-generated silver dove, created using OpenAI's image generation tool. Prompt and conversion details are recorded in `electron/icon-design.md`. |

The three runtime visual files, `columbina-intro-extended.mp4`, `moonlit-columbina.png`
and `columbina-icon.png`, are kept in `desktop/assets/` since 2026-10-15, so that every
build, on any machine and in CI, shows the same opening. They are still outside the
MIT license of the code. No audio binary is committed, and neither is the original
video. The build copies the three files into ignored `public/` and derives the icons
of Windows and macOS from `columbina-icon.png`. This notice is also copied to
`dist/ASSET-NOTICES.txt` in the application package. These notices describe provenance
and license boundaries; they are not a grant of permission from any third-party rights
holder.

## Artwork setup

`npm run dev` and `npm run build` copy the three files from `desktop/assets/`. To try other
artwork, point `COLUMBINA_ASSET_DIR` at an absolute directory that holds files of the same
names, or create a local, ignored `desktop/assets.local.json`:

```json
{ "assetDirectory": "F:\\path\\to\\columbina-local-assets" }
```

Run `npm run assets:extend` when regenerating the silent, extended video from the
original; FFmpeg and FFprobe must be on PATH, and the original must lie in such a
directory. The script writes the result there, to be copied into `desktop/assets/`, and it
stops if provenance tags are missing or changed. The original
video is archived outside the repository and is not shipped alongside its derivative.

## Local background music

No music comes with this project. Settings can import an audio file that the user
has, and the application also looks for one optional file in the application data of
the device. Either stays on that device: it is excluded from Git, and building a
package does not copy it. A recording that a user plays this way belongs to its
rights holders, and nothing here grants a right to it. Users may mute, replace or
remove their local music.

## Typefaces

The interface is set in two typefaces, which the build takes from their Fontsource
packages and ships inside the application. Both are under the SIL Open Font License,
Version 1.1, whose full text follows the table. This notice is shipped with the
application as `ASSET-NOTICES.txt`, beside the fonts.

| Typeface | Package | Copyright |
| --- | --- | --- |
| Figtree, for the words of the interface | `@fontsource-variable/figtree` | Copyright 2022 The Figtree Project Authors (https://github.com/erikdkennedy/figtree) |
| JetBrains Mono, for code and paths | `@fontsource-variable/jetbrains-mono` | Copyright 2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono) |

Chinese and Japanese are drawn in the faces of the system, which are not shipped.

```text
-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```
