# Columbina 第 3 阶段实施计划：文件与命令工具、权限与审批、系统提示、指令文件

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 `columbina run` 能在一个小仓库里读文件、改文件、跑命令验证，并且每一个有副作用的动作都经过权限检查。

**Architecture:** 文件工具经过一个文件系统接缝。接缝先判断一次访问能不能做，通过后发一张许可，文件只能经许可打开和替换。判断分两类：限制只能拒绝，每一条都要问到；放宽另行登记。命令工具经过子进程运行器和沙箱接缝。工具管线新增一层守卫，权限规则和审批放在守卫里，别的监听器绕不过去。系统提示由段落注册表拼出，会变的事实和指令文件作为用户角色的上下文消息在回合开始时写入日志。

**Tech Stack:** 与第 2 阶段相同，不增加运行依赖。Windows 上的命令工具用系统自带的 Windows PowerShell 5.1，其他平台用 bash。

设计依据是 `docs/plans/2026-10-09-columbina-design.md` 的第 8.4、8.5、9、10.1 到 10.3 节。本计划与这些章节不一致的地方列在"与设计文档的差异"一节，阶段结束时把设计文档改成一致。

本计划在任务 1 完成后请 Codex 复查过一次，共 12 条意见。处理结果列在文末的"计划复查记录"里，正文已经按结果改过。

本阶段已经完成。完成之后另有一份记录：各任务的提交、重新测量的测试数和覆盖率、复查的汇总、变异检查的结果和已知的限制。记录由作者留在本地，不在仓库里。实现与计划不同的地方记在本文的"实施中的调整"一节。

---

## 结束标志

无界面模式能读、改、测一个小仓库。具体是三条。

1. 用 DeepSeek Flash 运行 `columbina run`，任务是修好一个带失败检查脚本的小仓库。运行结束后，在测试里重新执行检查脚本，它通过。
2. 同一流程有脚本化测试，不需要密钥。那次真实运行的会话存为夹具，用回放适配器重跑，日志与录制的逐事件相同。
3. 默认的权限预设下，没有允许规则时命令工具不执行，模型读到的结果说明原因。

## 范围

本阶段做的：`write`、`edit`、`glob`、`grep`、命令工具，文件系统接缝，沙箱模式，审批，权限规则，子进程运行器，沙箱接缝和 `none` 提供者，系统提示服务，上下文消息（运行时上下文、指令文件、打断说明），`base` 清单，命令行的 `--permission` 和 `--allow`。

本阶段不做的：`todo_write`、`skill`、`subagent`、`ask_user_question`、`apply_patch`、`web_fetch`，结果级的溢出存盘，ripgrep 后端，容器沙箱，交互式的审批界面，系统提示里的变量，带登记的后台进程。它们各自属于后面的阶段。

## 约定

- 仓库根目录是 `F:\agent harness\agent-harness`。分支是 `stage-3-tools`，结束时快进合并到 `main`。不执行 `git push`。
- 项目在任务 3 完成之后由 Tack 改名为 Columbina。此前的提交里出现的 Tack 是同一个项目。
- 每个任务的顺序是：写测试，看它失败，写实现，看它通过，过闸门，提交。闸门是 `ruff check`、`ruff format --check`、`pyright`、带覆盖率的 `pytest`。
- 本计划给出每个任务的文件、对外接口和要写的测试。实现代码在执行时写，不在计划里预先写全。接口在实现中如有调整，在阶段记录里说明。
- 大约每两小时请 Codex 在只读快照上复查一次。复查在后台进行，等结果期间继续做不受它影响的任务。它的每条意见先核实，属实才修改；能写成测试的先写成会失败的测试。
- 同一时间只跑一个 pytest 会话。多个会话共用临时目录，会互相拖慢并造成假的失败。
- 带反斜杠的脚本和测试文件用编辑工具写，不用 shell 的 heredoc。
- 只在一种平台上运行的代码要标明平台，覆盖率在它能运行的平台上统计。标注的做法在任务 3 里定下来。本机只有 Windows，POSIX 一侧的代码要等 CI 第一次运行才得到验证，所以这一侧写得尽量短。

## 设计决定

下面十四条是本阶段要定下来的事。每条给出决定和理由。

**1. 权限检查放在守卫里，并且与执行绑在一起。** 工具管线在 `tools/pre-execute` 之后新增守卫。守卫是登记的函数，只能返回拒绝，没有"允许"这个返回值。管线在 waterfall 放行之后按登记顺序问守卫，问到第一个拒绝为止。权限规则和审批放在一个守卫里。检查紧挨着执行：循环先等到这次调用可以运行，再问检查，问完立刻执行。执行的必须是检查过的那一次调用：`tools/execute` 的监听器换了别的调用往下传，或者有监听器就地改了参数，工具都不运行。管线在调用开始时留一份参数的副本，用来比对。调用开始之后工具被卸载的，这次调用不再执行。工具拿到的 `ToolRun` 是只读的，里面带着检查时所说的 `target`。理由：waterfall 的监听器可以不调用 `next` 就返回放行，排在外层的监听器因此能跳过内层。守卫不在这条链上，谁也跳不过它。检查和执行之间隔得越久，检查时依据的状态就越可能已经变了。

**2. 工具自己说明它动什么。** `Tool` 增加两个字段。`effect` 取 `none`、`read`、`write`、`run`，说明一次调用对外界做什么。`target` 是一个函数，返回规则可以匹配的文本：文件工具返回定位之后的路径，命令工具返回命令。理由：规则的写法是 `工具名(匹配串)`，匹配串对的是什么，只有工具自己知道。

**3. 文件访问默认只许读工作目录，其余靠登记的放宽。** 接缝自己的规则只有一条：读工作目录之内的文件可以。写任何地方、读工作目录之外，都要有一条登记的放宽说可以才行。沙箱模式就是这样的放宽：`read-only` 不放宽任何东西；`workspace-write` 放宽工作目录之内的写，`.git` 和 `.columbina` 目录除外；`danger-full-access` 全部放宽。路径先解析符号链接和目录联接再判断。理由：这样没有沙箱插件时什么也写不了，少装一个插件只会更严，不会更松。工作目录之外的读在本阶段不提供"询问后放行"，等交互界面有了再加。

**4. 限制只能拒绝，放宽绕不过它。** 除了放宽，接缝还有登记的限制。限制对一次访问返回拒绝或者不表态。接缝按登记顺序逐条问，问到第一条拒绝为止。能让一条限制不被问到的，只有排在它前面的拒绝。放宽压不过限制。权限规则里针对文件的拒绝就登记成限制。理由：与守卫同理。如果限制和放宽排在同一条 waterfall 上，一个放行工作目录之外某个目录的插件，就会顺带跳过排在它里面的限制。

**5. 许可与实际的读写绑在一起，先读后写在替换的那一刻比对。** `Fs.permit()` 判断通过后发一张许可，文件只能经许可打开或替换。打开之后按句柄取文件的最终路径，与判断时的路径不同就拒绝。替换经上级目录的句柄完成：POSIX 上用 `dir_fd`，Windows 上在操作期间持有不允许删除共享的目录句柄。文件工具运行时再定位一次路径，落点与检查时所说的 `target` 不同就不读也不写。读的结果送到模型手里之后，接缝记下文件的指纹，即大小和修改时间；结果被别的插件扣下的不算看过。替换之前比对：已有的文件没给模型看过就拒绝，看过之后变了也拒绝并提示重读。Agent 自己写入后更新指纹。新建文件不需要先读。记录按会话存在内存里，恢复会话后要重读。理由：先读后写取自设计第 8.4 节，做法取自 DeepSeek Harness。判断和打开之间，目录可以被换成指向工作目录之外的联接或符号链接；命令被沙箱关住以后，这是从沙箱里影响宿主文件的一条路，所以读写要落在判断过的那个位置上。

**6. 审批只有一次性授权。** `approval.ask()` 的结果是封闭集合：`allowed-once`、`rejected`、`cancelled`、`unavailable`。审批策略为 `never` 时不问任何人，直接 `rejected`。没有应答者、应答者出错或答非所问，都算 `unavailable`。只有 `allowed-once` 放行。理由：设计第 9.5 节。无界面模式没有应答者，所以需要审批的动作在那里一律不执行。

**7. 规则的来源有三处，项目文件不能放宽。** 用户目录的 `<COLUMBINA_HOME>/permissions.yml` 可以写 `deny`、`ask`、`allow`。项目的 `.columbina/permissions.yml` 只认 `deny` 和 `ask`，里面的 `allow` 被忽略并记一条警告。命令行的 `--allow` 是本次运行的允许规则。求值顺序固定为 `deny`、`ask`、`allow`，首个命中生效。理由：克隆下来的仓库不应该能替用户批准命令。

**8. 文件规则在文件被碰到的地方生效。** 针对文件的规则在两处求值。守卫处对的是调用点名的路径，三种规则都算，命中 `ask` 就走审批。接缝处对的是工具实际碰到的每一个文件：命中 `deny` 的拒绝；搜索途中遇到的文件命中 `ask` 的跳过，不为它单独询问。两处匹配的都是定位之后的同一种路径写法。`glob` 和 `grep` 在结果里说明跳过了多少个文件。理由：`grep` 的调用只点名一个起点目录，守卫看不到它一路读了哪些文件。拒绝规则要挡得住，只能在接缝上逐个文件判断。

**9. 没有规则命中时的默认。** `none` 和 `read` 放行。`write` 放行，范围由接缝把关。`run` 在 `danger-full-access` 模式下放行，其余情况要审批。理由：本阶段唯一的沙箱提供者强度为 `none`，命令无法被约束，设计第 9.3 节要求这时一律审批或命中允许规则。"沙箱强度为 `full` 时免审批"留到第 7 阶段，和真实的提供者一起做，那时要把放行的依据和实际准备好的约束绑在一起。

**10. 命令的允许规则只对看得懂的命令生效。** 拆分器只认一种简单的形状：若干条简单命令，用 `&&`、`||`、`;`、`|` 或换行连接，每条简单命令由普通的词和不含展开的引号串组成。命令里出现这个形状之外的写法，拆分器就不拆，表示这条命令它看不懂。这样的写法包括变量和命令替换、反引号、圆括号、花括号、重定向、单个 `&`、here-string、转义符和没有闭合的引号。看不懂的命令，允许规则对它不生效，结果至少是询问。拒绝规则和询问规则照样按整条文本匹配。看得懂的命令逐段判断，取最严格的结果。bash 和 PowerShell 各有一个拆分器，由命令工具登记给权限服务。理由：允许规则按文本放行，`powershell(Write-Output *)` 不应该放行 `Write-Output "$(Get-Content 某个文件)"`。命令规则仍然不是安全边界，文档照旧写明，但放行的范围必须是拆分器确实看懂的那部分。

**11. Windows 上的命令工具叫 `powershell`。** 这台机器和多数 Windows 机器只有 Windows PowerShell 5.1，没有 PowerShell 7。工具名在同一平台上要稳定，规则才写得下去。工具描述写明版本和它与 bash 的主要差别。其他平台的工具叫 `bash`。命令工具依赖沙箱服务和权限服务，少了任何一个它就不加载。

**12. 一条命令启动的进程不比它活得久。** 命令正常结束、超时、调用方被取消，三种情况下它启动的进程都被停下。Windows 上进程一创建就在一个作业对象里，POSIX 上在新的会话和进程组里。进程是否退出由事件循环的退出通知判断，不依赖管道关闭。进程退出后管道最多再读 2 秒。理由：没有登记的后台进程会占着端口和文件，在 Windows 上还会让后面的清理失败。基准要连续跑很多任务，不能留尾巴。想留一个进程给后面的命令用，要等带登记的后台运行，那是以后的事。

**13. `grep` 的正则在另一个进程里匹配。** 遍历、权限判断和读文件都在主进程里做，文件内容经管道交给一个只做匹配的子进程，时限到了或调用被取消就杀掉它。理由：Python 的 `re` 在匹配时不放开 GIL，一条回溯很久的正则会让整个事件循环停住，时限和取消都生效不了。把每行截短限制不了回溯。

**14. 上下文消息是一种机制，来源分三种。** 指令文件、运行时上下文和打断说明都是用户角色的消息，由登记的来源在回合开始时给出文本。来源声明自己属于哪一种。`replace` 和 `append` 是常驻的文本，与日志里这个来源上一次的文本不同才写，前者顶替旧消息，后者追加。`note` 是对一件事的说明，来源每给一次就写一次。上一回合以 `aborted` 或 `interrupted` 结束时，打断说明这个来源给出一条 `note`。来源出错时，这一回合照常认领输入，然后以 `error` 结束，不向模型发请求。理由：三者的写入时机相同，分三处实现会各有各的边角。打断说明的文本每次都一样，按文本去重会吞掉第二次打断。来源出错时如果不认领输入，输入留在收件箱里，循环会立刻再开一个回合，没完没了。第 2 阶段的请求快照显示，被打断的回复后面直接跟着新的用户消息，模型看不出发生过什么，Codex 和 Claude Code 都写这样一条说明。

## 与设计文档的差异

1. 子进程模块叫 `columbina.process`，不叫 `columbina.subprocess`，避免与标准库同名。
2. Windows 上的命令工具叫 `powershell`，不叫 `pwsh`。
3. `grep` 和 `glob` 本阶段只有纯 Python 实现。ripgrep 后端以后作为优化加入。`grep` 的匹配在子进程里做。
4. 审批策略的状态放在 `columbina.approval`，沙箱模式的状态放在 `columbina.sandbox`，预设和规则放在 `columbina.permissions`。设计里没有写明归属。
5. 系统提示本阶段没有变量，只有段落。
6. 项目级规则文件里的 `allow` 不生效。设计里没有这条限制。
7. 新增打断说明。
8. 文件系统接缝没有意图事件。访问由登记的限制和放宽判断，文件经许可打开和替换。先读后写是替换时的比对，不是事件的监听器。
9. 文件工具在 `workspace-write` 模式下不写系统临时目录。文件工具读不回那里的文件，测试用的工作目录本身也在临时目录里。命令需要的临时空间属于第 7 阶段的沙箱配置。
10. 命令正常结束时，它留下的进程也被停下。设计只写了超时和取消。
11. 规则不实现为 `tools/pre-execute` 的监听器，实现为守卫和接缝上的限制。
12. `tools/execute` 的监听器不能换掉要执行的调用。
13. "沙箱强度为 `full` 时免审批"推迟到第 7 阶段。

## 模块与接口

### `columbina.tools`（修改）

```python
Effect = Literal["none", "read", "write", "run"]

@dataclass(frozen=True)
class Tool(Generic[ArgsT, ValueT]):
    ...
    effect: Effect = "none"
    target: Callable[[ArgsT, ToolRun], str] | None = None

Guard = Callable[[Context, Invocation], Awaitable[Deny | None]]

class Tools:
    guards: Registry[Guard]        # 按登记顺序运行，第一个拒绝生效
```

守卫拿到的上下文是发起这次调用的那一个，也就是 Agent 的上下文，审批的询问经它分发。守卫抛异常或返回别的东西时，调用以 `CHECK_FAILED` 结束，不执行。守卫经 `cancellation.guard` 等待。`target` 抛异常时调用以 `TOOL_ERROR` 结束。

`PendingCall` 在问检查之前和 `tools/execute` 链的最后一环各核对一次：这个工具还登记在工具服务里。最后一环之后紧接着就是工具体，所以监听器等待期间卸载的工具也拦得住。不在了就以 `UNKNOWN_TOOL` 结束。链的最后一环只执行管线交给它的那一个 `Invocation`，并且参数要与调用开始时留下的副本相等，否则调用以 `CHECK_FAILED` 结束。参数在问守卫之前也比对一次，守卫判断的就是模型写的参数。

`ToolRun` 的 `call_id`、`cwd`、`session`、`target` 都是只读属性。`target` 由管线在调用开始时填入，没有 `target` 函数的工具得到 `None`。

### `columbina.fs`（新建，插件，提供 `FS`）

```python
@dataclass(frozen=True, slots=True)
class Located:
    path: Path          # 绝对路径，符号链接和目录联接已解析
    written: str        # 模型写的原样
    inside: bool        # 是否在工作目录之内
    relative: str       # 在内时是相对工作目录的 POSIX 路径，在外时是绝对的 POSIX 路径

def locate(cwd: Path, written: str) -> Located      # 不是普通路径的写法抛 ToolFailure，错误码 INVALID_PATH

@dataclass(frozen=True, slots=True)
class Access:
    session: Session
    cwd: Path
    where: Located
    kind: Literal["read", "write"]
    tool: str           # 发起访问的工具名
    named: bool = True  # 调用点名的路径为真，搜索途中遇到的为假

@dataclass(frozen=True, slots=True)
class Refuse:
    reason: str
    code: str

Grant = Callable[[Access], bool]                    # 放宽：说可以，或者不表态
Restriction = Callable[[Access], Refuse | None]     # 限制：拒绝，或者不表态

@dataclass(frozen=True, slots=True)
class Fingerprint:
    size: int
    modified_ns: int

class Opened:                                       # 上下文管理器，离开时关闭文件
    file: BinaryIO
    fingerprint: Fingerprint                        # 打开那一刻的指纹

class Permit:                                       # 只由 Fs.permit 发放
    access: Access
    def open(self) -> Opened                        # 阻塞，在工作线程里调用。只用于读的许可
    def replace(self, data: bytes) -> None          # 阻塞，在工作线程里调用。只用于写的许可

class Fs:
    grants: Registry[Grant]
    restrictions: Registry[Restriction]
    def judge(self, access: Access) -> Refuse | None    # 不抛异常，给遍历用
    def permit(self, access: Access) -> Permit          # 被拒时抛 ToolFailure
    def shown(self, permit: Permit, fingerprint: Fingerprint) -> None   # 模型看到了这个版本
```

判断的顺序：先逐条问限制，问到第一条拒绝为止；再看接缝自己的规则，读工作目录之内的放行；再问放宽，有一条说可以就放行；都没有就拒绝。最后这一步的错误码，路径在工作目录之外是 `OUTSIDE_WORKING_DIRECTORY`，在内而不可写是 `NOT_WRITABLE`。限制和放宽都是不等待的普通函数，只读内存里的状态，所以遍历可以对每个文件都问一遍。限制出错算拒绝，放宽出错算没有放宽，两者都记日志。

`locate` 拒绝的写法：空串，含 NUL，Windows 上盘符之后还有冒号的（备用数据流），以点或空格结尾的名字，Windows 的设备名，以两个斜杠开头的路径（另一台机器或设备）。Windows 上路径里的 `..` 先按文本折叠，再逐级查看链接的目标，目标指向另一台机器或设备命名空间的不跟随。在不在工作目录之内，用系统给出的规范写法逐字比较，不先转小写。Windows 上存在的名字取磁盘上的大小写；其他平台名字保持写的样子，比较名字的地方要自己处理大小写。

`Permit.open` 打开后按句柄取最终路径，与 `where.path` 不是同一个位置就关闭并抛 `ToolFailure`，错误码 `MOVED`。两种写法是不是同一个位置，在写法不同的时候逐级向磁盘确认：每一级都是同一个目录项才算。链接和它指向的东西是两个目录项。`Permit.replace` 先打开上级目录并同样核对，再比对指纹，然后在这个目录里写临时文件并改名，最后记下新指纹。缺少的上级目录逐级创建，每一级都核对。比对不过的错误码是 `NOT_READ` 和 `CHANGED_SINCE_READ`。已有文件的权限位保留。

### `columbina.sandbox`（新建，插件，提供 `SANDBOX`）

```python
Mode = Literal["read-only", "workspace-write", "danger-full-access"]
Strength = Literal["none", "partial", "full"]

class SandboxModeEvent(EventData):          # "sandbox/mode"，可跳过
    mode: Mode

class SandboxProvider(Protocol):
    strength: Strength
    def confine(self, argv: Sequence[str], *, mode: Mode, cwd: Path) -> Sequence[str]: ...

@dataclass(frozen=True, slots=True)
class Confined:
    argv: tuple[str, ...]
    strength: Strength

class Sandbox:
    providers: Registry[SandboxProvider]
    def mode(self, session: Session) -> Mode
    def set_mode(self, session: Session, mode: Mode) -> None
    def confine(self, argv: Sequence[str], *, session: Session, cwd: Path) -> Confined
    def strength(self) -> Strength           # 当前提供者声明的强度
```

配置项 `mode` 是新会话的默认值，`provider` 选提供者，默认 `none`。没有可用提供者时 `confine` 抛 `SandboxUnavailableError`。`none` 提供者原样返回命令，强度为 `none`。强度是提供者声明的，`confine` 做不到就抛错，不返回一个更弱的结果。

本模块向 `FS` 登记一条放宽和一条限制，两者出自同一张表，执行设计决定第 3 条。限制的作用是给出讲得清原因的拒绝：只读模式下的写是 `READ_ONLY`，受保护目录是 `PROTECTED`。

### `columbina.approval`（新建，插件，提供 `APPROVAL`）

```python
Policy = Literal["ask", "never"]
Outcome = Literal["allowed-once", "rejected", "cancelled", "unavailable"]

class ApprovalPolicyEvent(EventData):       # "approval/policy"，可跳过
    policy: Policy
class ApprovalAskedEvent(EventData):        # "approval/asked"，可跳过
    call_id: str
    tool: str
    summary: str
class ApprovalDecidedEvent(EventData):      # "approval/decided"，可跳过
    call_id: str
    outcome: Outcome

@dataclass(frozen=True, slots=True)
class ApprovalRequest:
    session: Session
    call_id: str
    tool: str
    summary: str        # 给人看的一行：工具名和它动的东西
    reason: str         # 为什么要问

APPROVAL_ASK: SerialEvent[ApprovalRequest, Outcome]     # "approval/ask"

class Approval:
    def policy(self, session: Session) -> Policy
    def set_policy(self, session: Session, policy: Policy) -> None
    async def ask(self, ctx: Context, request: ApprovalRequest) -> Outcome
```

### `columbina.permissions`（新建，插件，提供 `PERMISSIONS`）

```python
@dataclass(frozen=True, slots=True)
class Rule:
    tool: str               # 可带 * 通配
    pattern: str | None     # 括号里的匹配串，没有括号时为 None

def parse_rule(text: str) -> Rule
def matches(rule: Rule, tool: Tool, target: str | None) -> bool

Splitter = Callable[[str], tuple[str, ...] | None]      # 看不懂时返回 None
def split_bash(command: str) -> tuple[str, ...] | None
def split_powershell(command: str) -> tuple[str, ...] | None

class Permissions:
    splitters: Registry[Splitter]       # 按工具名登记，命令工具登记自己的

class Config(BaseModel):
    preset: Literal["read-only", "workspace-write", "full-access"] = "workspace-write"
    user_file: Path | None = None       # 清单里写 ${COLUMBINA_HOME}/permissions.yml
    deny: tuple[str, ...] = ()
    ask: tuple[str, ...] = ()
    allow: tuple[str, ...] = ()
```

匹配规则：`effect` 为 `run` 的工具，`*` 匹配任意字符。其他工具把匹配串当作路径模式，`*` 不跨目录，`**` 跨目录，Windows 上不分大小写。通配匹配自己逐字符做，不经过 `re`，最坏情况是模式长度乘文本长度。

命令的求值按设计决定第 10 条。`run` 工具没有登记拆分器时，带匹配串的允许规则对它不生效。

插件加载时做四件事。登记守卫。向 `FS` 登记一条限制，执行设计决定第 8 条。在每个会话的第一步之前把预设对应的沙箱模式和审批策略写进日志，日志里已经有就不写。在同一个时机读入这个工作目录的项目规则文件；规则还没读入就被问到时，限制和守卫都拒绝。

守卫的错误码：规则拒绝是 `DENIED`，审批被拒是 `APPROVAL_REJECTED`，没有人能审批是 `APPROVAL_UNAVAILABLE`，审批被取消是 `APPROVAL_CANCELLED`。

### `columbina.process`（新建，不是插件）

```python
@dataclass(frozen=True, slots=True)
class Completed:
    exit_code: int | None       # 超时被停下时为 None
    timed_out: bool
    output: str                 # 标准输出和标准错误合在一起，按写入顺序
    omitted_bytes: int          # 没有出现在 output 里的字节数

async def run(
    argv: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    stdin: bytes | None = None,
    timeout_seconds: float,
    max_output_bytes: int = 64000,
) -> Completed

def cleaned(env: Mapping[str, str]) -> dict[str, str]    # 去掉名字里带 KEY、PASSWORD、SECRET、TOKEN 的变量
```

超出上限时保留头尾各一半，中间插入一行说明丢弃了多少字节，截断处不留半个字符。`run` 返回或抛出时，这条命令启动的进程都已经被停下，做法见设计决定第 12 条。停不下来的进程不会让 `run` 一直等：等它退出有上限，读管道也有上限。

### `columbina.builtin_tools`（新建五个工具）

| 工具 | 参数 | `effect` | 可并行 |
|---|---|---|---|
| `write` | `path`、`content` | `write` | 否 |
| `edit` | `path`、`old`、`new`、`replace_all=false` | `write` | 否 |
| `glob` | `pattern`、`path="."` | `read` | 是 |
| `grep` | `pattern`、`path="."`、`glob=null`、`ignore_case=false` | `read` | 是 |
| `bash` 或 `powershell` | `command`、`timeout_seconds=null` | `run` | 否 |

文件工具的 `target` 返回 `locate` 给出的 `relative`，与接缝上匹配规则用的写法相同。

`edit` 的错误码：`NOT_FOUND`、`NOT_TEXT`、`NO_CHANGE`（`old` 与 `new` 相同）、`NO_MATCH`、`AMBIGUOUS`（多处匹配又没给 `replace_all`）。文件用 CRLF 换行而 `old` 用 LF 时，按文件的换行再匹配一次，写回时保持文件原来的换行。

`glob` 按修改时间从新到旧，最多 100 条。`grep` 最多 250 条，每条是 `路径:行号:内容`。两者都跳过 `.git` 目录，不跟随符号链接和目录联接，对扫描的文件数和字节数设上限，超限时在结果里说明。遍历一次扫一个目录，扫描在工作线程里做，判断在事件循环里做。`grep` 跳过不像文本的文件，读文件经许可，匹配在子进程里做。子进程只用标准库，用 `-I -S` 启动，从标准输入收模式和文件内容，往标准输出写命中的行。正则写错时子进程报告，工具返回 `INVALID_PATTERN`。搜索超过时限返回 `SEARCH_TIMEOUT`。

命令工具的结果是输出加一行状态。状态行写退出码，或者写超时后被停下。命令跑完但退出码非零不算工具出错，起不来才算。工具描述写明：每次调用是一个新进程，它启动的进程在它结束时一并结束。

### `columbina.system_prompt`（新建，插件，提供 `SYSTEM_PROMPT`）

```python
@dataclass(frozen=True, slots=True)
class Section:
    text: str
    order: int = 0

SYSTEM_PROMPT_ASSEMBLE: WaterfallEvent[Draft, str]      # "system-prompt/assemble"

class SystemPrompt:
    sections: Registry[Section]
    async def render(self, ctx: Context, base: str) -> str
```

`base` 是 `AgentSpec.system_prompt`，排在最前。其后是各段落，按 `order` 升序、再按名字排序，段落之间空一行。循环用 `ctx.get(SYSTEM_PROMPT)` 取这个服务，取不到就直接用 `base`。

### `columbina.agent`（修改）：上下文消息

```python
@dataclass(frozen=True, slots=True)
class ContextSource:
    render: Callable[[Agent], Awaitable[str | None]]
    order: int = 0
    kind: Literal["replace", "append", "note"] = "append"

class Agents:
    context: Registry[ContextSource]
```

循环在每个回合的第一步之前，按 `order` 逐个询问来源，全部问完再写。写的规则见设计决定第 14 条。写下的消息是 `user/message`，`source` 是 `context/<名字>`，排在这一回合的输入之前。来源返回 `None` 时不写。有来源抛异常时，一条上下文消息也不写，输入照常认领，回合以 `error` 结束。

三个来源：`instructions`（`order` 0，`replace`）、`runtime`（`order` 100，`append`）、`interruption`（`order` 200，`note`）。

### `columbina.context.instructions`（新建，插件）

配置项：`names`（默认 `AGENTS.md`、`CLAUDE.md`）、`user_file`、`max_bytes`（默认 65536）。文本由宽到窄排列：先用户级文件，再从项目根到工作目录的链。项目根是最近的含 `.git` 的上级目录，找不到时只看工作目录。超预算时先丢更宽的文件，最后才截断最具体的文件，并在文本里说明。文件内容里出现包裹标签的字面量时转义。

## 任务

每个任务一个或几个提交。任务按执行顺序编号，表后的各节给出文件和测试要点。复查之后任务重新编过号，任务 1 不变。

| # | 任务 | 依赖 |
|---|---|---|
| 1 | 工具的 `effect`、`target` 和守卫（已完成） | |
| 2 | 检查与执行绑定 | 1 |
| 3 | 子进程运行器 | |
| 4 | 文件系统接缝：定位、判断、许可、受约束的打开 | 1 |
| 5 | 受约束的替换与先读后写 | 4 |
| 6 | 沙箱模式与沙箱接缝 | 4 |
| 7 | `write` 和 `edit` | 5、6 |
| 8 | 遍历与 `glob` | 4 |
| 9 | `grep` 与匹配进程 | 3、8 |
| 10 | 审批 | |
| 11 | 权限规则、守卫与接缝上的限制 | 2、4、6、10 |
| 12 | 命令工具 | 3、6、11 |
| 13 | 系统提示 | |
| 14 | 上下文消息与打断说明 | |
| 15 | 运行时上下文的事实 | 6、10、12、14 |
| 16 | 指令文件 | 14 |
| 17 | `base` 清单与命令行参数 | 1 到 16 |
| 18 | 端到端：脚本化、真实运行、回放 | 17 |
| 19 | 文档同步与阶段记录 | 18 |

### Task 1: 工具的 `effect`、`target` 和守卫

**Files:** 修改 `src/columbina/tools/tool.py`、`src/columbina/tools/pipeline.py`、`src/columbina/tools/__init__.py`。测试 `tests/tools/test_tool.py`、`tests/tools/test_pipeline.py`。

**测试要点。** 守卫的拒绝使调用得到错误结果，工具不执行。waterfall 的监听器不调用 `next` 就放行时，守卫仍然运行。几个守卫按登记顺序运行，第一个拒绝之后的不再运行。守卫抛异常、返回别的东西、自己抛出取消，都以 `CHECK_FAILED` 结束。贡献守卫的插件卸载后守卫消失。`effect` 的默认值是 `none`。

### Task 2: 检查与执行绑定

**Files:** 修改 `src/columbina/tools/pipeline.py`、`src/columbina/agent_loop/loop.py`。测试 `tests/tools/test_pipeline.py`、`tests/agent_loop/test_tools.py`。变异体的锚点随之更新。

**测试要点。** `tools/execute` 的监听器把另一个 `Invocation` 往下传时，工具不运行，结果是 `CHECK_FAILED`。监听器照原样往下传、包住执行、自己给出结果，三种都照常。调用开始之后、检查之前工具被卸载，调用以 `UNKNOWN_TOOL` 结束，守卫不被问到。检查之后、执行之前工具被卸载，同样不执行。循环里，一个独占的调用要等前面的调用都跑完才问它的检查；检查通过之后到工具开始运行之间，循环不等待别的东西。第 2 阶段的循环测试和性质测试全部保持通过。

### Task 3: 子进程运行器

**Files:** 新建 `src/columbina/process.py`。测试 `tests/test_process.py`。覆盖率的平台标注在这个任务里定下来，涉及 `pyproject.toml`、闸门脚本和 CI 的工作流。

**测试要点。** 用 `sys.executable -c` 当子进程，不依赖任何外部命令。退出码原样给出。标准输出和标准错误按写入顺序合并。输出超限时保留头尾并说明丢弃的字节数，恰好等于上限时不丢，截断处不留半个字符。超时后进程和它启动的进程都不在了。调用方被取消后同样。进程正常结束后，它留下的进程也不在了，哪怕留下的进程占着管道。留下的进程停不掉时，读管道到时限为止，调用照常返回；这一条在测试里把停进程的函数换成空的来模拟。进程自己停不掉时，等它退出也有上限。环境变量的清洗。输出不是 UTF-8 时用替换字符。给了输入的进程读得到输入，不读输入的进程不会卡住。

### Task 4: 文件系统接缝

**Files:** 新建 `src/columbina/fs/__init__.py`、`src/columbina/fs/bound.py`。修改 `src/columbina/builtin_tools/read.py`、`src/columbina/bundles/minimal.yml`。测试 `tests/fs/test_locate.py`、`tests/fs/test_fs.py`、`tests/fs/test_bound.py`，修改 `tests/builtin_tools/test_read.py`。

**测试要点。** `locate` 对相对路径、绝对路径、`..`、指向工作目录之外的符号链接和目录联接给出正确的 `inside`。Windows 上大小写不同的同一路径、短文件名、末尾带点或空格的写法，都定位到同一个文件。备用数据流、设备名和设备路径被拒绝。没有任何登记时，读工作目录之内放行，其余全部拒绝，错误码各自正确。放宽可以放行之外的读和之内的写。限制可以拒绝之内的读。限制拒绝的访问，放宽救不回来。排在前面的放宽说了可以，后面的限制仍然被问到。限制出错算拒绝，放宽出错算没有放宽。经许可打开的文件就是判断过的那个：许可发出之后把上级目录换成指向外面的联接，`open` 抛 `MOVED`，外面的文件没有被读到。`read` 经接缝读文件，读成功后接缝记下指纹。`read` 原有的测试全部保持通过。

### Task 5: 受约束的替换与先读后写

**Files:** 修改 `src/columbina/fs/__init__.py`、`src/columbina/fs/bound.py`。测试 `tests/fs/test_replace.py`。

**先做试验。** 在这台机器上确认：持有一个不允许删除共享的目录句柄时，这个目录和它的各级上级目录都不能被改名或删除。结论写进模块的说明。试验不成立时，改为写完之后按句柄核对临时文件的位置。

**测试要点。** 没给模型看过的已有文件不能替换，错误码 `NOT_READ`。看过之后可以。看过之后文件被别的进程改了，拒绝，错误码 `CHANGED_SINCE_READ`。Agent 自己替换过之后可以接着再替换。新建文件不需要先读，缺少的上级目录被创建。两个会话的记录互不相干。替换是原子的：写到一半出错时原文件不变，目录里不留临时文件。许可发出之后把上级目录换成指向外面的联接，`replace` 抛 `MOVED`，外面没有多出文件。替换期间上级目录换不掉，这一条在 Windows 上直接测，在 POSIX 上测文件落在原来那个目录里。已有文件的权限位保留。目标是目录时报错。

### Task 6: 沙箱模式与沙箱接缝

**Files:** 新建 `src/columbina/sandbox/__init__.py`。测试 `tests/sandbox/test_sandbox.py`。

**测试要点。** 模式默认取配置，`set_mode` 写一条 `sandbox/mode`，恢复的会话读回同一个模式。三种模式下读和写的放行表，逐格测试，包括 `.git` 和 `.columbina`。性质测试：随机的模式、路径和访问种类，接缝的结论与一个照定义写的参照实现一致。只读模式和受保护目录的拒绝各有自己的错误码和说明。没有提供者时 `confine` 抛错。`none` 提供者原样返回命令。没有这个插件时什么也写不了。没有这个插件也能读带 `sandbox/mode` 的会话。

### Task 7: `write` 和 `edit`

**Files:** 新建 `src/columbina/builtin_tools/write.py`、`src/columbina/builtin_tools/edit.py`。测试 `tests/builtin_tools/test_write.py`、`tests/builtin_tools/test_edit.py`。

**测试要点。** `write` 新建文件和缺少的上级目录，覆盖已有文件，内容按原样写入不改换行。`edit` 的五个错误码各一条测试。唯一匹配时只改那一处。`replace_all` 改全部。CRLF 文件用 LF 的 `old` 能匹配，写回后仍是 CRLF。文件不是 UTF-8 时拒绝。两个工具都经过接缝：只读模式下被拒，受保护的目录被拒，没读过的文件被拒。调用被取消时，磁盘上要么是旧内容要么是新内容。

### Task 8: 遍历与 `glob`

**Files:** 新建 `src/columbina/fs/walk.py`、`src/columbina/builtin_tools/glob.py`。测试 `tests/fs/test_walk.py`、`tests/builtin_tools/test_glob.py`。

**测试要点。** 遍历对每个遇到的文件和目录都问接缝，`named` 为假。被拒的目录不进去，被拒的文件不给出，两者都计数。遍历不跟随符号链接和目录联接，跳过 `.git`。扫描的条目数有上限。调用方被取消时遍历停下。`glob` 的 `**`、`*`、`?` 和字符集，Windows 上不分大小写。结果按修改时间从新到旧，超过 100 条时截断并说明。结果里说明跳过了多少个文件。星号很多的模式配上很长的文件名，匹配仍然很快。

### Task 9: `grep` 与匹配进程

**Files:** 新建 `src/columbina/builtin_tools/grep.py`、`src/columbina/builtin_tools/grep_worker.py`。测试 `tests/builtin_tools/test_grep.py`、`tests/builtin_tools/test_grep_worker.py`。

**测试要点。** `grep` 给出路径、行号和内容。`glob` 参数限定文件。`ignore_case` 生效。正则写错时返回 `INVALID_PATTERN`。跳过不像文本的文件。超过 250 条时截断并说明。很长的行只给出开头并说明。命中拒绝规则的文件读不到，这一条用一条登记的限制来测。回溯很久的正则：搜索在时限到达时返回 `SEARCH_TIMEOUT`，其间事件循环照常运转，一个每隔几十毫秒记一次时间的任务没有被卡住。调用被取消时匹配进程不在了。匹配进程单独测：协议的每一种消息，输入中途断掉时它自己退出。

### Task 10: 审批

**Files:** 新建 `src/columbina/approval/__init__.py`。测试 `tests/approval/test_approval.py`。

**测试要点。** 策略为 `never` 时不调用应答者，结果是 `rejected`。没有应答者是 `unavailable`。应答者抛异常、返回集合之外的值，是 `unavailable`。应答者返回四个值之一时原样给出。每次询问写一对事件，`never` 也写。等待期间调用方被取消，取消照常传播，日志里补一条 `cancelled` 的决定。策略写进日志，恢复后读回。

### Task 11: 权限规则、守卫与接缝上的限制

**Files:** 新建 `src/columbina/permissions/__init__.py`、`src/columbina/permissions/rules.py`、`src/columbina/permissions/commands.py`。测试 `tests/permissions/test_rules.py`、`tests/permissions/test_commands.py`、`tests/permissions/test_guard.py`。

**测试要点。** 规则的解析和写错时的报错。路径模式和命令模式的匹配，各一组表格测试。星号很多的模式配上很长的文本，匹配仍然很快。两种 shell 的拆分各一组表格测试：命令替换、变量、反引号、脚本块、圆括号、here-string、转义符、重定向、单个 `&`、没有闭合的引号，都判为看不懂；单引号里的这些字符不算。性质测试两条：由普通的词和五种连接符生成的命令，拆出的各段与生成时的一致；在引号之外插入任何一个特殊字符，结果都是看不懂。看不懂的命令命中允许规则时仍然要审批，命中拒绝规则时被拒。看得懂的命令逐段判断，有一段没被允许就要审批。没有登记拆分器的 `run` 工具，带匹配串的允许规则不生效。求值顺序：同时命中 `deny` 和 `allow` 时拒绝，同时命中 `ask` 和 `allow` 时询问。默认表按 `effect` 和沙箱模式逐格测试。项目文件里的 `allow` 不生效并记警告。预设在第一步之前写进日志，恢复的会话不重复写。守卫的四个错误码。接缝上的限制：命中拒绝规则的文件，任何工具经接缝都碰不到，换一种写法指向同一个文件也一样；搜索途中命中询问规则的文件被拒。规则还没读入时守卫和限制都拒绝。性质测试：随机的规则集和调用，守卫的结论与一个直接照定义写的参照实现一致。

### Task 12: 命令工具

**Files:** 新建 `src/columbina/builtin_tools/shell.py`。测试 `tests/builtin_tools/test_shell.py`。

**先做试验。** 在这台机器上试出 Windows PowerShell 5.1 的调用方式，要满足七条：中文输出不乱码；原生命令的退出码传得出来；出错的 cmdlet 给出非零退出码；标准错误是普通文本，不是 CLIXML；100 KiB 的命令能执行，或者有一个写明的长度上限并在超过时给出清楚的错误；工作目录和脚本路径里带空格时照常；调用被取消之后不留临时文件。候选做法是 `-EncodedCommand`、临时脚本文件加 `-File`、从标准输入读脚本。`-EncodedCommand` 受命令行 32767 个字符的限制，一万多个字符的命令就会超。试验的结论写进模块的说明。

**测试要点。** 命令在工作目录里执行。退出码写在状态行。超时写在状态行，进程被停下。`timeout_seconds` 超过上限时按上限。输出超限时截断。命令起不来时是工具出错。环境里的密钥不传给命令。`target` 返回命令本身。命令启动的进程在命令结束后不在了。工具向权限服务登记自己的拆分器。没有权限插件或沙箱插件时命令工具不加载。权限插件卸载后命令工具跟着卸载。按平台各有一组测试，另一平台的那组跳过。

### Task 13: 系统提示

**Files:** 新建 `src/columbina/system_prompt/__init__.py`。修改 `src/columbina/agent_loop/loop.py`。测试 `tests/system_prompt/test_system_prompt.py`、`tests/agent_loop/test_turns.py`。

**测试要点。** 段落按 `order` 再按名字排序。`system-prompt/assemble` 的监听器可以改写结果。没有这个服务时循环直接用 `AgentSpec.system_prompt`。段落变了，下一回合的 `system/message` 顶替旧的。文本没变就不写新事件。

### Task 14: 上下文消息与打断说明

**Files:** 修改 `src/columbina/agent/agent.py`、`src/columbina/agent_loop/loop.py`。测试 `tests/agent_loop/test_context.py`，更新快照 `tests/agent_loop/snapshots/a_session.txt`。

**测试要点。** 来源的文本在回合开始时写成用户消息，排在这一回合的输入之前。`replace` 和 `append` 的文本没变不重复写，前者顶替，后者追加。`note` 每给一次写一次。来源返回 `None` 不写。来源抛异常时：只有一个以 `error` 结束的回合，没有向模型发请求，输入在日志里而不在收件箱里，`when_idle()` 完成。上一回合被取消后，下一回合的请求里有打断说明。连续两次打断，各有一条说明，重新打开会话之后也一样。上一回合正常结束时没有。性质测试的不变量保持成立：历史只增不改这一条要为 `replace` 来源放宽。

### Task 15: 运行时上下文的事实

**Files:** 修改 `src/columbina/agent_loop/__init__.py`、`src/columbina/sandbox/__init__.py`、`src/columbina/approval/__init__.py`、`src/columbina/builtin_tools/shell.py`。测试 `tests/agent_loop/test_context.py`。

**测试要点。** 快照里有工作目录、沙箱模式、审批策略、沙箱强度和命令工具的种类。模式变了，下一回合追加新快照。没变就不追加。贡献事实的插件卸载后，它的那一行消失。

### Task 16: 指令文件

**Files:** 新建 `src/columbina/context/__init__.py`、`src/columbina/context/instructions.py`。测试 `tests/context/test_instructions.py`。

**测试要点。** 用户级文件在前，项目根到工作目录的链由宽到窄。没有 `.git` 时只看工作目录。内容相同的文件只出现一次。超预算时先丢更宽的文件，最后截断最具体的并说明。包裹标签的字面量被转义。文件变了，下一回合发一条顶替的消息。文件没了，发一条说明。没有任何指令文件时不写消息。文件读不了时按任务 14 的规定结束回合。

### Task 17: `base` 清单与命令行参数

**Files:** 新建 `src/columbina/bundles/base.yml`。修改 `src/columbina/bundles/headless.yml`、`src/columbina/apps/launcher.py`、`src/columbina/apps/headless.py`。测试 `tests/apps/test_launcher.py`、`tests/apps/test_headless.py`、`tests/test_package.py`。

**测试要点。** `--dump-config` 列出 `base` 的全部条目。`--permission` 改 `permissions` 条目的预设，没给时不动。`--allow` 可以给多次。清单里没有 `permissions` 条目而用了这两个参数时报错，退出码 2。每个产品可见的插件有一条经真实清单加载的测试。从 `base` 里去掉权限插件或沙箱插件，命令工具不出现在给模型的工具表里。

### Task 18: 端到端

**Files:** 新建 `tests/apps/test_fix_a_repository.py`、`tests/fixtures/recorded/deepseek-fix/`、`tests/live/test_deepseek_fix.py`。修改 `tests/replay/test_recorded.py`，让它对夹具目录下的每个录制各跑一遍。

**场景。** 小仓库里有 `calc.py`、`check.py` 和 `AGENTS.md`。`calc.py` 有一处错误，`check.py` 调用它并打印 `ok` 或失败原因，输出里不带时间和绝对路径。任务是让 `python check.py` 通过。允许规则只放行这一条命令。

**测试要点。** 脚本化的模型依次读、改、跑命令、回答。测试在运行结束后自己再执行一次 `check.py` 并断言通过。没有允许规则时命令不执行，结果的错误码是 `APPROVAL_UNAVAILABLE`。请求快照记下模型看到的指令文件、运行时上下文和每个工具的结果。真实运行的那条测试没有密钥时跳过。

### Task 19: 文档同步与阶段记录

更新设计文档的第 3.3、8、9、10 节和 README。写 `docs/plans/<日期>-columbina-stage3-record.md`，做法与第 2 阶段的记录相同：任务对应的提交，重新测量的测试数和覆盖率，复查记录，变异检查的结果。

## 测试策略

- 文件工具和权限用真实的临时目录测试，不模拟文件系统。
- 命令工具用 `sys.executable` 当被执行的程序，三个平台都能跑。
- 策略类的代码配性质测试，与照定义写的参照实现对比。本阶段有三处：规则求值，文件访问的放行表，命令的拆分。
- 把目录换成联接的测试在 Windows 上用目录联接，不需要管理员权限；在 POSIX 上用符号链接。
- 循环的性质测试加上工具真的写文件的情形，检查取消之后磁盘上没有半截的文件。
- 新代码照例做变异检查。先加 `permissions`、`process` 和 `fs` 三组变异体。

## 风险

| 事项 | 处理 |
|---|---|
| Windows PowerShell 5.1 的输出编码和退出码行为不直观 | 任务 12 先做试验，把结论写成测试 |
| 作业对象和目录句柄要用 `ctypes` 调系统接口，原型写错不容易看出来 | 每个调用都声明参数和返回类型，返回值逐个检查，失败时进程不留下 |
| 由别的系统服务代为启动的进程不在作业里，停不到 | 读管道和等退出都有上限。容器沙箱在第 7 阶段解决隔离 |
| 命令规则按文本匹配，绕得过去 | 允许规则只对拆分器看得懂的命令生效。文档写明它不是安全边界 |
| POSIX 一侧的代码本机跑不了 | 写得尽量短，用法只取文档写明的；CI 第一次运行后按结果修 |
| 接缝把路径解析到文件上，硬链接分辨不出来 | 文档写明。硬链接要有对目标的访问权才建得出来 |
| 查看链接和打开文件是先后两步，中间链接被换成指向另一台机器的，系统仍会去连接 | 读到的内容有句柄核对把关，连接这件事没有。模块说明里写明。要堵住得逐级持有目录句柄，留到第 7 阶段评估 |
| macOS 上名字保持写的样子，同一个文件可以有几种大小写 | 规则匹配在 Windows 和 macOS 上都不分大小写（任务 11）。句柄核对按目录项比较，不受影响 |
| macOS 上同一个文件换一种大小写写法去写，会被当成没读过 | 结果是多读一次，不会误写。文档写明：读和写用同一种写法。CI 在 macOS 上跑起来之后再评估按目录项识别文件 |
| POSIX 上子进程创建到一半时事件循环被关掉，子进程已经启动的后代可能留下 | 只在进程退出的瞬间出现。写明是已知的限制，容器沙箱在第 7 阶段兜底 |
| 先读后写在改名前一刻的第二次比对与改名之间仍有极短的空档 | 系统没有"比对并替换"的原子调用。新建文件用不覆盖的改名，没有这个空档 |
| 仓库把历史放在别的名字的目录里、用 `.git` 文件指过去时，那个目录不受保护 | 文档写明。按名字和顶层链接的落点保护，覆盖常见情形 |
| 本阶段代码量超过第 2 阶段 | 任务 1 到 12 完成后先合并一次，再做 13 到 19 |

## 实施中的调整

下面几条是实现时定下来的，计划原文没有写到或者写得不同。阶段记录以这里为准。

**任务 8 和 9。**

- 遍历把每个文件连同接缝发的许可一起交出，`grep` 用这张许可打开文件，不再另行申请。
- `grep` 的 `path` 指向一个文件时，这个文件总是被搜索，`glob` 参数只筛选目录下面的文件。
- `grep` 在结果里分别说明三类没搜的文件：太大的、不像文本的、读不了的。上限是每个文件 16 MiB、一次搜索 64 MiB、250 条、每行 200 个字符、20 秒。
- 匹配进程的最后一行带上它搜完的文件数，主进程据此确认没有内容在半路丢失。

**任务 10。** `ApprovalRequest` 增加 `target`，是调用所动的东西的全文。`summary` 是给日志和标题用的一行，长文本会被截短。向人提问的界面应当展示全文，只看开头没法对一条命令说"可以"。

**任务 11。**

- 规则名 `read`、`write`、`run` 指的是所有这样做的工具，不只是同名的那一个。`write(docs/**)` 也管 `edit`，`read(.env)` 也管 `grep` 和 `glob`。其余的名字按工具名匹配，可以带 `*`。理由：只按工具名的话，拒绝 `read(.env)` 之后还能用 `grep` 读到内容。
- 文件模式分三种起点。以 `/`、盘符或 `~` 开头的按完整路径匹配。单独一个名字、或者以 `**` 开头的，在任何深度上找：工作目录之内看工作目录以下的路径，之外看完整路径。其余的从工作目录算起，`./x` 把单独的名字钉在顶层，`../x` 指工作目录旁边。`..` 只和 `..` 匹配，星号不会走出工作目录。
- 模式同时管它点名的东西和下面的一切，所以 `read(secrets)` 管整个目录。
- 预设在 `turn/start` 事件之后写进日志。项目的规则文件在会话的第一次调用被判断时读入，之后不再读。
- 规则读不了时的错误码是 `RULES_UNREADABLE`，还没读入时是 `RULES_NOT_READ`。两种情况下这个会话什么都不做。
- 项目的规则文件只在工作目录之内读，经句柄核对打开，上限 256 KiB。读不了时不复述文件内容，因为它可能是指向别的文件的链接。用户的规则文件读不了时插件不加载，程序因此起不来。
- 拆分器比计划多拒绝几类写法。两种 shell 都拒绝看不见的字符、异形空格和改变文字方向的控制符，因为审批的人得看得见要运行的东西。PowerShell 还拒绝弯引号（它把弯引号当引号，直引号开头的字符串可以被弯引号结束）、`--%`、方括号和 `@`。
- 带匹配串的允许规则要求命令的每一段都被允许。不带匹配串的允许规则放行整个工具，命令看不懂时也放行。

**任务 12。** 试验比较了四种把命令交给 Windows PowerShell 5.1 的办法，用的是计划里的七条要求。

| 办法 | 结果 |
|---|---|
| `-EncodedCommand` | 错误以 CLIXML 输出，加了 `-OutputFormat Text` 也一样。12000 个字符的命令就超出命令行长度 |
| `-Command -`，命令走标准输入 | 多行的语句块不执行。输入按控制台代码页解码 |
| 临时脚本文件加 `-File` | 七条都满足，但要落一个文件，要机器的执行策略允许，错误信息里带着临时文件的路径 |
| 命令行上放一行固定的引导代码，命令走标准输入 | 七条都满足，100 万个字符的命令 0.3 秒跑完，不落文件。采用这一种 |

引导代码先把控制台的输入输出编码设成 UTF-8，再读入并执行命令。退出码的判断要写在命令文本的末尾：在语句块外面看，`$?` 总是成功。原生程序最后失败时用它的退出码，cmdlet 最后失败时用 1。

另外定下几件事。PowerShell 用 `%SystemRoot%` 下的完整路径启动，因为 Windows 找程序时先看本程序启动时所在的目录，而那里常常是一个仓库。命令的环境里设 `PYTHONIOENCODING=utf-8`（用户已经设了就不动），否则 Python 往管道里写的是系统代码页，中文会乱。工作目录的路径超过约 258 个字符时 Windows 起不了子进程，这时工具报错。POSIX 上用 `bash -c`，命令长度上限 120000 字节，超过时返回 `COMMAND_TOO_LONG`。工具名在 Windows 上是 `powershell`，其他平台是 `bash`。

**任务 13。** 循环在每个回合开始时向系统提示服务要一次文本，不再每一步都要。开头保持不变，服务商才能把它缓存下来。文本拼不出来时，回合照常认领输入，然后以 `error` 结束。

**任务 14。**

- 来源登记在 Agent 服务上，每个来源带 `render`、`order` 和 `kind`。
- 每条上下文消息的文本包在 `<context source="名字">` 和 `</context>` 之间。文本里出现的结束标签被拆开。转义由循环统一做，不由各个来源做，所以指令文件的内容不能提前结束这一段。
- 打断说明这个来源登记在 Agent 插件里，不在循环插件里。一个程序里可以装两个循环，同一个名字的来源却只能登记一次。
- 来源全部问完才写。有一个出错时一条也不写。

**任务 15。** 运行时上下文不写在循环里，是一个单独的插件 `columbina.context.runtime`。它自己只知道工作目录，其余的事实由各自的插件给出：它发一个 `agent/runtime-facts` 事件，沙箱、审批和命令工具各加一行。没装的插件不加，卸载的插件带走它的那一行。各行按名字排序，文本不随插件的加载顺序变。计划原来把快照放在循环里，那样循环要认识它不该认识的插件，核心的 34 条测试也都要跟着改。

**任务 16。**

- 指令文件由 Harness 替用户读，不经过文件系统接缝，否则工作目录以上的文件读不到。这个插件因此不能成为绕过沙箱和规则的路。
- 链接一律不跟随，不论指向哪里。计划写的是指向工作目录之外的不读。改严的原因是，仓库里的链接可以指向项目里被拒绝规则挡住的文件，例如 `.env`，而这里不问规则。判断一个位置上是不是链接时不跟随任何东西。模型会被告知哪个文件因为是链接而没读。
- 指向另一个指令文件位置的链接直接跳过，不作说明。仓库把同一份文件放在两个名字下面时就是这样，那个位置上的文件照常读。
- 每个文件最多读预算加一个字节。行尾统一成 `\n`。只有字节序标记的文件算空文件。
- 用 `.git` 找项目根时不跟随链接。
- 文件名的配置在加载时检查。带斜杠的名字，以及 Windows 上会被读成别的名字的写法，都不接受。
- 每个文件前面有一行 `==> 名字 <==`。名字是模型写这个路径时的写法，例如 `../AGENTS.md`。

**任务 17。**

- `base` 建立在 `minimal` 之上，`headless` 建立在 `base` 之上。重试插件从 `headless` 移到 `base`。
- 去掉权限插件或沙箱插件而留下命令工具时，程序不启动，退出码为 2，并说明命令工具在等哪个服务。计划写的是命令工具不出现在工具表里。启动器对加载不了的条目一律报错，这里保持一致。连命令工具一起去掉，其余工具照常可用。
- 命令行给的规则排在清单已有的规则之后。`permissions` 条目被禁用时，`--permission` 和 `--allow` 同样报错。
- 无界面前端的默认系统提示加了两句话。一句要求改动之后检查。另一句说，调用因为不被允许而被拒绝时，不要换个办法再做，要在回答里说明。
- 项目规则文件带 `allow` 时的警告改了措辞，一条规则和多条规则读起来一样。
- `tests/test_package.py` 检查三份清单随包带上，并且清单里每个条目点名的插件都能导入。

**任务 18。**

- 回放测试对夹具目录下的每个录制各跑一遍。每个录制带一个 `recording.yml`，写明当时加载的清单和规则。测试按它装配真实的清单，只把模型换成回放适配器。
- 调用了命令工具的录制只能在录制时的平台上重放，因为工具名就是那个平台的 shell。`recording.yml` 写明平台，别的平台上这条测试跳过。
- 日志里现在有工作目录，它在运行时上下文里。录制时把它换成 `/work`，重放时把实际路径同样换掉再比较。
- 工作目录的副本里，所有文件的修改时间设成同一个时刻。`glob` 按修改时间排序，而检出之后哪个文件最新因机器而异。
- 脚本化测试的请求快照把工作目录和 shell 的名字写成与平台无关的样子，一份快照在三个平台上都能比。
- 场景里的命令设了 `PYTHONDONTWRITEBYTECODE=1`，运行之后工作目录里不多出文件。

**任务 19。**

- 计划只要求同步文档和写阶段记录。实际还把十七组变异检查全部重跑了一遍，385 个变异体里漏掉一个。它在 `loop` 一组：回合在两步之间被取消时，循环仍然取走留给下一步的输入。这个行为因此补了一条确定的测试。经过写在阶段记录的"测试的检出力"一节。
- 变异检查现在让性质测试每次都用同一批样例。`mutate.py` 给测试进程设 `HYPOTHESIS_PROFILE=steady`，`tests/conftest.py` 登记这个配置。平时的测试仍然随机抽样。
- 阶段记录里结束时的指标，是提交之前在工作目录里量的。各任务的那些行仍然量自各自的提交。

## 计划复查记录

任务 1 完成后，Codex 在提交 07308c0 的只读快照上复查了这份计划，共 12 条意见。逐条对照代码核实后，11 条采纳，1 条部分采纳。

| # | 意见 | 核实 | 处理 |
|---|---|---|---|
| 1 | `fs/access` 是 waterfall，外层监听器放行就跳过了沙箱限制和先读后写 | 属实，与守卫要解决的是同一个问题 | 接缝不用事件，改成登记的限制和放宽。设计决定 3、4 |
| 2 | 少装或卸掉策略插件会让限制变成默许，已经开始的调用也不会失效 | 属实 | 写默认不放行；命令工具依赖沙箱和权限服务；调用执行前核对工具还在。设计决定 1、3、11 |
| 3 | 允许规则会放行拆分器没看的可执行写法，例如双引号里的命令替换 | 属实 | 允许规则只对看得懂的命令生效。设计决定 10 |
| 4 | 递归搜索只点名起点，拒绝子目录的规则挡不住 | 属实 | 规则在接缝上对每个文件求值，访问带上工具名。设计决定 8 |
| 5 | 判断和打开之间目录可以被换掉，读写落到别处 | 属实。本阶段命令不受约束，这条路没有多给什么；第 7 阶段起有意义 | 现在就做：许可、按句柄核对、经目录句柄替换。设计决定 5 |
| 6 | 检查过的不一定是执行的：`tools/execute` 能换调用；守卫看的强度和实际的约束可以不同；检查与执行之间有等待 | 第一点和第三点属实。第二点成立，但本阶段没有强度不为 `none` 的提供者 | 执行绑定到检查过的调用，检查挪到紧挨执行。强度免审批推迟到第 7 阶段。设计决定 1、9 |
| 7 | 进程树停不干净：父进程先退出的后代留着；等退出可能被管道拖住 | 属实。Python 3.11 的 `Process.wait()` 要等管道断开 | 作业对象和进程组；命令结束时一并停下；用退出通知判断退出。设计决定 12 |
| 8 | 行截断限制不了正则回溯，匹配占着 GIL，取消不了 | 属实 | 匹配放进可以杀掉的子进程。设计决定 13 |
| 9 | 上下文来源出错时输入没被认领，循环会不停地开出错的回合 | 属实，回合驱动在收件箱非空时立刻再开一个回合 | 出错时照常认领输入再结束。设计决定 14 |
| 10 | 接缝没有调用方的上下文，从插件的上下文分发会让别的 Agent 的监听器听到 | 属实，内核的作用域规则如此 | 接缝不分发事件，访问带着会话。设计决定 3、4 |
| 11 | `-EncodedCommand` 有长度上限，试验没有把它列进去 | 属实 | 长度、带空格的路径和取消后的清理加进试验条件。任务 12 |
| 12 | 按文本去重会吞掉第二次打断说明 | 属实 | 来源分三种，`note` 每次都写。设计决定 14 |

## 代码复查记录

### 第一次（任务 4 之后）

Codex 在提交 064ca4e 的只读快照上复查了任务 1 到 4 的代码，共 12 条意见。逐条核实，能写成测试的都先写成了会失败的测试。10 条采纳，2 条部分采纳。

| # | 意见 | 核实 | 处理 |
|---|---|---|---|
| 1 | Windows 上判断"在不在工作目录之内"用的是先转小写的比较，Python 的小写与磁盘的不一致，旁边的目录会被当成工作目录 | 属实。在 NTFS 上建了 `i̇` 和 `İ` 两个目录验证 | 用系统给出的规范写法逐字比较 |
| 2 | 逐级查看链接时，`..` 按 POSIX 的方式处理，而 Windows 是先按文本折叠再跟随链接，查看的和系统实际跟随的不是同一批链接 | 属实。在磁盘上验证了 `docs/../bait` 打开的是工作目录里的那个 | 按 Windows 的读法重写，原来的测试写反了，一并改正 |
| 3 | 链接目标带 `\\?\` 前缀时只认出了共享，设备命名空间里的其他名字被放过 | 属实 | 前缀后面只有盘符和无盘符的卷算本机磁盘 |
| 4 | 查看链接和打开文件之间链接可以被换掉，系统仍会去连接别的机器 | 属实，是两步之间的竞争 | 部分采纳：内容有句柄核对把关，连接这件事写明是已知的限制 |
| 5 | macOS 上句柄核对按折叠大小写比较，区分大小写的卷上会放过只差大小写的目录 | 成立，本机验证不了 macOS | 写法不同时逐级按目录项比较，Windows 上也用同一套，并有测试 |
| 6 | `tools/execute` 的监听器可以就地改参数或 `run.cwd`，对象还是同一个，绕过了身份核对 | 属实 | 留参数副本，问守卫之前和执行之前各比对一次；`ToolRun` 改为只读 |
| 7 | `read` 在检查时和执行时各解析一次路径，中间链接变了，守卫看的和读的不是同一个文件 | 属实，用目录联接复现 | 管线把检查时的 `target` 交给工具，落点不同就不读 |
| 8 | POSIX 上创建子进程期间被取消，asyncio 只杀直接子进程，它已经启动的后代留着 | 成立。本机用"创建完成得晚"的包装复现了没人收尾的情形 | 创建过程不随调用方取消，完成后由回调收尾 |
| 9 | macOS 上 `realpath` 不恢复磁盘上的大小写，`Located.relative` 不是规范写法 | 成立 | 部分采纳：模块说明改正，规则匹配在 macOS 上不分大小写（任务 11） |
| 10 | `tools/execute` 的监听器等待期间工具被卸载，放行后工具仍然运行 | 属实 | 链的最后一环再核对一次登记 |
| 11 | 读结果被 `tools/post-execute` 扣下时，文件仍被记成给模型看过 | 属实 | 改为在 `tools/result` 上、结果无错时才记 |
| 12 | 文档说"每条限制都要问到"，代码在第一条拒绝处就停了 | 属实，是文档的问题 | 改文档：逐条问，问到第一条拒绝为止 |

### 第二次（任务 9 之后）

Codex 在提交 5b49abf 的只读快照上复查了任务 5 到 9 的代码和第一次复查后的修改，共 18 条意见。逐条核实：能在本机复现的都先复现，再写成会失败的测试，然后才修。15 条采纳，1 条部分采纳，1 条记为已知限制，另有 1 条与第 6 条合并处理。修改分三次提交：965ef68、435c48d 和本次提交。

| # | 意见 | 核实 | 处理 |
|---|---|---|---|
| 1 | `grep` 按路径执行 `grep_worker.py`。工作目录就是本项目源码时，写文件的工具可以改掉它，下一次搜索执行的就是改过的程序 | 属实 | 模块加载时读入匹配程序的源码，以 `-I -S -c` 交给 Python。运行的是加载时的内容 |
| 2 | Windows 上目录里可以有以点结尾的名字，遍历把它交了出去，按普通方式打开时得到的是不带点的那个文件 | 属实。在 NTFS 上建了 `secret` 和 `secret.`，遍历交出的 `secret.` 读出了 `secret` 的内容 | 遍历跳过 `locate()` 会拒绝的名字，计入"不让看"的数目 |
| 3 | 检查和执行之间工作目录本身被换成指向别处的链接时，相对路径不变，写入落到了别处 | 属实，用目录联接复现 | `Located` 带上工作目录的落点。接缝记住会话第一次访问文件时的落点，之后落点变了就拒绝一切访问。工具经接缝定位 |
| 4 | 受保护目录只在工作目录以下的相对路径里找。会话开在 `.git` 里面时可以写；`.git` 是链接时，它指向的目录按自己的名字可以写 | 属实，各写了失败测试 | 改为看整条绝对路径上的名字，并保护工作目录顶层的 `.git` 和 `.columbina` 链接所指向的目录 |
| 5 | 遍历用"目录的写法加斜杠加名字"拼路径。从工作目录之外开始时，工作目录里的文件被写成绝对路径并算作在外面；从盘符根开始时多出一个斜杠 | 属实，在本机复现 | 调用点名的路径和遍历遇到的条目由同一个函数给出写法 |
| 6 | 一次回复里先 `read` 再 `write` 同一个文件，读一完成就算看过，写就通过了，而模型写这次调用时并没有看到内容 | 属实，在循环一层写了失败测试 | 读到的内容从下一步开始才算看过。接缝在每一步开始时把上一步读到的记为看过。Agent 自己写入的仍然立即生效 |
| 7 | 写文件的线程取消不了。调用被取消后线程继续跑，可能把后一次调用写好的内容盖掉 | 属实 | 写文件的调用等线程结束才返回，并通知线程放弃，线程在改名前检查。`edit` 把替换绑定到它读到的那个状态上 |
| 8 | 先比对再准备新内容，准备期间别人改了文件，改名时照样盖掉；原来不存在的文件期间被别人建出来，也会被盖掉 | 属实 | 新内容准备好之后再比对一次。新建文件用不覆盖的改名，名字被占就失败 |
| 9 | 写完之后按名字查文件状态，查到的可能是别人刚写的版本，这个版本于是被记成 Agent 写的 | 属实 | 状态取自写好的临时文件，在改名之前 |
| 10 | `grep` 按目录里记的大小凑批。文件实际更大时，一批可以远超上限 | 属实 | 按实际读到的字节数凑批，没读到的文件留给下一批 |
| 11 | 读结果里一行文件内容都没有，或者被监听器换成别的文字，文件仍被记成看过 | 属实 | 至少给出一行，或者文件为空，并且工具的原文还在模型读到的文字里，才算看过 |
| 12 | 目录有多少条目就全部枚举、取状态、排序，然后才看条目上限 | 属实 | 枚举带上限，只取用得上的条目，目录里还有更多时说明 |
| 13 | `grep` 的时限只在启动匹配进程时检查，遍历和读文件不受限 | 属实 | 整次搜索放在一个截止时间之下，到时即停并返回 `SEARCH_TIMEOUT` |
| 14 | macOS 上同一个文件换一种大小写去写，会被当成没读过 | 成立，本机验证不了 | 部分采纳：结果偏严不偏松，写进风险表，等 CI 在 macOS 上跑起来再处理 |
| 15 | 参数对象带有不能复制的私有属性时，留副本这一步抛异常，调用没有得到正常的出错结果 | 属实 | 留的是参数的取值。取不出来时这次调用以 `TOOL_ERROR` 结束 |
| 16 | POSIX 上子进程创建到一半时事件循环被关掉，后代进程可能留下 | Codex 标为"怀疑"，本机验证不了 | 不改，写进风险表 |
| 17 | `glob` 的匹配按路径深度递归，模式或路径很长时超出递归上限 | 属实 | 改成不递归的写法，放在 `columbina.fs.patterns`，权限规则的路径匹配也用它 |
| 18 | 没有可搜索的文件时匹配进程不启动，写错的正则得不到 `INVALID_PATTERN` | 属实 | 没有文件时也让匹配进程检查一次模式 |

接口上的变化：`Located` 多了 `root`；`Fs` 多了 `locate`、`showing` 和 `delivered`；`Permit.replace` 多了 `made_from` 和 `given_up` 两个参数；`bound.listing` 接受条目上限并返回"是否还有更多"；`columbina.fs` 新增 `carry_out` 和 `patterns` 模块。

Codex 认为没有问题的部分：限制先于放宽的求值顺序、Windows 上备用数据流和设备名和网络路径的拒绝、短文件名的展开、`..` 的处理、子目录被换成链接时的拒绝、POSIX 上经 `dir_fd` 的操作、进程组和作业对象的收尾、调用被改动或工具被卸载时的核对、`edit` 的匹配和换行保持、`grep` 与匹配进程之间的协议。
