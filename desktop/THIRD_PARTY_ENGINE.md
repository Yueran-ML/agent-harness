# Third-party software in the engine of the studio

<!--
For maintainers (this comment is not shipped: electron/engine-bundle.mjs drops it).
Do not tag desktop-v* for public release until FFmpeg is built from pinned sources with complete
notices (see plans): the BtbN build that is bundled now is the latest of its day, and the
libraries that it links into the DLLs have no notices of their own here yet.
-->

The engine of the studio, `columbina-engine.exe`, is part of Columbina and is under its MIT
license. The package brings it in `resources/engine/`, together with software of others that it
needs, each under its own license. This file is copied there as well, followed by what only the
build can tell: the exact build of FFmpeg, the DirectX Shader Compiler of Microsoft that it compiles
its shaders with, with its terms and version, the versions of OpenColorIO and of the libraries
linked with it, and the Rust crates in the engine.

## FFmpeg

`avcodec-62.dll`, `avformat-62.dll`, `avutil-60.dll`, `swresample-6.dll`, `swscale-9.dll`,
`avfilter-11.dll`, `avdevice-62.dll` and `ffmpeg.exe` are FFmpeg (https://ffmpeg.org),
copyright (c) 2000-2026 the FFmpeg developers. FFmpeg is a trademark of Fabrice Bellard.

- **Which build.** The `win64-lgpl-shared` build of FFmpeg 8.1 made by BtbN
  (https://github.com/BtbN/FFmpeg-Builds). It is configured with `--enable-version3` and with
  neither `--enable-gpl` nor `--enable-nonfree`. The packaging checks that its `ffmpeg
  -buildconf` says so.
- **License.** These files are licensed under the GNU Lesser General Public License, version 3 or
  (at your option) any later version. Its text is `licenses/FFmpeg/LICENSE.txt`, and the GNU
  General Public License version 3, on which it builds, is `licenses/FFmpeg/GPL-3.0.txt`. FFmpeg
  comes with no warranty.
- **How the engine uses them.** The engine is linked dynamically against the five libraries
  `avcodec`, `avformat`, `avutil`, `swresample` and `swscale`. Windows loads them by name from this
  folder when the engine starts. `ffmpeg.exe` is a separate program that the engine runs to make
  proxies of media; it also loads `avfilter` and `avdevice`.
- **Replacing them.** You may replace these files with your own build of FFmpeg 8.1, modified or
  not, as long as it keeps the same names and interface (libavcodec 62, libavformat 62, libavutil
  60, libswresample 6, libswscale 9). The engine uses whatever libraries lie in this folder. The
  `ffmpeg` that makes proxies may also be named by its full path with the variable
  `COLUMBINA_PROXY_FFMPEG`.
- **Source.** The exact revision of FFmpeg is given below under *This build of FFmpeg*. Its source
  is in the FFmpeg repository (https://git.ffmpeg.org/ffmpeg.git, mirrored at
  https://github.com/FFmpeg/FFmpeg). The scripts that configured and built it are at
  https://github.com/BtbN/FFmpeg-Builds. Those scripts also fetch the sources of the libraries
  that the build links into the DLLs, one for each `--enable-lib…` option of its configuration.
  These libraries are under their own licenses.

## OpenColorIO

The engine manages colour with OpenColorIO (https://opencolorio.org), copyright Contributors to
the OpenColorIO Project. It is linked statically into `columbina-engine.exe`, together with the
libraries that OpenColorIO is built with: yaml-cpp, expat, Imath, pystring, minizip-ng and zlib.
None of them is a file of its own in this folder.

- **Which build.** OpenColorIO is built as a static library from the source of its release,
  checked against its SHA-256, by `engine/crates/ocio/ci/build-ocio.ps1` of Columbina's
  repository. Its own CMake fetches the other libraries at the versions that it recommends. The
  versions, and the source of each, are listed below under *OpenColorIO and the libraries linked
  with it*. The packaging checks that the OpenColorIO that the engine is built with is of that
  version.
- **License.** OpenColorIO is under the BSD 3-Clause License, whose text is
  `licenses/OpenColorIO/LICENSE`. It includes code of other projects, whose notices are
  `licenses/OpenColorIO/THIRD-PARTY.md` (among them SampleICC and xxHash, which are compiled into
  the library; it lists some that only its tools and plugins use, which are not built here) and
  `licenses/OpenColorIO/FileFormatCDL-NOTICE.txt`. yaml-cpp and expat are under the MIT license
  (yaml-cpp includes dragonbox, `licenses/yaml-cpp/dragonbox-NOTICE.txt`), Imath and pystring under
  the BSD 3-Clause License, and minizip-ng and zlib under the zlib license. The text of each is in
  `licenses/<name>/`. They come with no warranty.
- **The ACES configs.** The colour spaces, displays and views of the studio are those of the
  configs for the Academy Color Encoding System (ACES) that are built into OpenColorIO
  (`ocio://studio-config-latest`, the ACES Studio config). They are part of its source
  (`src/OpenColorIO/builtinconfigs`), under its BSD 3-Clause License, and are made by the
  OpenColorIO-Config-ACES project
  (https://github.com/AcademySoftwareFoundation/OpenColorIO-Config-ACES).
- **Source.** https://github.com/AcademySoftwareFoundation/OpenColorIO, and the repository of each
  library at the tag listed below.

## Rust crates

The engine is compiled from crates of crates.io. Most of them are under the MIT license or the
Apache License 2.0, or both. The list below is generated from `engine/Cargo.lock` by
`desktop/scripts/engine-notices.mjs` when the package is made. It covers the crates that the
engine is compiled from on this system. It leaves out those used only for building or testing it,
and the procedural macros, which run in the compiler and are not part of the program.
The license files of each crate (its copyright notice and license text) are copied unchanged into
`licenses/crates/<name>-<version>/`.
