# 剪辑视频

Columbina 可以按视频里说的话来剪片子，并在上面叠画面、配乐和标题。它用的是你系统里的 ffmpeg；把说的话写下来用的是本机上的 faster-whisper。全程不往外发送任何东西。

## 需要准备

- **ffmpeg 6.0 或更新**，ffprobe 放在它旁边：Windows 上 `winget install Gyan.FFmpeg`，macOS 上 `brew install ffmpeg`，Linux 上用发行版的 `ffmpeg` 包。
- **faster-whisper 和一个模型**，用来转写。把它装进 Columbina 用的 Python：`pip install faster-whisper`，再把模型下载到你自己的目录：

  ```bash
  huggingface-cli download Systran/faster-whisper-small --local-dir ~/models/faster-whisper-small
  ```

  `large-v3` 听得更准，需要显卡；`small` 用处理器就能跑。Columbina 不会自己下载模型。

## 交给智能体

设了 `COLUMBINA_VIDEO=1` 才会有剪辑工具，`COLUMBINA_WHISPER_MODEL` 指向模型目录：

```bash
COLUMBINA_VIDEO=1 COLUMBINA_WHISPER_MODEL=~/models/faster-whisper-small columbina
```

然后直接说要什么，例如：「用 `raw/interview.mp4` 在 `talk` 里做一个片子：剪掉口头语和重说的句子，缩短长停顿，讲到这个工具的地方盖上 `raw/screen.mp4`，全程垫上 `music.mp3`。」智能体会导入文件、转写口播、改稿子、加挂件，并渲染一个预览。

## 稿子

一个工程就是一个目录：`project.json` 列出素材，`timeline.json` 是提交后的成片，`story.md` 是口播的文字稿。稿子里一行是一句：

```text
<!-- columbina story: 3f9a61c2e0d4 -->

[talk.0 00:01] 大家好，~~呃~~ 今天聊一聊怎么剪视频。
[talk.1 00:06] ~~这个这个~~ 这个工具可以按文字来剪。
  > picture screen picture-1
[pause 1.8s]
[talk.2 00:12] ~~我们先看，不对，我重说一遍。~~
```

- 删掉一行，或者用 `~~` 划掉，这句就剪掉；删掉或划掉某个词，这个词就剪掉；去掉划线就恢复。
- 挪动一行，这句话就挪过去。
- `[pause 1.8s]` 是这句之后的停顿：数字改小就缩短，删掉这一行就缩到很短。
- 以 `>` 开头的行说明那里盖着什么，只读。
- 不能加词或改词：稿子里的字就是录下来的声音。听错的字用 `video_correct` 或 `columbina video correct` 改显示。

用 `video_apply` 提交稿子。回执会说剪了什么、成片前后各多长、哪些挂件因为所在的词被剪掉而没处挂，以及哪些剪点落在没有停顿的地方、可能听得出来。每次提交都可以撤销。

两句之间的剪点落在中间的静音里，两边各留一点静音；转写给的时间会先按停顿整体校正。

## 手动操作

同一套操作，不经过模型：

```bash
columbina video import talk raw/interview.mp4 raw/screen.mp4 music.mp3
columbina video transcribe talk interview --model ~/models/faster-whisper-small
# 编辑 talk/story.md
columbina video apply talk
columbina video overlay talk add --kind picture --source screen --sentence interview.1 --words "按文字来剪" --position top-right --scale 0.4
columbina video overlay talk add --kind sound --source music --volume 0.4
columbina video render talk            # 预览
columbina video render talk --final    # 成片，旁边附字幕文件
columbina video undo talk
columbina video info talk
```

已经写好的逐词稿也可以代替模型，用 `--words FILE` 给出：一个对象列表，每项有 `start_us`、`end_us` 和 `text`。

## 安全

读素材、写工程目录都和读写普通文件一样，遵守本会话的规则。对素材运行的每一次 ffmpeg 都只允许本地文件、只允许视频、音频和图片的格式：指向别的文件的清单（例如 HLS 或 concat 清单）不会被打开。ffmpeg 的滤镜参数里从不出现路径。素材里说的话只是片子的一句台词，不是给智能体的指令。
