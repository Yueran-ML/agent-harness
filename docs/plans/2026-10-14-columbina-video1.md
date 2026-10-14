# Columbina 视频一实施计划：媒体与工程

**Goal:** 视频剪辑的底座。找到可以信任的 ffmpeg 和 ffprobe，查清素材是什么，从素材里抽出音频、找出静音，工程目录可以读写，时间线的每次提交可以撤销和重做。这一期结束时，导入一批文件，工程里记下的事实和 ffprobe 说的一致。

**Architecture:** 新包 `columbina/video/`，这一期有两个模块。`media` 管系统里的 ffmpeg：找程序、问版本、探测素材、抽音频、找静音。命令经 `process.run` 运行，参数从不经过 shell。`project` 管工程目录：数据模型、导入、素材的身份、原子写入、时间线的历史。两块都还不是插件，没有工具，也不进组合清单，那是视频五的事。找程序的规矩与容器沙箱找 `docker` 相同，所以那段代码先挪到公共模块 `columbina/programs.py`。

**Tech Stack:** 系统里装好的 ffmpeg 和 ffprobe，6.0 或更新，不随包分发。不加 Python 依赖。本机是 ffmpeg 8.1.1 的完整构建（libavformat 62，libavfilter 11）。

设计依据是 `docs/plans/2026-10-14-columbina-video-design.md`。作者 2026-10-14 让这一块按设计里的建议推进，八件需要决定的事都按建议，三件请求的事见下面"作者的三件事"。

---

## 结束标志

1. 用 ffmpeg 现场生成的一批素材导入同一个工程：横拍的视频、带旋转标记的竖拍视频、帧率会变的视频、带封面的 mp3、wav、png。工程里每个素材的种类、时长、画面宽高、旋转、帧率、是否变帧率、像素格式、音频的采样率和声道，与 ffprobe 对同一文件的回答逐项一致。
2. 工程的画幅和帧率取第一个视频素材的：竖拍的那段给出转正之后的宽高，变帧率的那段给出最接近的标准帧率。
3. 换掉一个已导入的文件之后再用它，程序说文件变了，不拿旧的事实去用。
4. 一段带两处停顿的口播（用纯音和静音拼成），静音检测给出的两段时间与拼接时的位置相差不超过 20 毫秒；抽出的音频是 16 kHz 单声道，时长与素材一致。
5. 时间线提交三次、撤销两次、重做一次之后，时间线的内容和预期的那一份逐字节相同；基于过期内容的提交被拒绝，时间线不变。
6. 机器上没有 ffmpeg 时，用到它的测试跳过；设了 `COLUMBINA_REQUIRE_FFMPEG=1` 时这算失败。持续集成的 Ubuntu 机器装好 ffmpeg 并设这个变量。

## 作者的三件事

- **验收用的真实素材。** 没有给。这一期的验收全用现场生成的素材，口播用系统的语音合成（`_review/video_exp/make_speech.ps1`）。
- **装 faster-whisper 和下载模型。** 已装在仓库之外的试验环境 `_review/whisper` 里，模型 `Systran/faster-whisper-large-v3`（2.9 GB）和 `Systran/faster-whisper-small`（464 MB）下载到 `_review/whisper_models`，从 Hugging Face 取。用于试验一和视频二。
- **阶段表。** 不改现有编号，这一块叫视频一到视频五，排在补强计划第二组之后。补强计划的两组都已提交。

## 范围

这一期做的：公共模块 `programs`。`media`：找到 ffmpeg 和它旁边的 ffprobe，查版本，探测素材，抽 16 kHz 单声道音频，静音检测，结果按素材的身份缓存。`project`：工程目录的布局，清单的数据模型，导入，素材的身份与核对，原子写入，时间线的提交、撤销和重做。真实 ffmpeg 的测试，持续集成。

这一期不做的：转写、稿子、渲染、挂件（视频二到视频四）。给模型的工具、给人的命令、组合清单和指南（视频五）。抽帧留到需要它的时候。

## 约定

与第 7 阶段相同：在工作树 `_review/s5b` 里做，闸门通过之后快进本地的 `main`，不执行 `git push`。每一步先写测试，看它失败，再写实现，过闸门，提交。提交信息用英文，末尾不加共同作者行。新增的行为各有变异体。同一时间只跑一项重任务，用到 ffmpeg 的测试和变异检查算重任务。测试用的素材都在测试里现场生成，测试自己不下载任何东西。

## 先做的试验

设计第 13 节的七项试验，这一期用到的是第 4、5、7 项，都已做完，记录在 `_review/video_exp/results.md`。

**试验四：手机拍的那几类素材。** 没有手机录的素材，用 ffmpeg 造了三种。

- 帧率会变的文件，ffprobe 说 `r_frame_rate` 30/1，`avg_frame_rate` 6210/269（23.09）。前者只是能容下所有时间戳的最细的帧率，不是播放的帧率。两者不同就是变帧率，两者都不能直接当工程的帧率。
- 先用 `fps` 滤镜把帧补齐成工程的帧率，再按时间选段，剪出的画面帧数正好，每一帧来自素材的正确位置。声音按 1 毫秒一块选段，长度也正好。
- 带显示矩阵的文件，ffprobe 报的宽高是存储的 1280x720，旋转在 `side_data_list` 里，`rotation: 90`，单位是逆时针的度数。ffmpeg 在 `-vf` 和 `-filter_complex` 下都会自己转正，出来是 720x1280。所以素材显示的宽高是存储的宽高在 90 度和 270 度时对调，工程取的是这个。
- 10 位的 HEVC，ffprobe 给出像素格式 `yuv420p10le`。HDR 看 `color_transfer` 是不是 smpte2084 或 arib-std-b67，文件里有这个标记时才认得出。

**试验五和试验七** 的结论已经写进设计第 10 节：滤镜参数里只出现程序自己起的文件名；对外来文件调用 ffprobe 和 ffmpeg 时同时加协议白名单和格式白名单。

另补了三个小检查（`_review/video_exp/e8 …`）：路径写成 `file:` 加 Windows 的完整路径时，带中文、空格、逗号、单引号和方括号的目录照样读得到；格式白名单里没有 `image2` 时，png 被认作 `png_pipe`，照样读得出；mp3 的封面是一路视频流，`disposition.attached_pic` 为 1。

## 设计决定

### 程序

**1. 找程序的规矩挪到 `columbina/programs.py`。** `found(name, search, outside)` 和 `trusted(program, outside)` 原样从 `sandbox/docker.py` 挪过来，`found` 多一个程序名的参数。测试和 23 个变异体随之挪走，成为单独的一组。docker 照旧用它们。

**2. ffmpeg 在搜索路径上找，ffprobe 取它旁边的那个。** 两个程序出自同一次构建，选项才一致，所以 ffprobe 不另外搜索：ffmpeg 所在的目录里没有 ffprobe，就说缺了它。`outside` 是本程序启动时的目录和会话的工作目录，与 docker 相同。每次运行之前再走一遍 `trusted`，路上的东西可能在两次运行之间被换掉。找不到时说怎样装：Windows 上 `winget install Gyan.FFmpeg`，macOS 上 `brew install ffmpeg`，Linux 上用发行版的包。

**3. 版本看库的版本，不看程序的版本号。** 夜间构建的版本号是 `N-12345-g…`，没有数字可比。`ffmpeg -version` 里 `libavformat` 和 `libavfilter` 的主版本号总在。要求 libavformat 60 以上，也就是 ffmpeg 6.0 以上：Ubuntu 24.04 带的是 6.1。libavfilter 10 以上（ffmpeg 7.0）才有 `-/filter_complex`，更旧的用 `-filter_complex_script`。这一期只记下版本，选哪个选项是渲染的事。

### 探测

**4. 对外来文件的每次调用都加两张白名单。** 协议只许 `file`，格式只许下表里的解复用器，输入写成 `file:` 加绝对路径：文件名里有冒号或者以减号开头时，不会被当成别的协议或者选项。

| 解复用器 | 用于 |
|---|---|
| `mov` | mp4、mov、m4a、3gp |
| `matroska` | mkv、webm |
| `avi`、`mpegts` | avi、ts、mts |
| `wav`、`mp3`、`flac`、`ogg`、`aac` | 声音 |
| `png_pipe`、`jpeg_pipe`、`webp_pipe` | 图片 |

`image2` 不在表里：它会按文件名里的格式串去读同一目录下的一串文件。`mov` 的外部引用缺省就不跟随，不用另加选项。ffprobe 报出的 `format_name` 再和程序自己的表对一遍，不在表里的拒绝。

**5. 事实从 ffprobe 的 JSON 里取，取法是纯函数。** 第一路不是封面的视频流给出画面，第一路音频流给出声音。时长用 `format.duration`，按十进制字符串换成整数微秒，不经过浮点。旋转取显示矩阵的 `rotation`，规整到 0、90、180、270。显示的宽高是存储的宽高乘上像素宽高比，90 度和 270 度时对调。`avg_frame_rate` 与 `r_frame_rate` 相差超过百分之一时算变帧率。像素格式名里带 10 或 12 的算高位深，`color_transfer` 是 smpte2084 或 arib-std-b67 的算 HDR，两者导入时都警告：这一版不做色彩转换。

**6. 素材分三种。** 格式是图片的是图片；有画面的是视频；只有声音的是音频；两样都没有的拒绝。带封面的 mp3 是音频。

### 工程

**7. 目录布局按设计第 3 节。** 这一期用到 `project.json`、`timeline.json`、`history/`、`cache/`。别的目录由后面几期建立。

**8. 清单是 pydantic 模型，写成缩进的 JSON。** 字段：版本号；画幅（宽、高、帧率，帧率写成 `30000/1001` 这样的分数字符串）；素材表。素材有编号、路径、种类、格式、时长（微秒）、画面的事实、声音的事实、身份。读到不认得的版本号时拒绝，不猜。

**9. 素材的编号取自文件名。** 去掉扩展名，转成小写，字母、数字和汉字之外的字符换成连字符，最长 40 个字符，重名时加 `-2`、`-3`。编号一旦给出就不变，稿子和挂件靠它指向素材。

**10. 身份是大小、修改时间和头尾的摘要。** 文件开头和结尾各 64 KiB，连同大小一起算 SHA-256。用素材之前核对：大小或修改时间变了就重算摘要，摘要也变了就是换了文件，程序说出来，不拿旧的事实和缓存去用。只改了修改时间而内容没变的，更新修改时间，照常使用。

**11. 导入是幂等的。** 同一个路径再导入一次：身份相同就什么都不做；身份不同就重新探测，更新事实，回执里写明。路径记成绝对路径，不解析链接：用户指的是这个名字。

**12. 工程的画幅取第一个视频素材的。** 宽高取显示的宽高。帧率在定帧率时取 `r_frame_rate`；变帧率时取 `avg_frame_rate` 最接近的标准帧率：23.976、24、25、29.97、30、48、50、59.94、60。

**13. 每个文件都是先写临时文件再改名。** 临时文件在同一个目录里，名字以点开头，带随机后缀。改名用 `os.replace`。Windows 上目标被别的程序开着时改名会失败，这时报错，说是哪个文件，不重试。

### 缓存

**14. 抽音频和找静音的结果按身份缓存。** 缓存的文件名里有素材编号和摘要的前 12 位，静音检测还带上参数。身份变了，旧的缓存自然用不上。缓存删掉可以重建。

**15. 抽音频：16 kHz、单声道、16 位 PCM 的 wav。** 输出写成一个裸文件名的临时文件，成功之后改名到 `cache/`。

**15a. ffmpeg 和 ffprobe 每次都在为这一次新建的目录里运行。** 目录在 `cache/` 下，名字带随机的后缀，用完删掉。程序在 Windows 上找动态库时会看当前目录，工作目录里的东西是命令写得进去的，所以不在那里运行。结果先写在这个目录里，查过之后改名到 `cache/`，同一个盘上改名是一步完成的。

**16. 静音检测：`silencedetect` 加 `ametadata` 写到文件。** 一小时的口播有上千处停顿，写到错误输出会超过 `process.run` 留下的长度。`ametadata` 的 `file` 参数只写裸文件名，ffmpeg 在 `cache/` 里运行，这是试验五的结论。缺省的门限是 -35 dB，最短 0.3 秒，这两个数是参数，等试验一和试验二的结果再定缺省。结果是一串 `(开始, 结束)` 的微秒数，素材以静音结尾时最后一段的结束是素材的时长。

### 历史

**17. 撤销和重做是两个栈。** 每次提交把当前的时间线压进撤销栈，清空重做栈。撤销把当前的压进重做栈，从撤销栈取一份回来；重做反过来。两个栈都是 `history/` 下按序号命名的文件，带一份说明：哪个会话的哪次调用、回执的第一行、时间。撤销栈保留最近的两百份。取回来的时间线和提交走同一条写入的路，稿子随之重新生成（视频三）。

**18. 提交带着它所依据的时间线的摘要。** 当前时间线的摘要和它不同，就是别处改过，提交被拒绝，时间线不动。撤销和重做也一样。同一个进程里对同一个工程的提交依次进行，不同进程之间靠这个摘要发现冲突。

### 测试与持续集成

**19. 用到 ffmpeg 的测试在没有 ffmpeg 的机器上跳过。** 设了 `COLUMBINA_REQUIRE_FFMPEG=1` 时这是失败，与容器的测试同一个规矩。素材在测试里用 `lavfi` 生成：纯色画面配固定频率的单音，旋转标记用 `-display_rotation`，变帧率用 `setpts` 的表达式。

**20. 解析 ffprobe 的回答有自己的测试，不需要 ffmpeg。** 回答取自真实文件，存成测试数据：横拍、竖拍、变帧率、带封面的 mp3、png、wav、10 位。本机的闸门要求 100% 的覆盖率，这些测试加上真实 ffmpeg 的测试把每一行都走到。

**21. 持续集成的 Ubuntu 机器装 ffmpeg。** `sudo apt-get install -y --no-install-recommends ffmpeg`，并设 `COLUMBINA_REQUIRE_FFMPEG`。Windows 和 macOS 的机器不装，那里的测试跳过。

## 模块与接口

### `columbina.programs`（新建）

`found(name, search, outside=()) -> str | None`，`trusted(program, outside) -> bool`。

### `columbina.video.media`（新建）

- `Tools`：ffmpeg 和 ffprobe 的路径，libavformat 和 libavfilter 的主版本号。
- `located(search, outside) -> Tools`：找程序，查版本。找不到、太旧、ffprobe 不在旁边时抛 `MediaUnavailableError`，话里说怎么办。
- `probe_argv(tools, path) -> list[str]`、`facts(answer: dict) -> Facts`：纯函数。
- `probed(tools, path, *, cwd, outside) -> Facts`：运行 ffprobe。
- `audio(tools, source, target, *, cwd, outside)`、`silences(tools, source, *, cwd, outside, noise_db, shortest_us) -> tuple[Silence, ...]`。

### `columbina.video.project`（新建）

- 模型：`Rate`、`VideoFacts`、`AudioFacts`、`Identity`、`Asset`、`Canvas`、`Manifest`。
- `Project(directory)`：`manifest()`、`imported(paths, tools, outside) -> Imported`、`checked(asset) -> Asset`（核对身份）、`cached_audio(asset)`、`cached_silences(asset, …)`。
- 时间线：`timeline() -> (text, digest)`、`commit(text, base, note) -> digest`、`undo(base, note)`、`redo(base, note)`；冲突时抛 `StaleError`。

## 任务

1. **公共模块 `programs`。** 文件：`programs.py`、`sandbox/docker.py`、`tests/test_programs.py`、`tests/sandbox/test_docker.py`、`tests/support/engines.py`、变异体。（新组 `programs`）
2. **找 ffmpeg，查版本。** 文件：`video/__init__.py`、`video/media.py`、测试。测试：搜索路径上的 ffmpeg 和旁边的 ffprobe 被找到；工作目录里的不用；ffprobe 不在旁边、版本太旧、版本读不出、程序启动不了各是什么话；每次运行前再查一遍路上的链接。（新组 `video-media`）
3. **探测与事实。** 文件：`video/media.py`、`tests/video/data/*.json`、测试。测试：七种素材的回答各自给出的事实；白名单的参数；`file:` 前缀；格式不在表里、没有画面也没有声音、时长读不出时拒绝；旋转的四个方向和负数；像素宽高比；变帧率的判断。真实 ffmpeg：结束标志第 1 条。（`video-media`）
4. **工程、清单与导入。** 文件：`video/project.py`、测试。测试：新工程的目录；编号的取法与重名；幂等的导入；换了文件再导入；画幅取第一个视频素材，竖拍的对调，变帧率的取标准帧率；清单的版本号不认得时拒绝；写到一半失败时旧文件完好。（新组 `video-project`）
5. **身份与核对。** 文件：`video/project.py`、测试。测试：只改修改时间时照常用并更新；改了内容时说出来；文件不在了时说出来；头尾之外的改动按大小和修改时间发现。（`video-project`）
6. **抽音频和静音检测，按身份缓存。** 文件：`video/media.py`、`video/project.py`、测试。测试：结束标志第 4 条；缓存命中时不再运行 ffmpeg；身份变了重做；参数不同各存一份；以静音结尾的素材；没有声音的素材说清楚。（`video-media`、`video-project`）
7. **时间线的历史。** 文件：`video/project.py`、测试。测试：结束标志第 5 条；过期的提交、撤销、重做被拒绝；撤销栈满两百份时丢掉最旧的；没有可撤销或可重做的时候说清楚；性质测试：任意一串提交、撤销、重做之后，时间线等于一个简单模型给出的那一份。（`video-project`）
8. **持续集成与设计文档。** `ci.yml` 给 Ubuntu 装 ffmpeg 并设变量；设计文档的状态和第 4、10 节按试验和本计划改写；主设计文档补一节视频剪辑的位置。

## 留给作者的检查

这一期没有给人用的入口，作者不用做手工检查。持续集成里第一次在 Ubuntu 上跑真实 ffmpeg 的测试，结果要看一眼：失败的测试会写在运行的摘要和注释里。
