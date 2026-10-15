# Artwork and audio notices

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
