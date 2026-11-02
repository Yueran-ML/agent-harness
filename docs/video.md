# Editing video

Columbina edits video in two places that share one project: the editing studio of the desktop
application, where a person cuts by hand, and the agent's video tools (and `columbina video` at
the command line). Both go through `columbina-engine`, which owns the project, the timeline
rules, the undo history, playback and export. A change made in one place shows at once in the
other, and either can undo the last step of the other. Python prepares word analysis and
editable stories; it does not build an ffmpeg filter graph or render a film of its own.

## Requirements

- Windows. The engine, and with it the editing studio, the video tools and `columbina video`,
  runs on Windows only for now; macOS support will come later. On macOS and Linux the studio's
  entry in the desktop application is greyed out and says why, the tools are not offered, and
  every `columbina video` command says so; the rest of the agent works there as usual.
- The desktop package brings its engine. For the agent and the CLI outside the desktop, set
  `COLUMBINA_ENGINE` to the absolute path of an installed engine, or put `columbina-engine` on
  PATH. It must be outside the working directory. The video plugin's `engine` setting also
  accepts this path. See [protocol 2](../engine/PROTOCOL.md). To build from source, run
  `cargo build --release -p columbina-server` in `engine/` with the FFmpeg development libraries
  configured.
- The engine needs its shared LGPL FFmpeg libraries. Export uses the hardware encoders of the
  graphics card (NVENC, Quick Sync or AMF); without one it writes ProRes. System **ffmpeg and
  ffprobe** are used for audio extraction and silence analysis, not for the film.
- For local speech recognition, install faster-whisper separately and set
  `COLUMBINA_WHISPER_MODEL` to a downloaded model directory. No model downloads automatically.
  `transcribe --words FILE` instead reads a JSON array of `start_us`, `end_us`, `text` objects.
- Set `COLUMBINA_VIDEO=1` to enable the agent's tools. Speech analysis stays on this machine.
  The sound extracted for the transcriber is deleted once the words are kept (or the
  transcription fails); `analysis/` keeps only the words and silences of each source.

## The editing studio

*Studio* in the side bar of the desktop application opens the studio of the project folder. Its
project is `.columbina/studio` in that folder, the same one the agent uses by default. The
studio follows DaVinci Resolve: its pages, keys, mouse gestures and the names in its interface
are those of Resolve, checked against the DaVinci Resolve 20 Reference Manual. Four pages, Media,
Cut, Edit and Color, are switched at the top, or with the left and right arrow keys on the page
tabs. The Color page has a section of its own below.

**Media.** The media pool, with the master bin and bins inside bins. *Import* adds files to the
chosen bin; files stay where they are, and the pool only names them. Clips are shown as a list
or as thumbnails, and *Media details* shows the codec, size, rotation, frame rate, bit depth,
duration, sample rate, channels, path and the In and Out points of the clip chosen. F2 renames,
Delete removes from the pool (refused while a timeline uses the clip), and media is dragged into
another bin. A double-click loads a clip into the source viewer. The engine makes a proxy of
media that is slow to seek in, as phones and cameras record it (above 1080p, more than 8 bits,
HEVC or AV1, B-frames); the viewer says whether it shows the proxy, the original or both, and
*Use proxies* turns them off. Export always reads the originals.

**Edit.** The page of the timeline: the media pool and the effects library on the left, the
source viewer and the timeline viewer side by side, the inspector on the right, and the timeline
below. Where the window is narrow (the conversation open beside it), one viewer is shown with a
switch between them, and each keeps its place. The viewer that was clicked last has the keys of
playback and of In and Out points. The bar above chooses the timeline, makes a new one (name,
size and frame rate, by default those of the first video in the pool) and opens the inspector.

- **Tools.** Selection (A), Trim (T) and Blade (B), snapping (N) and linked selection
  (Ctrl+Shift+L). With the selection tool a clip is dragged to move it, and its edge to trim it;
  the two-pixel seam between two clips rolls the cut. With the trim tool an edge ripples, the
  upper half of a clip slips and the lower half slides it. The blade cuts the track clicked, or
  every track with Shift.
- **Dragging.** A clip dragged onto others overwrites them, and the frames it will cover are
  hatched in red while it moves. Shift keeps its time and only changes its track; Ctrl+Shift
  shuffles it in (the others make way); Alt leaves a copy where it is dropped. A clip moved to
  another track takes its linked clips as many tracks along their own kind, as Resolve's Linked
  Move Across Tracks does: a picture moved from V1 to V2 takes its sound from A1 to A2. No track
  is added for it: where there is none, nothing moves and the studio says why. Media dragged from
  the pool overwrites; with Ctrl+Shift it is inserted, with Shift only its sound is placed and
  with Alt only its picture. N during a drag turns snapping on or off for that drag alone.
- **Three-point edits.** F9 to F12 and Shift+F10, Shift+F12 put what the source viewer shows on
  the timeline by DaVinci Resolve's rules of three-point editing: the source In and Out (its
  first and last frame where none is set), the timeline In and Out, or the playhead, with
  backtiming where only an Out is set. The track headers choose the destination tracks of
  picture and sound, and lock, mute or hide a track.
- **Inspector.** For a clip on a video track: zoom, position, rotation, opacity and fades; on an
  audio track: volume, pan and fades; for a title: text, size, colour, position and alignment;
  for a solid colour: its colour. A number is typed, or dragged sideways (with Shift ten times
  as fast), and is sent once when it is let go; Escape takes it back and a double-click on its
  label resets it. With several clips chosen only what they share is shown. A transition shows
  its kind, length and alignment; where the media has too few frames for it, the inspector
  offers the longest length that fits. With nothing chosen it shows the timeline's settings;
  the frame rate is locked once the timeline holds anything.
- **Speed.** R (or the button in the inspector) changes the speed of the clips chosen, from 1% to
  10000%, forward or reversed, with or without rippling what follows. Sound changes pitch with
  speed in this version.
- **Effects library.** Dissolve, dip to black or white and the audio crossfade are dropped on an
  edit point, centred on it, or ending or starting there by where they fall; on a transition
  already there, they change its kind. A title or a solid colour dropped on a video track
  overwrites five seconds there.

Every gesture or key is one transaction and one step of undo (Ctrl+Z, Ctrl+Shift+Z), shared
with the agent. Where the engine refuses an edit (a locked track, a clip in the way of a ripple,
too few frames for a transition, a project that changed meanwhile), the timeline says why in a
sentence and nothing changes.

**Cut.** The page for cutting quickly. One viewer shows the source clip, the source tape or the
timeline. The source tape lays every clip of the current bin, and of the bins inside it, end to
end, to be scrubbed through as one tape; the up and down arrows go to the clip before or after.
Below the viewer are the buttons *Smart insert* (at the edit point nearest the playhead, which
the timeline marks), *Append*, *Ripple overwrite*, *Place on top* (to an edit point within five
frames of the playhead) and *Source overwrite*, with *Dissolve* and *Dip to black* at the edit
point nearest the playhead. Source overwrite lines clips up by their timecode; as the engine does
not read timecode yet, it is disabled and says so, as Resolve does nothing where no timecode
overlaps. Two timelines lie below: the whole film above, the part around the playhead below,
both editable. As on Resolve's Cut page, Track 1 is magnetic: moving or deleting a clip there
closes its gap, a clip let go on an edit point is inserted there, and its edges ripple, while
clips on the tracks above only move or resize. Clips of other tracks that lie across the gap,
such as a bed of music or a clip moved straight up off Track 1 with its sound, stay where they
are, and what follows the gap moves back on every track.

**Subtitles.** A right-click on a track header adds a subtitle track, and the *+* above the
headers adds one in a chosen language. A right-click on a subtitle track, or a double-click in an
empty place of it, adds a subtitle of three seconds at the playhead. A subtitle is dragged to
move it and by its ends to change them, and its text, start and length are edited in the
inspector. Backspace removes it and leaves the gap; Delete removes it and moves the later
subtitles of the track up. As in Resolve, a timeline shows one subtitle track at a time: the eye
of a subtitle track shows it in place of the one shown. Subtitles are drawn in the timeline
viewer and are never burnt into the picture.

**Export.** *Quick export* on the Edit and Cut pages opens the export: the timeline, the whole
of it or from its In to its Out, the codec (H.264, HEVC or ProRes), the quality, and the subtitle
tracks to save as SRT beside the film (each track has a box, the shown one checked). *Export to…*
asks where to write. A bar under the studio's header shows the frames done, the speed and the
encoder, cancels, and opens the folder when it is done; editing and playback go on meanwhile, as
the export reads the project as it was when it started. One export runs at a time, the agent's
included. An existing film is replaced only after the save dialog has asked; SRT files or a
`.mov` already beside it are not replaced, and the export says so.

**Old projects.** When the studio opens a folder on Windows, films that the agent cut before the
studio existed are made timelines of the studio in the background (see Old projects below). A
banner says which, and what could not be carried over; the old files are left as they were.

### Keys of the studio

The keys are DaVinci Resolve's defaults. "RM" is the page of the DaVinci Resolve 20 Reference
Manual where Resolve describes the key; "—" where the manual gives none and the key follows
Resolve's other pages or common use.

| Key | What it does | RM |
|---|---|---|
| Space | Plays or stops the viewer that has the keys | — |
| J / K / L | Plays backward, stops, plays forward; J or L again plays faster (up to 8×); with K held, a frame back or forward | — |
| ← / → | A frame back or forward; with Shift, a second | — |
| Home / End | First or last frame | — |
| ↑ / ↓ | The edit point before or after the playhead; in the source tape of the Cut page, the clip before or after | 722 |
| I / O, Alt+X | In point, Out point, clear both (on media in the source viewer, on the timeline in the timeline viewer) | 721, 819 |
| A / T / B | Selection, Trim, Blade tool | 796, 924 |
| N | Snapping; during a drag, for that drag only | 711, 745 |
| Ctrl+Shift+L | Linked selection | 745 |
| Ctrl+Alt+L | Links the clips chosen, or unlinks them | 771, 1097 |
| F9 / F10 / F11 / F12 | Insert, overwrite, replace, place on top | 745, 820 |
| Shift+F10 / Shift+F12 | Ripple overwrite (the range between timeline In and Out, where set), append | 561, 745, 842 |
| F9 (Cut page) | Smart insert; F10 stays overwrite there | 559, 565 |
| Backspace | Lifts the clips chosen and leaves a gap; with timeline In and Out, empties that range | 765 |
| Delete, Shift+Backspace, Shift+Delete | Ripple delete; with timeline In and Out, deletes that range and closes it | 765, 796 |
| Ctrl+B, Ctrl+\ | Cuts at the playhead | 744, 804 |
| Ctrl+T | Standard transition (one second) at the edit points of the clips chosen, or at the playhead; on the Cut page a dissolve at the nearest edit point | 1020 |
| D | Turns the clips chosen off or on | 796 |
| R | Changes the speed of the clips chosen | — |
| Alt+Shift+1–8, 9, F1–F8 | Locks or unlocks V1–V8, all video tracks, A1–A8 | 762 |
| Ctrl+= / Ctrl+− | Zooms in or out around the playhead | 711 |
| Shift+Z | Fits the whole timeline; again, back to the zoom before | 711 |
| Alt+wheel / Ctrl+wheel / Shift+wheel | Zooms, scrolls along the timeline, changes track heights | 712 |
| Ctrl+Z / Ctrl+Shift+Z | Undo, redo (shared with the agent) | 745 |
| Esc | Cancels a drag | — |
| F2, Delete, Ctrl+A (media pool) | Rename, remove from the pool, choose all | — |

Keys are not taken while a field is being written in, while an input method composes, or while a
dialog is open. Ctrl+M, which marks in Resolve, is left for markers; the export has its button.

## The Color page

The Color page grades the timeline as DaVinci Resolve's Color page does, and is laid out as it
is: the viewer, with the Gallery, the LUT browser or the media pool on its left and the node
editor on its right; below them the thumbnails of the clips and the mini timeline; and at the
foot the palettes, with the video scopes at the right. The viewer shows the timeline's frame at
the playhead; the up and down arrows go to the clip before or after. Where the window is narrow,
the left and centre palettes share one row of buttons, as Resolve's do below 1920×1080. A
palette in which the current node is adjusted shows an orange dot on its button.

**Grades and nodes.** Each clip has its own grade, a tree of nodes; a new clip has one
corrector, 01. A group of clips has a pre-clip and a post-clip grade, and the timeline a grade
over every clip, applied in that order; the node editor switches between Clip, Timeline, Group
Pre-Clip and Group Post-Clip. Nodes are added about the current one with Alt+S (serial), Shift+S
(serial before), Alt+P (parallel), Alt+L (layer), Alt+K (append at the end) and Alt+O (an
Outside node, which takes the current node's picture and its key inverted); a parallel or layer
node brings its mixer. Links are dragged from an output to an input, the picture's in
green and the key's in blue, and a loose node dropped on a link goes into it; Ctrl+drag swaps
what two correctors do. A node is labelled, locked (Lock Node), turned off with Ctrl+D or a
click on its number (all of them with Alt+D), extracted with E and deleted with Delete;
Shift+Home, Ctrl+Shift+Home and Ctrl+Home reset the node, every node, or the whole grade. The
right-click menu of a thumbnail makes groups and collapses them; = and - apply the grade of the
clip one or two before, and Ctrl+C and Ctrl+V copy and paste a whole grade where the thumbnails
have the focus. Every change is one step of undo, shared with the agent. A setting with
keyframes (which the agent sets) shows and changes the key at the playhead or the one to its
left.

**Primaries and Curves.** Primaries has Resolve's three modes: Color Wheels (Lift, Gamma, Gain
and Offset, with Contrast, Pivot, Saturation, Hue, Lum Mix, Temp, Tint, Color Boost and Midtone
Detail), Color Bars, and Log Wheels (Alt+Z switches between the wheels and Log). A wheel is
dragged; with Shift it jumps to where it is clicked, with Ctrl the master ring moves; a
double-click resets it. The black and white point pickers take their colours from the viewer.
Curves has the custom curves, ganged or not, the six HSL curves (Hue vs Hue, Hue vs Sat, Hue vs
Lum, Lum vs Sat, Sat vs Sat, Sat vs Lum), whose points are also taken from the viewer, and Soft
Clip.

**Qualifier and Key.** The Qualifier palette keys a colour in HSL, RGB or LUM mode, with Matte
Finesse; its pickers add or subtract what is clicked or dragged in the viewer. Shift+H shows the
key of the current node as Resolve's Highlight does (the rest grey), and Alt+Shift+H in black
and white. The Key palette sets a node's key input, output and qualifier (Invert, Matte or Mask,
Gain, Offset, and the blur of the input), and the inputs and output of a key mixer; keys go from
node to node along the blue links.

**Power Windows.** The Window palette adds Linear, Circular, Polygon, Curve and Gradient windows
to the current node, each with its Size, Aspect, Pan, Tilt, Rotate, Opacity and softness, and
they are drawn and moved with their controls on the viewer: a Circular or Gradient window by its
centre, size, softness and angle; a Linear or Polygon window by its corners, edges and points (a
click on an edge adds a point, a middle-click removes one); a Curve window point by point, with
Bézier handles dragged out of its points and closed by a click on its first point. Shift and a
drag around the points of a Curve chooses several, to move, scale, rotate or delete together. A
drag on a playing timeline pauses it. Windows are saved as presets (Save as New Preset, Update
Preset, Delete Preset) in a library that every project of the user shares; a preset recalled on
a node replaces its windows, as Resolve's does, and is kept in the project in full.

**Tracker.** The Tracker palette makes the windows of a node follow the picture: the cloud
tracker follows the whole of what the window covers, and the point tracker the points that are
placed. It tracks Pan, Tilt, Zoom, Rotate and 3D, forward, backward or a frame at a time, in
Clip or Frame mode; a track is cleared, interpolated, copied and pasted. The stabilizer comes in
a later phase.

**Blur.** The Blur palette has Resolve's three modes: Blur, Sharpen and Mist. Radius, H/V Ratio
and Scaling are set for red, green and blue, ganged at first; Sharpen adds Coring Softness and
Level, and Mist its Mix. Over Radius, a notch of the wheel changes it by 0.01. Blur works within
the node's key, so it blurs or sharpens what a qualifier or a window keys.

**LUTs.** The LUT browser lists the `.cube` files of the project's folder and of the LUT library
(each marked 1D or 3D), with Favorites. A LUT is dragged onto a node, or applied from its
right-click menu or from a node's LUT submenu. A LUT that a node names and that cannot be found
is said over the viewer, at its lower right. Project Settings set an Input, an Output and a
Color Viewer LUT for the whole project. *Generate LUT* in the right-click menu of a thumbnail
writes the grade of that clip as a 17, 33 or 65 point cube; a node with a qualifier, a window,
Midtone Detail or a blur that changes the picture cannot be baked and is left out, and the page
says how many were.

**The Gallery and comparisons.** Ctrl+Alt+G grabs a still of the frame shown, with its grade,
into the current album. The project's albums are its own; the PowerGrade albums are shared by
every project of the user. A still's grade is applied to the clips chosen, or its nodes appended
(Append Node Graph), with keyframes as *Apply Grades Using* says. Alt+1 to Alt+8 keep the grade
of the current clip as the memories A to H, and Ctrl+1 to Ctrl+8 apply them. Stills deleted are
brought back with Ctrl+Z in the Gallery. Ctrl+W wipes the frame against the still chosen (a
double-click on a still does too), in one of eight styles, its line dragged on the viewer; Alt+W
inverts it, and Ctrl+Alt+B and Ctrl+Alt+N go to the still before or after. *Wipe Timeline Clip*
on a thumbnail wipes against another clip. Ctrl+Alt+W shows a split screen of the selected
clips, the selected stills, the clips about the current one or the current group, up to 16
pictures. Comparisons are of the viewer only; an export never shows them.

**Video scopes.** Waveform, Parade, Vectorscope, Histogram and CIE Chromaticity, docked at the
right of the palettes or in a floating panel (Ctrl+Shift+W), one, two, four or nine at a time,
each with its own settings (scale in 10-bit, percent or nits, channels, brightness, reference
levels). They measure the frame as the timeline viewer shows it, before the Color Viewer LUT.

**The viewer.** Alt+F enlarges the viewer, Shift+F shows it full screen and P as cinema (Esc
leaves it). Ctrl+= and Ctrl+- zoom, Shift+Z fits and Alt+Shift+Z shows it at 100%; the wheel and
the middle button zoom and pan. Shift+D turns Bypass Color on and off: the picture without its
grades, as on the Edit and Cut pages.

**Shot Match and Auto Color.** *Shot Match to This Clip* in the right-click menu of a thumbnail
makes the clips chosen look like the one clicked, each by the node chosen in it; as in Resolve,
the match is an adjustment that no palette shows, and the thumbnail and the node are marked as
matched. *Reset Shot Match* takes it away. The A button of the Primaries palette (Alt+Shift+C)
balances the blacks and whites of the current node into its Lift and Gain, where they can be
seen. Shot Match is made for clips not graded yet, in Rec.709 Gamma 2.4 (pp. 3047–3048).

### Colour management

*Project Settings › Color Management* sets the colour of the project as Resolve's do, and its
changes take effect when *Save* is pressed, as one step of undo. **Color science** is DaVinci
YRGB by default: nothing is converted, as in a new project of Resolve. DaVinci YRGB Color
Managed converts each clip from its input colour space into the timeline colour space, where it
is graded, and from there into the output colour space, by a preset (SDR Rec.709, HDR Rec.2020
and the others) or by spaces chosen one by one, with the working luminance, the Input and Output
DRT and Limit Output Gamut. ACEScct and ACEScc grade in ACES, with ACES 1.3 or 2.0, an output
device transform and the reference gamut compression. The transforms are those of OpenColorIO
and its built-in ACES configs; the engine carries them, and nothing else needs to be installed.
Every space may be chosen wherever a space is: Rec.709, Rec.2020, P3 and the other display
spaces with each gamma, Rec.2100 HLG and ST2084 (PQ), and the spaces of cameras and of grading,
such as Apple Log, DJI D-Log, Sony S-Log3, ARRI LogC, Canon Log, Panasonic V-Log, Blackmagic
Film Gen 5 and DaVinci Intermediate. The right-click menu of media in the media pool sets their
Input Color Space, Bypass and Data Levels; a phone's HLG or PQ video is known by what its file
says. The 3D LUTs of the project are read between their points by Trilinear interpolation
(Resolve's default) or Tetrahedral.

**HDR.** Where the output colour space is HDR, the viewers show HDR on a display for which
Windows has HDR on; *Display HDR On Viewers If Available*, in the viewer's options, turns that
off, and is kept on this computer. An export to an HDR output space is written in 10 bits (HEVC
Main 10 or ProRes; H.264 is refused) and tagged with its colour space, the mastering display and
the light of its content (MaxCLL and MaxFALL), measured over every frame it writes. A project
that is not managed exports Rec.709 as before.

### Keys of the Color page

These are the Color page's own keys; playback, In and Out and undo are those of the table above.

| Key | What it does | RM |
|---|---|---|
| ↑ / ↓ | The clip before or after | 2991 |
| Alt+S / Shift+S | Adds a serial node after or before the current one | 3318 |
| Alt+P / Alt+L / Alt+K | Adds a parallel node, a layer node, a node at the end (Append) | 3318 |
| Alt+O | Adds an Outside node of the current one (its key inverted) | 3318, 3341 |
| Ctrl+D / Alt+D | Turns the nodes chosen, or all nodes, off or on | 3315 |
| E, Delete | Extracts the node chosen; deletes the nodes chosen | 3318–3319 |
| Shift+Home / Ctrl+Shift+Home / Ctrl+Home | Resets the current node, every node, the whole grade | 3316 |
| Alt+Shift+O | The grade as it was when its clip was chosen (Original Memory) | 3317 |
| Alt+Shift+; / Alt+Shift+' | The node before or after | 3314 |
| Ctrl+C / Ctrl+V | Copies and pastes a node in the node editor, a whole grade in the thumbnails | 3286, 3318 |
| = / - | Applies the grade of the clip one or two before | 3288 |
| Alt+Z | Primaries: between the wheels and Log | 3068 |
| Alt+Shift+C | Auto Color | 3045 |
| Shift+H / Alt+Shift+H | Highlight, Highlight in black and white | 3170–3171 |
| Shift+D | Bypass Color | 2987 |
| Ctrl+Alt+G | Grabs a still | 3257 |
| Ctrl+Alt+B / Ctrl+Alt+N | The still before or after | 3260 |
| Alt+1–8 / Ctrl+1–8 | Saves the memory A–H, applies it | 3271–3272, 3289 |
| Ctrl+W / Alt+W | Image wipe on or off, inverts the wipe | 3000–3002 |
| Ctrl+Alt+← / → | The clip of a timeline wipe a frame back or on | 3002 |
| Ctrl+Alt+W | Split screen on or off | 3003 |
| Ctrl+Shift+W | The floating video scopes on or off | 3007–3008 |
| Alt+F / Shift+F / P | Enhanced viewer, full screen viewer, cinema viewer | 2968–2969 |
| Ctrl+= / Ctrl+- / Shift+Z / Alt+Shift+Z | Zooms the viewer in or out, fits it, shows it at 100% | — |
| H | The node editor's selection tool or hand | 3311 |

## The agent's tools

The editing tools are `video_import`, `video_info`, `video_edit`, `video_transcribe`,
`video_story`, `video_apply`, `video_correct`, `video_render` and `video_undo`; the grading
tools, `video_grade_info`, `video_grade_set`, `video_grade_node`, `video_grade_copy`,
`video_grade_lut`, `video_grade_group`, `video_grade_scopes`, `video_color_settings`,
`video_media_color` and `video_shot_match`, are described under Grades below. By default they
connect to `.columbina/studio` under the session workspace, the studio's project; an explicit
`project` selects another directory. With more than one timeline in the project, a tool that
works on a timeline needs its id.

- `video_import` adds media to the pool, optionally into an existing bin, and returns the media
  ids. It places nothing on a timeline.
- `video_info` lists media, bins and timelines with their tracks, clips, source In and Out,
  placement, duration, speed and transitions; a graded clip or timeline with its grade in short,
  and the groups of grades.
- `video_edit` applies a batch of edits as **one atomic transaction and one undo step**. Its
  receipt has the operations done, the ids given to `$names` and the inputs that were clamped.
- `video_transcribe` transcribes media into `analysis/`, and can replace a subtitle track with
  cues from the words (one undo step).
- `video_story`, `video_apply` and `video_correct` edit by text (below).
- `video_render` exports, with progress while it waits (below).
- `video_undo` undoes the last step of the project, the person's in the studio included;
  `redo=true` does it again. Analysis and story files are outside this history.

The edit commands of `video_edit` are DaVinci Resolve's: `insert`, `overwrite`, `replace`,
`place_on_top`, `append`, `smart_insert`, `ripple_overwrite`, `lift`, `ripple_delete`,
`ripple_delete_range`, `close_gap` (closing empty space, not Resolve's Close Up, which reframes a
shot), `blade`, `move`, `insert_move`, `trim`, `roll`, `slip`, `slide`, `set_speed`,
`add_transition`, `set_transition`, `remove_transition`, `add_track`, `remove_track`, `add_cue`,
`set_cue`, `remove_cue` and `cues_from`. A batch may mix them with the basic operations of the
protocol, such as `add_timeline`, `add_bin`, `set_clip` (transform, gain, pan, fades),
`set_track`, `set_media` and `set_timeline`. The engine decides track locks, links and available
handles. Timeline positions are integer frames; source times use ticks and their scale. `null`
means an explicitly cleared field or disabled destination, while an omitted field keeps its
documented default. The tool's description lists the fields of each command. `move` with
`to_track` takes the linked clips as many tracks along their own kind (Linked Move Across
Tracks), and `close_gap` with `straddling="stay"` leaves the clips of other tracks across the
gap where they are, as the Cut page closes up Track 1.

When a range is removed, subtitles in that range are trimmed or deleted and following cues move
with the picture. Inserting through a cue splits it into two cues with the same text; the latter
moves with its original picture. Locked tracks stay locked.

A timeline shows one subtitle track at a time, as DaVinci Resolve does. A subtitle track added
with `add_track` is hidden while another is shown. To show another track, hide the shown one in
the same batch first (`set_track` with `enabled:false`, then `enabled:true` on the other);
otherwise the engine refuses with `VIDEO_SUBTITLE_SHOWN`.

### Grades

The grading tools work on the grades of the Color page as DaVinci Resolve builds them: a tree of
nodes for each clip, a pre-clip and a post-clip grade for each group of clips, and one grade over
the whole timeline, applied in that order. A tool names a grade by a clip id, or by
`{"level": "pre_clip" | "post_clip", "group": "k2"}` or `{"level": "timeline"}`, and a node by
its number as the Color page shows it (1 is the first added) or by its id. Each change is one
transaction and one undo step, shared with the person in the studio, and its receipt has the
grade's summary.

- `video_grade_info` summarises a grade: its nodes in order with number, id, kind, label and
  whether each is on; what each input is linked from; only the corrector settings that change
  the picture, by their paths (`primaries.saturation`) in DaVinci's units; LUTs, and those that
  are missing; mixers and their inputs; and whether Shot Match adjusted a node. Without a grade,
  it lists every graded clip, the timeline's grade and the groups. With `reference: true` it
  gives the full reference instead: every setting of a node with DaVinci's name, its range and
  neutral value, keyframes, mixers, the colour settings and the colour spaces. The tools'
  descriptions keep only the essentials, so that they cost little on every request.
- `video_grade_set` patches one node's settings, label or `enabled`. Values are checked before
  they are sent, so a value out of range is told with DaVinci's name, range and neutral value
  (Saturation: 0 to 100, 50 changes nothing). A number may be keyframes, `{"keys": [{"frame": 0,
  "value": 40, "interp": "linear"}, ...]}`, with `hold` for static keyframes and `bezier` for
  eases. A key's frame is, for a clip of media, the frame of its media (at the media's own rate,
  from 0) whose picture is shown, so keys stay on their pictures through trims, slips and speed
  changes; for a title, a colour or a still, the clip's frame from its first; for the timeline's
  grade, the timeline's frame. The settings are those of the Primaries, Curves, Qualifier, Blur
  and Key palettes, Soft Clip and a node's LUT; Power Windows and their tracking are drawn on
  the Color page.
- `video_grade_node` adds a corrector as `append` (Append Node), `add_serial`,
  `add_serial_before`, `add_parallel`, `add_layer` or `add_outside` (Add Outside Node, whose key
  is the current node's inverted) add one, with its label and settings in the same step if
  given, or removes, resets or extracts a node.
- `video_grade_copy` copies a grade onto others (with `preserve_nodes`, each keeps its first N
  nodes), appends its nodes to theirs (Append Node Graph), or pastes one node's settings or one
  setting. `keyframes` says where copied keys go: `source_timecode` (the default, the same time
  of the media) or `start_frame` (as far from each clip's first picture).
- `video_grade_lut` lists the `.cube` LUTs of the project folder and the LUT library
  (`COLUMBINA_LUTS`, otherwise the `luts` folder in `COLUMBINA_HOME` or in `~/.columbina`) with
  their kind (1D, 3D, shaper and 3D), and sets or clears a corrector's LUT by name; without a
  node, the LUT goes on a new corrector at the end. `export` is Generate LUT: it writes a grade,
  or one of its correctors, as a 17, 33 or 65 point `.cube` at `path`, and names the nodes it
  had to leave out.
- `video_grade_group` makes a group of clips, adds or removes clips, renames or deletes it,
  copies a grade into its pre-clip or post-clip grade, or collapses its grades into each clip's
  own. A group's grades are edited further with the other tools and `pre_clip`/`post_clip`.

- `video_grade_scopes` measures a frame drawn off screen, as an export draws it, so it needs no
  monitor and works with the agent's own headless engine: for R', G', B' and luma their least,
  most and mean and the levels below which 1, 5, 25, 50, 75, 95 and 99 percent of the pixels
  are, the chroma (its centroid, a cast where it is off the centre) and the mid grey. `clip`
  measures the middle of a clip; with `node`, the picture going into the node or coming out of
  it (`at`) is measured instead of the frame; `histogram` adds a coarse histogram.
- `video_color_settings` reads the project's colour management (Project Settings › Color
  Management: DaVinci YRGB, DaVinci YRGB Color Managed, ACEScct, ACEScc) with the spaces it puts
  in force (the timeline space a preset chooses, for instance), or sets a patch of it as one undo
  step.
- `video_media_color` lists the colour of media (what the file says, and what is set by hand),
  or sets the Input Color Space of media, `bypass`, or `default` (the project's), and their Data
  Levels, as the media pool does, for many media in one undo step.
- `video_shot_match` is Shot Match to This Clip: it makes clips look like a target clip, or a
  still of the Gallery (`{"still": "s1"}`). As in DaVinci Resolve the result is invisible: it is
  the `shot_match` of a node, its very last adjustment after Soft Clip, "similar to a LUT"
  (p. 3048), and no setting of the palettes changes; `video_grade_info` shows the node as
  matched. The node is `node` of each clip, its last corrector otherwise, or a corrector added
  to a grade that has none. Each clip and the target are measured off screen, the clip's node
  without any match it already had; per channel a gamma, a gain and an offset, and a contrast
  about the target's mid grey, are fitted by least squares to the percentiles, the mean chroma
  and the mid grey, within bounds narrower than the engine allows, so that a clip far from its
  target is brought closer without harm, and a parameter held at a bound is told. The clips,
  however many, are one undo step. `reset: true` takes a match away; `auto: true` is the A
  button instead, balancing each clip's blacks and whites into its visible Lift and Gain.
  Shot Match is made for clips that are not graded yet, normalized to Rec.709 Gamma 2.4
  (pp. 3047–3048); the receipt says so where the timeline works in another space or a clip was
  graded already.

A LUT is missing when there is no such `.cube` file where the engine looks for it. One that is
there but does not parse is listed with why, and is not set.

## Editing by text

`video_story` writes `story/<timeline id>.md` for the dialogue track (A1 by default).
Read that file and strike words or whole lines with `~~`, or remove them; `video_apply` turns
those changes into ripple deletions on unlocked tracks, in one transaction: picture, sound,
titles and unlocked subtitles follow the cuts. Word timing respects trimmed, repeated,
accelerated and reversed clips. Lines cannot be moved or words invented.
`[pause 1.8s]` can shorten a pause. `video_apply` with `dry=true` previews the removed ranges.
A mark without letters or digits, such as `—`, `♪` or `...`, goes with the nearest word of its
line (the word before it, or the one after it at the start of a line); a line of such marks
alone is cut only when it is removed, emptied or struck through. An unchanged story cuts nothing.

If the timeline or transcript has changed, refresh the story before cutting again: a story is
checked against the digest in its first line, which covers the timeline, the track and the
words. The revision in that line is informational; the cut itself is applied only at the
revision and engine session read for it, so an edit made meanwhile, or a restarted engine,
refuses it with `VIDEO_STALE_REVISION`. `video_apply` before any `video_story` says to write
the story first. `video_correct` changes displayed words only; it does not replace speech or
automatically rewrite existing subtitle cues. Transcribe with a subtitle track id to regenerate
those cues: the track is replaced by the cues of every transcribed media on the dialogue track,
in timeline order, each ending no later than the next begins.

## CLI

Every editing command prints a JSON receipt. Use the actual ids from these receipts:

```bash
columbina video import .columbina/studio raw/interview.mp4
columbina video info .columbina/studio
columbina video edit .columbina/studio --file edits.json
columbina video transcribe .columbina/studio m1 --subtitles TRACK_ID --language en
columbina video story .columbina/studio
# Edit the returned story path, then:
columbina video apply .columbina/studio --dry
columbina video apply .columbina/studio
columbina video render .columbina/studio --codec prores --output out/cut.mov
columbina video undo .columbina/studio
columbina video undo .columbina/studio --redo
```

For `edits.json`, the example below creates a timeline and places `m1` at its start. Substitute
the imported media id. The receipt maps `$film`, `$v`, `$a`, `$s` to engine ids; use the id of
`$s` as `TRACK_ID`. Add `--timeline ID` when the project contains more than one timeline.

```json
[
  {"op":"add_timeline","timeline":{
    "id":"$film","name":"Cut","width":1920,"height":1080,
    "rate":{"num":25,"den":1},"sample_rate":48000,
    "tracks":[
      {"id":"$v","name":"V1","kind":"video","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$a","name":"A1","kind":"audio","enabled":true,"locked":false,"clips":[],"transitions":[]},
      {"id":"$s","name":"S1","kind":"subtitle","enabled":true,"locked":false,"language":"en","cues":[]}
    ]
  }},
  {"edit":"insert","timeline":"$film","source":{"kind":"media","media":"m1"},"at":0}
]
```

`edit --edits JSON` accepts the same array directly. `story --track ID` chooses dialogue;
`apply --fresh` rewrites the story. `correct DIR MEDIA SENTENCE WRONG RIGHT` corrects one word,
with sentence indices starting at zero. The old `overlay` and `render --final` commands are
retired: use engine edits and `render --quality high|medium|small` instead.

## Export

Export uses original media, not playback proxies. `--codec` is `h264`, `hevc` or `prores`;
ProRes uses MOV. H.264/HEVC hardware unavailability falls back to ProRes MOV, and the receipt
names the actual file. Subtitles are UTF-8 SRT files beside the film, **not burnt in**; titles
are drawn into the picture. As in DaVinci Resolve's Deliver page, the subtitle tracks to write
are chosen, shown or hidden: `--subtitles visible` (the default) writes the shown track,
`--subtitles all` every subtitle track, and `--subtitles S1,S2` the tracks named (the
`video_render` tool takes `subtitles: "visible" | "all" | [ids]`, `[]` for none). One track is
written as `<film>.srt`, several as `<film>.<language or track name>.srt`. `--no-subtitles`
omits SRT, and `--replace` explicitly permits replacement of existing outputs. A render observes
a fixed project snapshot; later editing can continue without changing that export. One export
runs at a time, whether the studio or the agent started it.

## Old projects

```bash
columbina video migrate --workspace /path/to/workspace --json
```

Migration finds unconverted schema-1 film directories up to three levels down, skipping Git,
node_modules and `.columbina`. Each becomes one Studio timeline and one undo step; sources
already in the media pool are reused. Existing files stay unchanged, with only a `migrated.json`
receipt added in each old directory. Repeating migration skips completed projects. The first
agent connection to the default Studio also runs this migration, and so does the desktop when
its studio opens a folder (on Windows), once while the application runs, or again after a
failure.

Each old project is migrated on its own. One that cannot be (unreadable files, sources that
moved, changed or lie outside the session's permissions, missing transcripts, an engine
refusal) is reported as `{"source", "error"}` while the others are migrated, and is tried again
next time; the command then exits with 1. An agent tool never fails because of migration: it
gives migration at most 20 seconds once a session, reports what happened in `migrations`, and
leaves the rest to `columbina video migrate`. When an old source has no cached silences and
finding them takes longer than 8 seconds there, its cuts fall at the word boundaries and the
receipt says so. Relative source paths are looked up beside the old project, then in the
workspace. If a migration is interrupted after the engine accepted it, the next run finds the
new timeline from a note kept in `analysis/migrations/` instead of creating it twice. If the
timeline named in `migrated.json` is no longer in the project (for example after undoing the
migration), the next run migrates the project again and says so in `warnings`; to stop that,
delete the old directory or keep its timeline.

The receipt lists omissions and shortened transitions. Automatic music ducking and sound notes
are not migrated; fades, gain, speed, picture placement, titles and subtitle timing are mapped.
If old source files have changed, migration refuses to apply old cuts to different content.
An old audio-only film becomes a 25 fps Studio timeline with its old millisecond positions
converted to frames. Old story edits that were not committed to timeline.json are not applied.

Source reads and output writes obey the agent session's file permissions, including media
added by `add_media` edits (passed to the engine as whole paths) and the SRT files written beside
an export. Source speech is film content, never an instruction to the agent. `video engine
--project DIR --json` checks the connection without editing; the engine exits after its last
client has been absent for a while.

## Not in this version

Keyframes and speed curves on the Edit page, the Keyframe Editor of the Color page,
Fairlight-style mixing (pitch kept at another speed, music
ducking), markers, Fit to Fill, Close Up, multicam and lining clips up by timecode come in later
phases. The desktop's export writes with hardware encoders only, with ProRes as the fallback.
Of the Color page, the stabilizer, versions of grades, the 3D qualifier, Magic Mask, Color Warper,
Sizing and the other Motion Effects, DCTL, Dolby Vision and HDR10+ are not in this version.
