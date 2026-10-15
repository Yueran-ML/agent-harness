# Columbina 桌面应用的发布计划

**Goal:** 从 GitHub 的 Release 直接下载桌面应用，解压就能用：Windows x64 和 Apple 芯片的 macOS 各一个包。包里带着服务进程要的 Python 和 columbina，开场视频和图片都在，换一台没装过任何东西的机器，打开的样子和作者本机一样。界面在两个平台上各按其习惯排布。

**Architecture:** 应用打包之后默认启动包里自带的服务：一份独立的 CPython，装好从同一个提交构建的 columbina。只有开发时（没有打包）才回到模拟的服务；指定了 `COLUMBINA_SERVICE_EXECUTABLE` 时照旧用指定的。素材从仓库里的 `desktop/assets/` 取。CI 在 Windows 和 macOS 上各打一个包，试过之后作为构件上传，打 `desktop-v*` 标签时附到 Release 上。PyPI 不动。

**Tech Stack:** 不增加运行时依赖。打包用已有的 `@electron/packager`，Python 用 uv 取得（python-build-standalone，版本固定为 3.12.13），CI 用 GitHub Actions，macOS 上用系统自带的 `codesign`、`ditto`、`sips`、`iconutil`。

2026-10-15 作者要求从 GitHub 能直接下载图形界面的版本，开场视频一起上传，并且 macOS 和 Windows 的界面分别适配（作者在 Mac 上看到标题栏左上角的红绿灯压在了标志上）。作者定了四件事：包里内置 Python；应用用到的三份素材进仓库，47 MB 的原片不进；平台是 Windows x64 和 Apple 芯片的 Mac；只发桌面应用，PyPI 保持 0.1.0。

---

## 不做的

- 代码签名和公证。两者都要付费的证书。Windows 会弹出 SmartScreen 的提示，点“更多信息”再点“仍要运行”；macOS 要在终端里运行一次 `xattr -dr com.apple.quarantine /Applications/Columbina.app`，下载说明里写明。只去隔离标记、不走“隐私与安全性”放行，是因为包里的 Python 也是可执行文件，放行应用本身不一定连带放行它。
- 安装程序。先发 zip，解压即用。
- Intel 的 Mac、Linux、自动更新。

## 设计决定

**1. 服务从哪里来。** 打包后的应用在 `resources/python` 里找自带的解释器，用 `python -I -B -m columbina serve` 启动；`columbina setup` 也走它。`-I` 不读任何 `PYTHON` 开头的变量、不加用户的 site，也不把项目文件夹放到包里的包前面（服务的工作目录就是用户的项目）；`-B` 不往包里写字节码。最初的计划是在环境里设 `PYTHONNOUSERSITE` 等变量，改用参数是因为环境会被 agent 运行的命令继承，那些变量会改变用户自己项目里的 Python。包里没有解释器时报错，不退回模拟服务：用户看到的必须是真的。开发时（没打包）照旧：没指定服务就用模拟服务。

**2. 内置的 Python。** `scripts/prepare-python.mjs` 用 uv 取一份 CPython 3.12.13（python-build-standalone，可以整个目录搬走），先解到系统的临时目录再复制过来（在作者的 F 盘上 uv 移动刚解出来的目录会被拒绝访问），保留其中的符号链接。依赖用 `uv export --locked` 导出、按哈希安装，版本和 `uv.lock` 一致，包括 `mcp`；columbina 用同一个提交构建的轮子装进去。uv 给它的 Python 带着 `EXTERNALLY-MANAGED` 标记，复制出来的这一份是应用私有的，去掉这个标记再装。之后删掉服务用不到的部分（IDLE、tkinter、pip、编译扩展用的头文件和库，以及按安装路径调用 Python、搬走后就失效的脚本），并以 `unchecked-hash` 方式预编译全部模块：解压后的文件时间对不上也照样用，运行时不会往包里写，在 macOS 上就不会破坏应用的签名封装。最后用它跑一次 `columbina --version`。

**3. 素材。** 加长静音版开场视频（10.6 MB）、封面（2.5 MB）、图标（1.1 MB）放进 `desktop/assets/`，提交进仓库，`.gitattributes` 把视频和图标标成二进制。`npm run assets:prepare` 默认从这里取，`COLUMBINA_ASSET_DIR` 和 `assets.local.json` 仍然可以另指。版权说明不变：同人、AI 生成、不在 MIT 许可之内。原片照旧不进仓库，也不进应用。

**4. 两个平台的界面。** 预加载脚本把平台告诉页面，窗口的根元素带上 `data-native`，值就是平台名。
- macOS：标题栏左边给红绿灯留出位置，右边不留；红绿灯在 40 像素高的标题栏里垂直居中（`trafficLightPosition: { x: 16, y: 13 }`），全屏时红绿灯隐藏，标题栏把左边的位置收回；表单（选文件夹的对话框、填密钥的窗口）从标题栏下方弹出（`setSheetOffset`）；快捷键提示写 ⌘（⌘B，批准卡片上是 ⌘ Enter）；应用菜单只有应用、文件、编辑、窗口四项，能复制粘贴、关窗、退出，没有“重新加载”和“开发者工具”；图标用 `.icns`。
- macOS 从访达打开的应用只有很短的 `PATH`，找不到 Homebrew 装的程序。打包的应用启动时问一次用户的登录 shell（像终端那样以 `-ilc` 启动，答案两头加标记），服务和 agent 的命令都用这个 `PATH`；5 秒内没有答案就在原来的 `PATH` 后面补上 Homebrew 和 `/usr/local` 的目录。
- Windows：维持现状，右边留给窗口按钮。

**5. 打包。** `electron/package.mjs` 取代 `package-win.mjs`，按所在平台打：win32 x64 或 darwin arm64，并生成该平台的图标。Python 在打包器之后再复制进去：打包器复制额外资源时会把相对链接改成指向本机的绝对路径，换一台机器 `bin/python3` 就断了。macOS 打完后做 ad-hoc 签名（`codesign --force --deep --sign -`）并校验，再用 `ditto` 压成 zip，保住包里的符号链接；Windows 用系统自带的 `tar.exe` 压成 zip，文件放在 zip 的顶层，用“全部解压缩”会解到以 zip 命名的文件夹里。

**6. CI。** `.github/workflows/desktop.yml`，在 `desktop-v*` 标签和手动触发时运行，windows-latest 和 macos-latest 各一个任务：装依赖、跑桌面端的测试、打包、试包、上传构件；标签触发时先检查标签与 `desktop/package.json` 的版本一致，最后创建 Release 并附上两个 zip。试包（`scripts/try-package.mjs`）用包里的解释器跑 `columbina --version`、一个脚本化模型的任务、`columbina serve` 的握手（`initialize` 之后 `shutdown`），并确认包里的文件一个都没被改动；macOS 上再校验一次签名封装。

## 任务

1. 素材进仓库：`desktop/assets/`、`prepare-assets.mjs`、`ASSETS.md`、`.gitignore`、`.gitattributes`。
2. 服务的选择：`main.cjs` 的 `serviceCommand`，测试。
3. 两个平台的界面：预加载、`App.tsx`、`shell.css`、快捷键提示、菜单、`trafficLightPosition`、全屏、表单位置、登录 shell 的 `PATH`，测试。
4. 内置 Python 的准备脚本。
5. 按平台打包，macOS 的图标，试包脚本。
6. CI 工作流。
7. 文档：README 三种语言的下载一节，`desktop/README.md`，CHANGELOG。
8. 验证：在本机打 Windows 的包并试包，不设任何服务变量地打开它；手动触发工作流，下载两个包；macOS 的包请作者在 Mac 上打开。

## 验收

- 在没装 Python、没设任何环境变量的 Windows 账户里，解压后双击 `Columbina.exe`：开场视频播放，进入界面，能设好 DeepSeek 的密钥并完成一个任务。
- macOS 上同样，标题栏里没有东西被红绿灯压住（作者验证）。
- CI 的试包在两个平台上都通过。

## 本机验证（2026-10-15）

- 桌面端测试 1310 个通过，在 Linux 容器里拷一份整个仓库再跑也全部通过（macOS 的 CI 会跑同一套）。主进程的 20 个手工变异全部被测试抓到。
- Windows 包：解压后 494 MB，zip 197 MB，其中 Electron 本体 214 MB、Python 134 MB。试包通过，包里 6192 个 Python 文件运行后无一改动；故意去掉 `-B` 并删掉一个字节码文件时，试包会指出被写入的文件。
- 用独立的用户数据目录、空的 Columbina 主目录和空钥匙串打开打包好的 `Columbina.exe`：开场视频从包里加载（窗口可见时正常播放），点最近的项目后启动的正是包里的 `python.exe -I -B -m columbina serve`，项目页显示服务空闲；关窗后包里的进程全部退出。
- 浏览器里模拟 macOS 的标题栏：红绿灯占 17 到 69 像素，品牌从 84 像素开始，与红绿灯垂直居中对齐。
