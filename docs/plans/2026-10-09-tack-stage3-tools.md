# Tack 第 3 阶段实施计划：文件与命令工具、权限与审批、系统提示、指令文件

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 `tack run` 能在一个小仓库里读文件、改文件、跑命令验证，并且每一个有副作用的动作都经过权限检查。

**Architecture:** 文件工具经过一个文件系统接缝，读和写各有一个意图事件，沙箱模式和"先读后写"都挂在上面。命令工具经过子进程运行器和沙箱接缝。工具管线新增一层守卫，权限规则和审批放在守卫里，别的监听器绕不过去。系统提示由段落注册表拼出，会变的事实和指令文件作为用户角色的上下文消息在回合开始时写入日志。

**Tech Stack:** 与第 2 阶段相同，不增加运行依赖。Windows 上的命令工具用系统自带的 Windows PowerShell 5.1，其他平台用 bash。

设计依据是 `docs/plans/2026-10-09-tack-design.md` 的第 8.4、8.5、9、10.1 到 10.3 节。本计划与这些章节不一致的地方列在"与设计文档的差异"一节，阶段结束时把设计文档改成一致。

---

## 结束标志

无界面模式能读、改、测一个小仓库。具体是三条。

1. 用 DeepSeek Flash 运行 `tack run`，任务是修好一个带失败检查脚本的小仓库。运行结束后，在测试里重新执行检查脚本，它通过。
2. 同一流程有脚本化测试，不需要密钥。那次真实运行的会话存为夹具，用回放适配器重跑，日志与录制的逐事件相同。
3. 默认的权限预设下，没有允许规则时命令工具不执行，模型读到的结果说明原因。

## 范围

本阶段做的：`write`、`edit`、`glob`、`grep`、命令工具，文件系统接缝，沙箱模式，审批，权限规则，子进程运行器，沙箱接缝和 `none` 提供者，系统提示服务，上下文消息（运行时上下文、指令文件、打断说明），`base` 清单，命令行的 `--permission` 和 `--allow`。

本阶段不做的：`todo_write`、`skill`、`subagent`、`ask_user_question`、`apply_patch`、`web_fetch`，结果级的溢出存盘，ripgrep 后端，容器沙箱，交互式的审批界面，系统提示里的变量。它们各自属于后面的阶段。

## 约定

- 仓库根目录是 `F:\agent harness\agent-harness`。分支是 `stage-3-tools`，结束时快进合并到 `main`。不执行 `git push`。
- 每个任务的顺序是：写测试，看它失败，写实现，看它通过，过闸门，提交。闸门是 `ruff check`、`ruff format --check`、`pyright`、带覆盖率的 `pytest`。
- 本计划给出每个任务的文件、对外接口和要写的测试。实现代码在执行时写，不在计划里预先写全。接口在实现中如有调整，在阶段记录里说明。
- 大约每小时请 Codex 在只读快照上复查一次。它的每条意见先写成会失败的测试，属实才修改。
- 同一时间只跑一个 pytest 会话。多个会话共用临时目录，会互相拖慢并造成假的失败。
- 带反斜杠的脚本和测试文件用编辑工具写，不用 shell 的 heredoc。

## 设计决定

下面十条是本阶段要定下来的事。每条给出决定和理由。

**1. 权限检查放在守卫里。** 工具管线在 `tools/pre-execute` 之后新增守卫。守卫是登记的函数，只能返回拒绝，没有"允许"这个返回值。管线在 waterfall 放行之后依次运行全部守卫，第一个拒绝生效。权限规则和审批放在一个守卫里。理由：waterfall 的监听器可以不调用 `next` 就返回放行，排在外层的监听器因此能跳过内层。守卫不在这条链上，谁也跳不过它。

**2. 工具自己说明它动什么。** `Tool` 增加两个字段。`effect` 取 `none`、`read`、`write`、`run`，说明一次调用对外界做什么。`target` 是一个函数，返回规则可以匹配的文本：文件工具返回路径，命令工具返回命令。理由：规则的写法是 `工具名(匹配串)`，匹配串对的是什么，只有工具自己知道。

**3. 文件工具的边界。** 文件工具可以写任何路径，由策略决定放不放行。读：工作目录之内放行，之外只在 `danger-full-access` 模式下放行。写：`read-only` 一律拒绝；`workspace-write` 只放行工作目录之内和系统临时目录；`danger-full-access` 全部放行。`.git` 和 `.tack` 目录在前两种模式下不可写。路径先解析符号链接再判断。理由：与设计第 9.2 节一致。工作目录之外的读在本阶段不提供"询问后放行"，等交互界面有了再加。

**4. 先读后写挂在文件系统接缝上。** 读成功后记下文件的指纹，即大小和修改时间。写和改之前比对：没读过就拒绝，读后变了也拒绝并提示重读。Agent 自己写入后更新指纹。新建文件不需要先读。记录按会话存在内存里，恢复会话后要重读。理由：设计第 8.4 节，做法取自 DeepSeek Harness。

**5. 审批只有一次性授权。** `approval.ask()` 的结果是封闭集合：`allowed-once`、`rejected`、`cancelled`、`unavailable`。审批策略为 `never` 时不问任何人，直接 `rejected`。没有应答者、应答者出错或答非所问，都算 `unavailable`。只有 `allowed-once` 放行。理由：设计第 9.5 节。无界面模式没有应答者，所以需要审批的动作在那里一律不执行。

**6. 规则的来源有三处，项目文件不能放宽。** 用户目录的 `<TACK_HOME>/permissions.yml` 可以写 `deny`、`ask`、`allow`。项目的 `.tack/permissions.yml` 只认 `deny` 和 `ask`，里面的 `allow` 被忽略并记一条警告。命令行的 `--allow` 是本次运行的允许规则。求值顺序固定为 `deny`、`ask`、`allow`，首个命中生效。理由：克隆下来的仓库不应该能替用户批准命令。

**7. 没有规则命中时的默认。** `none` 和 `read` 放行。`write` 放行，范围由第 3 条的文件策略把关。`run` 在 `danger-full-access` 模式下放行，在沙箱强度为 `full` 时放行，其余情况要审批。理由：沙箱强度为 `none` 时命令无法被约束，设计第 9.3 节要求这时一律审批或命中允许规则。

**8. Windows 上的命令工具叫 `powershell`。** 这台机器和多数 Windows 机器只有 Windows PowerShell 5.1，没有 PowerShell 7。工具名在同一平台上要稳定，规则才写得下去。工具描述写明版本和它与 bash 的主要差别。其他平台的工具叫 `bash`。

**9. 上下文消息是一种机制。** 指令文件、运行时上下文和打断说明都是用户角色的消息，由登记的来源在回合开始时给出文本。文本与日志里上一次写的不同才写新消息。来源声明变化时是追加还是原位替换。理由：三者的写入时机和去重规则相同，分三处实现会各有各的边角。

**10. 回复被打断后给模型一句说明。** 上一回合以 `aborted` 或 `interrupted` 结束时，下一回合开始先写一条说明。理由：第 2 阶段的请求快照显示，被打断的回复后面直接跟着新的用户消息，模型看不出发生过什么。Codex 和 Claude Code 都写这样一条说明。

## 与设计文档的差异

1. 子进程模块叫 `tack.process`，不叫 `tack.subprocess`，避免与标准库同名。
2. Windows 上的命令工具叫 `powershell`，不叫 `pwsh`。
3. `grep` 和 `glob` 本阶段只有纯 Python 实现。ripgrep 后端以后作为优化加入。
4. 审批策略的状态放在 `tack.approval`，沙箱模式的状态放在 `tack.sandbox`，预设和规则放在 `tack.permissions`。设计里没有写明归属。
5. 系统提示本阶段没有变量，只有段落。
6. 项目级规则文件里的 `allow` 不生效。设计里没有这条限制。
7. 新增打断说明。

## 模块与接口

### `tack.tools`（修改）

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

守卫拿到的上下文是发起这次调用的那一个，也就是 Agent 的上下文，审批的询问经它分发。守卫抛异常或返回别的东西时，调用以 `CHECK_FAILED` 结束，不执行。守卫经 `cancellation.guard` 等待。`target` 抛异常时同样以 `CHECK_FAILED` 结束。

### `tack.fs`（新建，插件，提供 `FS`）

```python
@dataclass(frozen=True, slots=True)
class Located:
    path: Path          # 绝对路径，符号链接已解析
    written: str        # 模型写的原样
    inside: bool        # 是否在工作目录之内
    relative: str       # 在内时是相对工作目录的 POSIX 路径，在外时是绝对的 POSIX 路径

def locate(cwd: Path, written: str) -> Located: ...

@dataclass(frozen=True, slots=True)
class Access:
    session: Session
    cwd: Path
    where: Located
    kind: Literal["read", "write"]

@dataclass(frozen=True, slots=True)
class Permit: ...

@dataclass(frozen=True, slots=True)
class Refuse:
    reason: str
    code: str

FS_ACCESS: WaterfallEvent[Access, Permit | Refuse]      # "fs/access"
FS_OBSERVED: EmitEvent[Observed]                        # "fs/observed"，读或写成功之后

class Fs:
    async def check(self, access: Access) -> None       # 拒绝时抛 ToolFailure，不作答也算拒绝
    def observed(self, session: Session, path: Path) -> None
```

没有监听器时的默认：工作目录之内放行，之外拒绝，错误码 `OUTSIDE_WORKING_DIRECTORY`。

### `tack.sandbox`（新建，插件，提供 `SANDBOX`）

```python
Mode = Literal["read-only", "workspace-write", "danger-full-access"]
Strength = Literal["none", "partial", "full"]

class SandboxModeEvent(EventData):          # "sandbox/mode"，可跳过
    mode: Mode

@dataclass(frozen=True, slots=True)
class Confined:
    argv: tuple[str, ...]
    strength: Strength

class SandboxProvider(Protocol):
    def confine(self, argv: Sequence[str], *, mode: Mode, cwd: Path) -> Confined: ...

class Sandbox:
    providers: Registry[SandboxProvider]
    def mode(self, session: Session) -> Mode
    def set_mode(self, session: Session, mode: Mode) -> None
    def confine(self, argv: Sequence[str], *, session: Session, cwd: Path) -> Confined
    def strength(self) -> Strength           # 当前提供者能做到的强度
```

配置项 `mode` 是新会话的默认值，`provider` 选提供者，默认 `none`。没有可用提供者时 `confine` 抛 `SandboxUnavailableError`。`none` 提供者原样返回命令，强度为 `none`。

本模块还登记 `fs/access` 的监听器，执行设计决定第 3 条。

### `tack.approval`（新建，插件，提供 `APPROVAL`）

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

### `tack.permissions`（新建，插件）

```python
@dataclass(frozen=True, slots=True)
class Rule:
    tool: str               # 可带 * 通配
    pattern: str | None     # 括号里的匹配串，没有括号时为 None

def parse_rule(text: str) -> Rule
def matches(rule: Rule, tool: Tool, target: str | None) -> bool
def split_command(command: str) -> list[str]     # 按 && || ; | 和换行拆开，引号内的不拆

class Config(BaseModel):
    preset: Literal["read-only", "workspace-write", "full-access"] = "workspace-write"
    user_file: Path | None = None       # 清单里写 ${TACK_HOME}/permissions.yml
    deny: tuple[str, ...] = ()
    ask: tuple[str, ...] = ()
    allow: tuple[str, ...] = ()
```

匹配规则：`effect` 为 `run` 的工具，`*` 匹配任意字符。其他工具把匹配串当作路径模式，`*` 不跨目录，`**` 跨目录。复合命令逐段判断，取最严格的结果，顺序是拒绝、询问、放行。没有任何规则命中的那一段按默认处理。

插件加载时做三件事：登记守卫；在每个会话的第一步之前把预设对应的沙箱模式和审批策略写进日志（日志里已经有就不写）；项目规则文件在第一次用到某个工作目录时读入。

守卫的错误码：规则拒绝是 `DENIED`，审批被拒是 `APPROVAL_REJECTED`，没有人能审批是 `APPROVAL_UNAVAILABLE`，审批被取消是 `APPROVAL_CANCELLED`。

### `tack.process`（新建，不是插件）

```python
@dataclass(frozen=True, slots=True)
class Completed:
    exit_code: int | None       # 被停下时为 None
    timed_out: bool
    output: str                 # 标准输出和标准错误合在一起，按到达顺序
    omitted_bytes: int          # 中段被丢弃的字节数

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

超出上限时保留头尾各一半，中间插入一行说明丢弃了多少字节。超时和调用方被取消时杀掉整棵进程树。进程退出后最多再等 2 秒读管道，防止孙进程占着管道不放。

### `tack.builtin_tools`（新建五个工具）

| 工具 | 参数 | `effect` | 可并行 |
|---|---|---|---|
| `write` | `path`、`content` | `write` | 否 |
| `edit` | `path`、`old`、`new`、`replace_all=false` | `write` | 否 |
| `glob` | `pattern`、`path="."` | `read` | 是 |
| `grep` | `pattern`、`path="."`、`glob=null`、`ignore_case=false` | `read` | 是 |
| `bash` 或 `powershell` | `command`、`timeout_seconds=null` | `run` | 否 |

`edit` 的错误码：`NOT_FOUND`、`NOT_TEXT`、`NO_CHANGE`（`old` 与 `new` 相同）、`NO_MATCH`、`AMBIGUOUS`（多处匹配又没给 `replace_all`）。文件用 CRLF 换行而 `old` 用 LF 时，按文件的换行再匹配一次，写回时保持文件原来的换行。

`glob` 按修改时间从新到旧，最多 100 条。`grep` 最多 250 条，每条是 `路径:行号:内容`。两者都跳过 `.git` 目录，跳过不像文本的文件，对扫描的文件数设上限，超限时在结果里说明。

命令工具的结果是输出加一行状态。状态行写退出码，或者写超时后被停下。命令跑完但退出码非零不算工具出错，起不来才算。

### `tack.system_prompt`（新建，插件，提供 `SYSTEM_PROMPT`）

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

### `tack.agent`（修改）：上下文消息

```python
@dataclass(frozen=True, slots=True)
class ContextSource:
    render: Callable[[Agent], Awaitable[str | None]]
    order: int = 0
    on_change: Literal["append", "replace"] = "append"

class Agents:
    context: Registry[ContextSource]
```

循环在每个回合的第一步之前、认领输入之前，按 `order` 逐个询问来源。来源给出的文本与日志里这个来源上一次的文本不同，就写一条 `user/message`，`source` 是 `context/<名字>`。`on_change` 为 `replace` 时新消息顶替旧消息。来源返回 `None` 时不写。来源抛异常时回合以 `error` 结束。

三个来源：`instructions`（`order` 0，`replace`）、`runtime`（`order` 100，`append`）、`interruption`（`order` 200，`append`，只在上一回合被打断时有内容）。

### `tack.context.instructions`（新建，插件）

配置项：`names`（默认 `AGENTS.md`、`CLAUDE.md`）、`user_file`、`max_bytes`（默认 65536）。文本由宽到窄排列：先用户级文件，再从项目根到工作目录的链。项目根是最近的含 `.git` 的上级目录，找不到时只看工作目录。超预算时先丢更宽的文件，最后才截断最具体的文件，并在文本里说明。文件内容里出现包裹标签的字面量时转义。

## 任务

每个任务一个或几个提交。表后的各节给出文件和测试要点。

| # | 任务 | 依赖 |
|---|---|---|
| 1 | 工具的 `effect`、`target` 和守卫 | |
| 2 | 文件系统接缝，`read` 改为经过它 | 1 |
| 3 | 沙箱模式与沙箱接缝 | 2 |
| 4 | 先读后写 | 2 |
| 5 | `write` 和 `edit` | 2、4 |
| 6 | `glob` 和 `grep` | 2 |
| 7 | 审批 | |
| 8 | 权限规则与守卫 | 1、3、7 |
| 9 | 子进程运行器 | |
| 10 | 命令工具 | 3、8、9 |
| 11 | 系统提示 | |
| 12 | 上下文消息与打断说明 | |
| 13 | 运行时上下文的事实 | 3、7、10、12 |
| 14 | 指令文件 | 12 |
| 15 | `base` 清单与命令行参数 | 1 到 14 |
| 16 | 端到端：脚本化、真实运行、回放 | 15 |
| 17 | 文档同步与阶段记录 | 16 |

### Task 1: 工具的 `effect`、`target` 和守卫

**Files:** 修改 `src/tack/tools/tool.py`、`src/tack/tools/pipeline.py`、`src/tack/tools/__init__.py`。测试 `tests/tools/test_tool.py`、`tests/tools/test_pipeline.py`。

**测试要点。** 守卫的拒绝使调用得到错误结果，工具不执行。waterfall 的监听器不调用 `next` 就放行时，守卫仍然运行。几个守卫按登记顺序运行，第一个拒绝之后的不再运行。守卫抛异常、返回别的东西、自己抛出取消，都以 `CHECK_FAILED` 结束。贡献守卫的插件卸载后守卫消失。`effect` 的默认值是 `none`。

### Task 2: 文件系统接缝

**Files:** 新建 `src/tack/fs/__init__.py`。修改 `src/tack/builtin_tools/read.py`、`src/tack/bundles/minimal.yml`。测试 `tests/fs/test_fs.py`，修改 `tests/builtin_tools/test_read.py`。

**测试要点。** `locate` 对相对路径、绝对路径、`..`、指向工作目录之外的符号链接和目录联接给出正确的 `inside`。Windows 上大小写不同的同一路径算在内。没有监听器时工作目录之内放行、之外拒绝。监听器可以拒绝之内的，也可以放行之外的。监听器不作答算拒绝。`read` 成功后发出 `fs/observed`。`read` 原有的测试全部保持通过。

### Task 3: 沙箱模式与沙箱接缝

**Files:** 新建 `src/tack/sandbox/__init__.py`。测试 `tests/sandbox/test_sandbox.py`。

**测试要点。** 模式默认取配置，`set_mode` 写一条 `sandbox/mode`，恢复的会话读回同一个模式。三种模式下读和写的放行表，逐格测试，包括临时目录和 `.git`、`.tack`。没有提供者时 `confine` 抛错。`none` 提供者原样返回命令。没有这个插件也能读带 `sandbox/mode` 的会话。

### Task 4: 先读后写

**Files:** 新建 `src/tack/fs/observation.py`。测试 `tests/fs/test_observation.py`。

**测试要点。** 没读过的已有文件不能写。读过之后可以。读后文件被别的进程改了，拒绝并提示重读。Agent 自己写过之后可以接着再写。新建文件不需要先读。两个会话的记录互不相干。

### Task 5: `write` 和 `edit`

**Files:** 新建 `src/tack/builtin_tools/write.py`、`src/tack/builtin_tools/edit.py`。测试 `tests/builtin_tools/test_write.py`、`tests/builtin_tools/test_edit.py`。

**测试要点。** `write` 新建文件和缺少的上级目录，覆盖已有文件，内容按原样写入不改换行，写入是先写临时文件再改名。`edit` 的五个错误码各一条测试。唯一匹配时只改那一处。`replace_all` 改全部。CRLF 文件用 LF 的 `old` 能匹配，写回后仍是 CRLF。文件不是 UTF-8 时拒绝。两个工具都经过文件系统接缝：只读模式下被拒，受保护的目录被拒。

### Task 6: `glob` 和 `grep`

**Files:** 新建 `src/tack/builtin_tools/glob.py`、`src/tack/builtin_tools/grep.py`、`src/tack/fs/walk.py`。测试 `tests/builtin_tools/test_glob.py`、`tests/builtin_tools/test_grep.py`、`tests/fs/test_walk.py`。

**测试要点。** `glob` 的 `**`、`*`、`?` 和字符集。结果按修改时间从新到旧，超过 100 条时截断并说明。`grep` 给出路径、行号和内容，`glob` 参数限定文件，`ignore_case` 生效，正则写错时返回 `INVALID_PATTERN`。两者跳过 `.git`，跳过不像文本的文件，不走出工作目录，不跟随指向外面的符号链接。扫描的文件数有上限。调用方被取消时遍历的线程停下。

### Task 7: 审批

**Files:** 新建 `src/tack/approval/__init__.py`。测试 `tests/approval/test_approval.py`。

**测试要点。** 策略为 `never` 时不调用应答者，结果是 `rejected`。没有应答者是 `unavailable`。应答者抛异常、返回集合之外的值，是 `unavailable`。应答者返回四个值之一时原样给出。每次询问写一对事件，`never` 也写。等待期间调用方被取消，取消照常传播，日志里补一条 `cancelled` 的决定。策略写进日志，恢复后读回。

### Task 8: 权限规则与守卫

**Files:** 新建 `src/tack/permissions/__init__.py`、`src/tack/permissions/rules.py`。测试 `tests/permissions/test_rules.py`、`tests/permissions/test_guard.py`。

**测试要点。** 规则的解析和写错时的报错。路径模式和命令模式的匹配，各一组表格测试。复合命令的拆分，引号内的分隔符不拆。求值顺序：同时命中 `deny` 和 `allow` 时拒绝，同时命中 `ask` 和 `allow` 时询问。默认表按 `effect`、沙箱模式和强度逐格测试。项目文件里的 `allow` 不生效并记警告。预设在第一步之前写进日志，恢复的会话不重复写。守卫的四个错误码。性质测试：随机的规则集和调用，守卫的结论与一个直接照定义写的参照实现一致。

### Task 9: 子进程运行器

**Files:** 新建 `src/tack/process.py`。测试 `tests/test_process.py`。

**测试要点。** 用 `sys.executable -c` 当子进程，不依赖任何外部命令。退出码原样给出。标准输出和标准错误按到达顺序合并。输出超限时保留头尾并说明丢弃的字节数，恰好等于上限时不丢。超时后进程和它的子进程都不在了。调用方被取消后同样。孙进程占着管道时不会一直等。环境变量的清洗。输出不是 UTF-8 时用替换字符。

### Task 10: 命令工具

**Files:** 新建 `src/tack/builtin_tools/shell.py`。测试 `tests/builtin_tools/test_shell.py`。

**先做试验。** 在这台机器上试出 Windows PowerShell 5.1 的调用方式，要满足四条：中文输出不乱码；原生命令的退出码传得出来；出错的 cmdlet 给出非零退出码；标准错误是普通文本，不是 CLIXML。候选做法是 `-EncodedCommand` 和临时脚本文件加 `-File`。试验的结论写进模块的说明。

**测试要点。** 命令在工作目录里执行。退出码写在状态行。超时写在状态行，进程被停下。`timeout_seconds` 超过上限时按上限。输出超限时截断。命令起不来时是工具出错。环境里的密钥不传给命令。`target` 返回命令本身。按平台各有一组测试，另一平台的那组跳过。

### Task 11: 系统提示

**Files:** 新建 `src/tack/system_prompt/__init__.py`。修改 `src/tack/agent_loop/loop.py`。测试 `tests/system_prompt/test_system_prompt.py`、`tests/agent_loop/test_turns.py`。

**测试要点。** 段落按 `order` 再按名字排序。`system-prompt/assemble` 的监听器可以改写结果。没有这个服务时循环直接用 `AgentSpec.system_prompt`。段落变了，下一回合的 `system/message` 顶替旧的。文本没变就不写新事件。

### Task 12: 上下文消息与打断说明

**Files:** 修改 `src/tack/agent/agent.py`、`src/tack/agent_loop/loop.py`。测试 `tests/agent_loop/test_context.py`，更新快照 `tests/agent_loop/snapshots/a_session.txt`。

**测试要点。** 来源的文本在回合开始时写成用户消息，排在这一回合的输入之前。文本没变不重复写。`append` 追加，`replace` 顶替。来源返回 `None` 不写。来源抛异常时回合以 `error` 结束。上一回合被取消后，下一回合的请求里有打断说明。上一回合正常结束时没有。性质测试的不变量保持成立：历史只增不改这一条要为 `replace` 来源放宽。

### Task 13: 运行时上下文的事实

**Files:** 修改 `src/tack/agent_loop/__init__.py`、`src/tack/sandbox/__init__.py`、`src/tack/approval/__init__.py`、`src/tack/builtin_tools/shell.py`。测试 `tests/agent_loop/test_context.py`。

**测试要点。** 快照里有工作目录、沙箱模式、审批策略、沙箱强度和命令工具的种类。模式变了，下一回合追加新快照。没变就不追加。贡献事实的插件卸载后，它的那一行消失。

### Task 14: 指令文件

**Files:** 新建 `src/tack/context/__init__.py`、`src/tack/context/instructions.py`。测试 `tests/context/test_instructions.py`。

**测试要点。** 用户级文件在前，项目根到工作目录的链由宽到窄。没有 `.git` 时只看工作目录。内容相同的文件只出现一次。超预算时先丢更宽的文件，最后截断最具体的并说明。包裹标签的字面量被转义。文件变了，下一回合发一条顶替的消息。文件没了，发一条说明。没有任何指令文件时不写消息。

### Task 15: `base` 清单与命令行参数

**Files:** 新建 `src/tack/bundles/base.yml`。修改 `src/tack/bundles/headless.yml`、`src/tack/apps/launcher.py`、`src/tack/apps/headless.py`。测试 `tests/apps/test_launcher.py`、`tests/apps/test_headless.py`、`tests/test_package.py`。

**测试要点。** `--dump-config` 列出 `base` 的全部条目。`--permission` 改 `permissions` 条目的预设，没给时不动。`--allow` 可以给多次。清单里没有 `permissions` 条目而用了这两个参数时报错，退出码 2。每个产品可见的插件有一条经真实清单加载的测试。

### Task 16: 端到端

**Files:** 新建 `tests/apps/test_fix_a_repository.py`、`tests/fixtures/recorded/deepseek-fix/`、`tests/live/test_deepseek_fix.py`。修改 `tests/replay/test_recorded.py`，让它对夹具目录下的每个录制各跑一遍。

**场景。** 小仓库里有 `calc.py`、`check.py` 和 `AGENTS.md`。`calc.py` 有一处错误，`check.py` 调用它并打印 `ok` 或失败原因，输出里不带时间和绝对路径。任务是让 `python check.py` 通过。允许规则只放行这一条命令。

**测试要点。** 脚本化的模型依次读、改、跑命令、回答。测试在运行结束后自己再执行一次 `check.py` 并断言通过。没有允许规则时命令不执行，结果的错误码是 `APPROVAL_UNAVAILABLE`。请求快照记下模型看到的指令文件、运行时上下文和每个工具的结果。真实运行的那条测试没有密钥时跳过。

### Task 17: 文档同步与阶段记录

更新设计文档的第 3.3、8、9、10 节和 README。写 `docs/plans/<日期>-tack-stage3-record.md`，做法与第 2 阶段的记录相同：任务对应的提交，重新测量的测试数和覆盖率，复查记录，变异检查的结果。

## 测试策略

- 文件工具和权限用真实的临时目录测试，不模拟文件系统。
- 命令工具用 `sys.executable` 当被执行的程序，三个平台都能跑。
- 策略类的代码配性质测试，与照定义写的参照实现对比。本阶段有两处：规则求值，文件访问的放行表。
- 循环的性质测试加上工具真的写文件的情形，检查取消之后磁盘上没有半截的文件。
- 新代码照例做变异检查。先加 `permissions` 和 `process` 两组变异体。

## 风险

| 事项 | 处理 |
|---|---|
| Windows PowerShell 5.1 的输出编码和退出码行为不直观 | 任务 10 先做试验，把结论写成测试 |
| 杀进程树在 Windows 上靠 `taskkill /T`，父进程先死的孙进程杀不到 | 读管道设 2 秒上限，保证工具不会一直等。容器沙箱在第 7 阶段解决隔离 |
| 命令规则按文本匹配，绕得过去 | 文档写明它不是安全边界 |
| 模型写的正则可能回溯很久，Python 的 `re` 停不下来 | 每行截到 2000 字符再匹配，工具的时限兜底 |
| 本阶段代码量可能接近第 2 阶段 | 任务 1 到 10 完成后先合并一次，再做 11 到 17 |
