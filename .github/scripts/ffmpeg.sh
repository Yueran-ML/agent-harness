#!/usr/bin/env bash
# FFmpeg 8.1 for the jobs of the engine in .github/workflows/ci.yml, set up for the steps that
# follow through GITHUB_ENV and GITHUB_PATH. ffmpeg-next 8.1 is for FFmpeg 8 (libavcodec 62),
# and the FFmpeg of Ubuntu is older and that of Homebrew newer, so neither is taken.
#
# Linux and Windows: two builds of BtbN (github.com/BtbN/FFmpeg-Builds), each checked against
# the checksums of the release it is taken from.
#   - The LGPL shared build, which the engine is built with and linked against: FFMPEG_DIR, and
#     its libraries where the loader looks (LD_LIBRARY_PATH; on Windows PATH, for the DLLs).
#     Its ffmpeg makes the proxies of an engine built with it.
#   - The static GPL build, whose ffmpeg has libx264 and libx265 and makes the media of the
#     tests. It comes first on PATH, so that `ffmpeg` and `ffprobe` are its own; it has no
#     libraries that could be loaded in place of those of the LGPL build.
# macOS: ffmpeg@8 of Homebrew, which pkg-config finds for the build and whose ffmpeg makes both
# the media of the tests and the proxies.
#
# With FFMPEG_SH_PATH=none (the packaging of the desktop, Linux and Windows), neither build is put on
# PATH or LD_LIBRARY_PATH: FFMPEG_DIR names the LGPL build, and COLUMBINA_TEST_FFMPEG the ffmpeg of
# the GPL build, for what makes the media of a test, and for nothing else.
set -euo pipefail

base="${COLUMBINA_BTBN_URL:-https://github.com/BtbN/FFmpeg-Builds/releases/download/latest}"
root="$RUNNER_TEMP/ffmpeg"

# A path as the system has it: on Windows D:\a\_temp\ffmpeg, not /d/a/_temp/ffmpeg.
native() {
  if [ "$RUNNER_OS" = Windows ]; then cygpath -w "$1"; else printf '%s\n' "$1"; fi
}

# Unpack ffmpeg-n8.1-latest-<platform>-<flavor>-8.1 of the release into $root/<flavor>.
fetch() {
  local flavor=$1 name file attempt
  name="ffmpeg-n8.1-latest-$platform-$flavor-8.1"
  file="$name.$archive"
  # The release is made anew each day: checksums and an asset read while it is replaced may
  # not agree, and are read again a little later.
  for attempt in 1 2 3; do
    if curl -fsSL --retry 5 --retry-all-errors -o "$root/checksums.sha256" "$base/checksums.sha256" \
      && curl -fsSL --retry 5 --retry-all-errors -o "$root/$file" "$base/$file" \
      && (
        cd "$root"
        awk -v file="$file" '{ name = $2; sub(/^\*/, "", name) } name == file' \
          checksums.sha256 > "$file.sha256"
        test -s "$file.sha256" && sha256sum -c "$file.sha256"
      ); then
      break
    fi
    if [ "$attempt" = 3 ]; then
      echo "::error::$file of BtbN could not be downloaded, or its sha256 is not that of checksums.sha256"
      exit 1
    fi
    sleep 60
  done
  case "$archive" in
    zip)
      if command -v unzip > /dev/null; then
        unzip -q "$root/$file" -d "$root"
      else
        7z x -y -bd "-o$root" "$root/$file" > /dev/null
      fi
      ;;
    tar.xz) tar -xJf "$root/$file" -C "$root" ;;
  esac
  rm -rf "${root:?}/$flavor"
  mv "$root/$name" "$root/$flavor"
  rm -f "$root/$file" "$root/$file.sha256"
}

mkdir -p "$root"
case "$RUNNER_OS" in
  Linux | Windows)
    if [ "$RUNNER_OS" = Windows ]; then
      platform=win64 archive=zip exe=.exe
    else
      platform=linux64 archive=tar.xz exe=
    fi
    fetch lgpl-shared
    fetch gpl
    include="$root/lgpl-shared/include"
    making="$root/gpl/bin/ffmpeg$exe"
    echo "FFMPEG_DIR=$(native "$root/lgpl-shared")" >> "$GITHUB_ENV"
    if [ "${FFMPEG_SH_PATH:-}" = none ]; then
      echo "COLUMBINA_TEST_FFMPEG=$(native "$making")" >> "$GITHUB_ENV"
    elif [ "$RUNNER_OS" = Linux ]; then
      echo "LD_LIBRARY_PATH=$root/lgpl-shared/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" >> "$GITHUB_ENV"
    else
      native "$root/lgpl-shared/bin" >> "$GITHUB_PATH"
    fi
    # Each line of GITHUB_PATH goes before those written earlier.
    if [ "${FFMPEG_SH_PATH:-}" != none ]; then
      native "$root/gpl/bin" >> "$GITHUB_PATH"
    fi
    ;;
  macOS)
    # pkgconf is there already on the images of GitHub; asked for so that it is sure to be.
    HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1 \
      brew install --quiet ffmpeg@8 pkgconf
    prefix="$(brew --prefix ffmpeg@8)"
    include="$prefix/include"
    making="$prefix/bin/ffmpeg"
    # ffmpeg@8 is keg-only: neither pkg-config nor PATH finds it by itself, and it is to be
    # found before an ffmpeg of Homebrew that may be there too.
    search="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    found="$(PKG_CONFIG_PATH="$search" pkg-config --modversion libavcodec)"
    if [ "${found%%.*}" != 62 ]; then
      echo "::error::pkg-config finds libavcodec $found, and ffmpeg-next 8.1 is for 62 (FFmpeg 8)"
      exit 1
    fi
    echo "PKG_CONFIG_PATH=$search" >> "$GITHUB_ENV"
    echo "$prefix/bin" >> "$GITHUB_PATH"
    # The engine never takes the ffmpeg of its proxies from PATH, and FFMPEG_DIR, where it looks
    # otherwise, is not set on macOS, since ffmpeg-sys-next is to use pkg-config there.
    echo "COLUMBINA_PROXY_FFMPEG=$prefix/bin/ffmpeg" >> "$GITHUB_ENV"
    ;;
  *)
    echo "::error::no FFmpeg for $RUNNER_OS"
    exit 1
    ;;
esac

major="$(awk '$2 == "LIBAVCODEC_VERSION_MAJOR" { print $3 }' "$include/libavcodec/version_major.h")"
if [ "$major" != 62 ]; then
  echo "::error::libavcodec of $include is $major, and ffmpeg-next 8.1 is for 62 (FFmpeg 8)"
  exit 1
fi
version="$("$making" -hide_banner -version)"
echo "${version%%$'\n'*}, libavcodec $major"
encoders="$("$making" -hide_banner -encoders)"
for encoder in libx264 libx265 prores_ks aac; do
  if ! grep -q " $encoder " <<< "$encoders"; then
    echo "::error::$making has no $encoder, which the media of the tests are made with"
    exit 1
  fi
done
