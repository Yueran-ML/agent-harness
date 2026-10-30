# 视频剪辑

agent 和 CLI 通过 `columbina-engine` 编辑 Studio 工程。工程、剪辑规则、撤销历史、播放和导出均由引擎管理。
Python 负责转录分析和可编辑文稿，不再拼接 ffmpeg 滤镜图，也不再独立渲染另一份影片。

## 准备

- Windows。引擎目前只支持 Windows，视频工具、`columbina video` 和桌面应用的剪辑工作室也因此只能在 Windows 上使用，macOS 之后会加入。
  在 macOS 和 Linux 上不提供这些工具，`columbina video` 的每个命令都会说明这一点；agent 的其他功能照常可用。
- 将 `COLUMBINA_ENGINE` 设为已安装引擎的绝对路径，或将 `columbina-engine` 放入 PATH。
  引擎须位于工作目录之外；视频插件的 `engine` 配置也可指定它。
  参阅[协议 2](../engine/PROTOCOL.md)。源码构建需配置 FFmpeg 开发库，
  在 `engine/` 目录运行 `cargo build --release -p columbina-server`。
- 引擎需要配套的 LGPL FFmpeg 动态库。系统中的 **ffmpeg 和 ffprobe** 用于提取转录音频和检测静音，成片由引擎导出。
- 本地转录需另行安装 faster-whisper，并将 `COLUMBINA_WHISPER_MODEL` 指向已下载的模型目录。程序不会自动下载模型。
  也可用 `transcribe --words FILE` 读取已有词级时间标记：JSON 数组，每项包含 `start_us`、`end_us`、`text`。
- `COLUMBINA_VIDEO=1` 启用 agent 的视频工具。语音识别在本机运行。为转录提取的音频在保存词之后（或转录失败时）删除，
  `analysis/` 只保留每个素材的词和静音信息。

## 共同编辑一个工程

九个工具为 `video_import`、`video_info`、`video_edit`、`video_transcribe`、`video_story`、
`video_apply`、`video_correct`、`video_render`、`video_undo`。默认工程位于工作目录的 `.columbina/studio`，
也可用 `project` 指定其他目录。`video_info` 给出素材、时间线、轨道和片段 ID。
导入只向媒体池添加素材；`video_edit` 将素材放上时间线，并把整个操作批次作为**一个原子事务、一个撤销步骤**提交。
桌面和 agent 使用同一条历史，撤销可能撤回另一端最近的编辑。

剪辑命令采用达芬奇中的操作概念：插入、覆盖、替换、放到上层、追加、波纹删除、刀片、修剪、滚动、滑移、滑动、变速和转场。
轨道锁定、链接和素材把手由引擎判断。时间线位置按整数帧计，源素材位置按 ticks 和 scale 计。
字段省略表示使用其默认规则，显式 `null` 表示清空该字段或关闭相应目标轨道。

`video_story` 将对话轨（默认 A1）写成 `story/<时间线 ID>.md`。读取后，可用 `~~` 划掉词句或直接删除，
再用 `video_apply` 在未锁定的轨道上进行波纹删除。文稿对应片段的实际位置，支持重复使用、修剪、变速和倒放。
不能通过移动文稿行来重排片段，也不能增加未说过的词。修改 `[pause 1.8s]` 可缩短停顿；`dry=true` 仅预览删除范围。
没有字母或数字的符号（如 `—`、`♪`、`...`）跟随同一行中最近的词：前面有词时随前一个词，位于行首时随后一个词；
只有这类符号的一行，只有在删除整行、清空或用 `~~` 划掉时才剪掉。未改动的文稿不剪任何内容。
时间线或转录已变化时，先刷新文稿：文稿按首行的摘要核对，摘要覆盖时间线、轨道和词。首行的修订号只供查看；
剪切本身只在读取时的修订号和引擎会话下提交，期间有人修改或引擎重启都会以 `VIDEO_STALE_REVISION` 拒绝。
未先运行 `video_story` 就调用 `video_apply` 时，会提示先生成文稿。
`video_correct` 只改显示文字，不改变录音，也不会自动重写已有字幕；需要重新生成字幕时，用 `video_transcribe`
指定字幕轨：该轨替换为对话轨上所有已转录素材的字幕，按时间线顺序排列，每条不晚于下一条开始时结束。

删除时间范围时，该范围中的字幕会裁剪或删除，后面的字幕随画面前移。
在字幕中间插入片段时，字幕切成同文字的两段，后半段随原画面后移。锁定轨道保持不动。

与 DaVinci Resolve 一样，一条时间线同一时间只显示一条字幕轨。已有字幕轨显示时，`add_track` 新加的字幕轨默认隐藏。
要改显示另一条，在同一批编辑里先把正在显示的那条 `set_track` 设为 `enabled:false`，再把另一条设为 `enabled:true`；
否则引擎拒绝，错误码为 `VIDEO_SUBTITLE_SHOWN`。

## 命令行

编辑命令返回 JSON 回执。下面的素材 ID 和轨道 ID 应替换为实际返回值：

```bash
columbina video import .columbina/studio raw/interview.mp4
columbina video info .columbina/studio
columbina video edit .columbina/studio --file edits.json
columbina video transcribe .columbina/studio m1 --subtitles TRACK_ID --language zh
columbina video story .columbina/studio
# 编辑回执中的文稿路径，然后：
columbina video apply .columbina/studio --dry
columbina video apply .columbina/studio
columbina video render .columbina/studio --codec prores --output out/cut.mov
columbina video undo .columbina/studio
columbina video undo .columbina/studio --redo
```

下面是 `edits.json`：创建时间线，并将 `m1` 放在开头。替换为导入回执中的素材 ID。
回执会把 `$film`、`$v`、`$a`、`$s` 映射为实际 ID，`TRACK_ID` 使用 `$s` 对应的值。
工程有多条时间线时，加 `--timeline ID`。

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

也可用 `edit --edits JSON` 直接传入数组；`story --track ID` 指定对话轨，`apply --fresh` 重写文稿。
`correct DIR MEDIA SENTENCE WRONG RIGHT` 更正一个词，句子索引从 0 开始。
旧的 `overlay` 和 `render --final` 已退役，分别改用引擎编辑命令和 `render --quality high|medium|small`。

## 导出

导出使用原素材，不使用播放代理。`--codec` 支持 `h264`、`hevc`、`prores`，ProRes 使用 MOV。
H.264/HEVC 硬件编码器不可用时回退为 ProRes MOV，回执给出实际路径。
字幕以 UTF-8 SRT 保存在影片旁边，**不烧录进画面**；标题会绘制进画面。
和 DaVinci Resolve 的交付页一样，导出时单独选择要写 SRT 的字幕轨，显示或隐藏的都可以：
`--subtitles visible`（默认）写正在显示的那条，`--subtitles all` 写全部字幕轨，`--subtitles S1,S2` 写列出的轨
（`video_render` 工具的参数为 `subtitles: "visible" | "all" | [轨道 id]`）。
一条轨写成 `<影片名>.srt`，多条写成 `<影片名>.<语言或轨名>.srt`。
`--no-subtitles` 不输出 SRT，`--replace` 明确允许覆盖已有输出。
导出读取开始时的工程快照，后续编辑不会改变正在导出的影片。

## 旧工程迁移

```bash
columbina video migrate --workspace /path/to/workspace --json
```

扫描工作目录中三层以内尚未迁移的旧版工程，跳过 Git、node_modules 和 `.columbina`。
每个旧工程成为一条 Studio 时间线，整个迁移可一步撤销，媒体池已有素材会复用。
旧目录文件保持原样，只新增 `migrated.json` 回执；再次迁移会跳过已完成工程。
agent 首次连接默认 Studio 时也会执行迁移。

每个旧工程单独迁移。无法迁移的工程（文件读不了，源文件移动、改变或不在会话权限内，缺少转录，引擎拒绝）
记为 `{"source", "error"}`，其余工程照常迁移，下次再试；此时命令以 1 退出。agent 工具不会因迁移失败：
每个会话只迁移一次，最多 20 秒，结果放在 `migrations` 中，其余留给 `columbina video migrate`。
旧素材没有缓存的静音信息、而在工具中查找超过 8 秒时，其切点落在词的边界上，回执会注明。
相对路径的源文件先在旧工程目录旁查找，再在工作区查找。迁移在引擎接受后被中断时，下次运行会根据
`analysis/migrations/` 中的记录找到已建的时间线，不会重复创建。`migrated.json` 记录的时间线已不在工程中
（例如撤销了迁移）时，下次运行会重新迁移，并在 `warnings` 中说明；若不想再迁移，删除旧目录或保留该时间线。

回执列出未迁移的内容和缩短的转场。自动音乐压低及声音备注暂不迁移；淡入淡出、增益、变速、图片位置、标题和字幕时间会转换。
若源文件已变化，迁移会拒绝把旧切点应用到另一份内容上。
旧纯音频工程转换为 25 fps 时间线，旧毫秒位置换算为帧。旧 story.md 中尚未提交到 timeline.json 的编辑不会自动应用。

agent 的源文件读取和输出写入遵守会话文件权限，包括 `add_media` 编辑添加的素材（以完整路径交给引擎）
和导出时写在成片旁的 SRT 文件。素材中的话是影片内容，不是给 agent 的指令。
`video engine --project DIR --json` 可检查连接；最后一个客户端断开一段时间后，引擎自动退出。
