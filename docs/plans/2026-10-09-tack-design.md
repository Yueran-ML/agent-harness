# Tack 设计文档

日期：2026-10-09
状态：四个部分均已由作者确认。第 1 阶段已完成，第 3.4 节和第 4 节已按实现结果修订。

Tack 是一个用 Python 从零实现的插件化 Agent harness。实现路线对齐 DeepSeek Harness，并吸收 Claude Code 和 Codex CLI 的做法。本文是实施计划的依据。

## 1. 定位与目标

### 1.1 定位

Tack 是一个能安装、能日常使用、做得扎实的开源 harness。它是工程项目，不是研究项目。基准的用途是验收和回归，不是检验假说。

### 1.2 完成标准

1. 可安装。一条命令装好，Windows、Linux、macOS 三个平台的持续集成全绿。
2. 可用。用 DeepSeek Flash 能在终端里完成真实的编码任务。会话可恢复、可分叉。进程被杀后重启不丢状态。
3. 可扩展。新增一个工具插件不超过 30 行。有插件开发指南和三个示例插件。
4. 兼容。`AGENTS.md`、`SKILL.md`、MCP、Claude Code 格式的 `hooks.json` 直接可用。
5. 安全。有权限规则、审批和容器沙箱。
6. 可观测。每个会话能查 token、缓存命中率、耗时、各工具的调用次数与失败率。`tack doctor` 自检环境。

### 1.3 明确不做

- 文件监听式热重载、每个能力拆成三个包、多代会话格式迁移链。
- 三家各自的原生沙箱（Seatbelt、Landlock、Windows 受限令牌）。隔离交给容器。
- 常驻子代理、代理团队、工作流引擎、模型写代码调工具的模式。
- 把会话日志上传给模型提供方，以及任何匿名标识请求头。
- LSP、浏览器或电脑操作、定时任务、桌面应用。

### 1.4 决策记录

| 事项 | 决定 |
|---|---|
| 项目名 | Tack。导入包名 `tack`，命令 `tack`，发行包名 `tack-harness` |
| 仓库 | `github.com/Yueran-ML/agent-harness`。本地只做提交，推送由作者用 GitHub Desktop 完成 |
| 实现路线 | 对齐 DeepSeek Harness，参考 Claude Code 和 Codex，取其精华 |
| 重心 | 偏工程 |
| 界面 | 先终端 CLI，后补 Web，Web 要好看 |
| API 模型 | DeepSeek Flash（`deepseek-flash`，即 V4.1-Flash），密钥在环境变量 `DEEPSEEK_API_KEY` |
| 本地模型 | `D:\ollama-models\gguf` 下的 Qwen2.5、Llama、Gemma 各量化档 |
| 基准任务 | 代码修改、文件与命令行操作、SQL 数据查询、多步工具计算，四类都做 |
| 评测隔离 | 容器内执行。Docker 和 WSL 由作者在第 7 阶段前安装 |
| 交付 | 公开仓库、工程报告、Web 界面。SUS 可用性调查由作者完成 |

## 2. 参考与取舍

调研笔记在 `docs/research/`：`dsh-digest.md`、`codex-digest.md`、`claude-code-digest.md`。前两份来自源码，逐条带路径和行号。第三份只用了公开文档。

| 子系统 | 以谁为主 | 从另两家补充 |
|---|---|---|
| 内核与组合 | DeepSeek Harness：服务仓库、依赖声明、可撤销注册、五种事件分发 | Codex："提交进、事件出"，前端只碰这两条通道 |
| Agent 循环 | DeepSeek Harness：回合与步、四个拦截点、三种输入 | Claude Code：结束原因分类。Codex：中断标记 |
| 模型接入 | DeepSeek Harness：统一的内容块和流式词汇、错误码归一、重试外挂 | Codex：只对完整的输出项动作 |
| 工具 | DeepSeek Harness：分层管线、守卫只能拒绝、参数记录后不可改 | Claude Code：精确替换编辑加先读后改。Codex：补丁式编辑、头尾截断 |
| 上下文 | DeepSeek Harness：日志即事实、只有追加和替换、压缩是带锁的事务 | Codex：交接式摘要。Claude Code：压缩后重新注入指令和计划 |
| 安全 | DeepSeek Harness 与 Codex 相同的两个旋钮，审批失败即关闭 | Claude Code：规则语法，拒绝先于询问先于允许 |
| 编排 | DeepSeek Harness：一次性子代理，深度限 1 | Claude Code：工具白名单和回合上限 |
| 扩展 | 三家一致：`SKILL.md` 渐进披露、MCP 工具命名 | Claude Code 与 Codex：`hooks.json` |
| 测试 | DeepSeek Harness：脚本化假模型、录制回放、性质测试 | Codex：请求形状快照 |

三家都没有、Tack 需要的两项，做成可开关的插件：

- 回合预算。DeepSeek Harness 和 Codex 的循环都没有内置步数上限。无人值守运行必须有。
- 工具调用修复。两家都不修畸形 JSON，只把错误返回给模型。弱模型上修不修差别可能很大，所以做成开关。

Tack 与 DeepSeek Harness 的另一处不同：后者没有内置终端界面，Tack 先交付终端 CLI。

## 3. 架构总览

### 3.1 数据流

```
用户 / 基准任务
      │  followup / steer / inject / cancel
      ▼
   Agent ──────► ctx.llm（适配器：OpenAI Chat、Ollama 原生、脚本化、回放）
      │  ▲
      ▼  │ 工具结果
   ctx.tools 管线：前置拦截 → 守卫 → 审批 → 环绕执行 → 后置拦截 → 最终不变式
      │
      ▼
   Session（只追加的事件日志，唯一事实来源）
      │
      ├──► 消息投影：日志 → 下一次请求的消息列表
      ├──► 状态投影：inbox、待办、用量、重试计数
      ├──► 持久化：JSONL
      └──► 前端：CLI、无界面、之后的 JSON-RPC 与 Web
```

规则只有一条：模型能看到的，必须先写进日志。任何一次请求都能由日志重建。

### 3.2 技术栈

- Python 3.11 及以上，asyncio。
- 运行依赖：`httpx`、`pydantic` v2、`PyYAML`、`rich`、`prompt_toolkit`。第 6 阶段加官方 `mcp` SDK。
- 开发依赖：`pytest`、`pytest-asyncio`、`hypothesis`、`ruff`、`pyright`、`coverage`。
- 环境与构建用 `uv`，构建后端 `hatchling`，`src/` 布局。
- 数据目录 `~/.tack/`，可用环境变量 `TACK_HOME` 改。

### 3.3 包结构

单一发行包，导入名 `tack`。每个子包是一个或一组插件。

```
src/tack/
  kernel/          上下文、插件实例、事件、服务、组合加载
  session/         事件类型、日志、消息投影、JSONL 持久化、尾部修复、分叉、状态投影
  llm/             词汇、分片装配、运行时、错误码、重试、token 估算
    adapters/      openai_chat、ollama、scripted、replay
  agent/           Agent 接口、活体注册表、inbox
  agent_loop/      默认循环驱动、工具调度、运行时上下文快照
  tools/           工具定义、注册表、执行管线
  system_prompt/   段落注册与渲染
  fs/ subprocess/ shell/ sandbox/ approval/ permissions/
  builtin_tools/   read、write、edit、glob、grep、shell、todo_write、skill、subagent、ask_user_question
  context/         指令文件、溢出存盘、压缩
  guard/           回合预算、重复调用提醒、工具超时、工具调用修复
  skills/ hooks/ subagent/ mcp/ commands/ telemetry/
  apps/            启动器、无界面、cli/、bench/
  bundles/         minimal.yml、base.yml、headless.yml、cli.yml、bench.yml
tests/   benchmarks/   docs/   examples/plugins/
```

### 3.4 组合清单

应用由 YAML 清单组合出来，清单是插件条目的列表。条目有四个字段：`id`、`plugin`、`config`、`disabled`。`plugin` 是导入路径，写成 `package.module` 或 `package.module:attribute`。

| 清单 | 内容 | 用途 |
|---|---|---|
| `minimal` | 内核、会话、模型、循环、一个 shell 工具 | 最小可行脚手架，也是性能基准的对象 |
| `base` | 全部共用插件 | 其余清单的第一层 |
| `headless` | `base` 加一次性运行器 | 脚本和自动化 |
| `cli` | `base` 加终端界面 | 日常使用 |
| `bench` | `base` 加基准运行器，审批为"从不"，带回合预算 | 验收基准 |

补丁按 `id` 定位条目，可以整体替换它的 `config`、把它设为 `disabled`，或插入新条目。叠加顺序是清单、用户目录下的补丁、命令行 `--patch`。

清单和补丁是同一种格式，叫做层。一层是一组操作：`insert` 插入条目，带 `id` 的操作修改已有条目。一层可以用 `extends` 写明它建立在哪一层之上。配置里的字符串可以用 `${NAME}` 和 `${NAME:-default}` 引用环境变量。

## 4. 内核

内核是 Cordis 五个概念的 Python 复刻，共 8 个模块、约 2000 行。它的精确语义以 `2026-10-09-tack-stage1-kernel.md` 里的"内核语义速查"为准。

### 4.1 概念

1. 上下文是服务仓库。服务名是带类型的 `ServiceKey[T]`。`ctx.provide(KEY, service)` 注册服务，`ctx.use(KEY)` 取必需的服务，`ctx.get(KEY)` 取可选的服务。
2. 插件声明依赖。插件是带 `apply(ctx, config)` 的可调用对象或模块，可带 `name`、`inject`、`Config`。`inject` 里的服务没到齐，插件实例停在挂起状态。某个依赖消失时，依赖它的插件自动卸载，依赖回来后自动重新加载。插件只能 `use` 自己提供的服务，或者自己及外层插件声明过的服务。
3. 事件带类型。每个事件在声明时固定分发方式，只带一个载荷对象，用错方式调用直接报错。
4. 五种分发方式。`emit` 同步广播，忽略返回值。`parallel` 并发执行并等待全部结束。`serial` 按序等待，第一个非空返回值胜出。`bail` 是同步版的 `serial`。`waterfall` 是环绕式中间件，监听器收到载荷和 `next`。调用 `next(payload)` 继续，不调用即短路，多次调用即重试。
5. 注册可撤销。`ctx.effect(fn)` 执行 `fn` 并收集它返回的撤销函数。`ctx.on()`、`ctx.provide()`、`ctx.plugin()` 本身都是 effect。

### 4.2 插件实例

`ctx.plugin(plugin, config)` 返回一个 `Fiber`。状态为 `PENDING`、`LOADING`、`ACTIVE`、`FAILED`、`UNLOADING`、`DISPOSED`。`await fiber.dispose()` 逆序撤销全部 effect，递归卸载子插件，等清理真正结束后才返回。处于 `UNLOADING` 时拒绝新建 effect。每次状态变化都发出 `FIBER_STATE_CHANGED` 事件。

配置用 pydantic 模型校验，校验在创建 `Fiber` 时进行。校验失败时插件实例进入 `FAILED`，启动器打印错误并以非零码退出。

### 4.3 作用域

每个 Agent 有自己的作用域上下文 `agent.ctx`，由 `ctx.scope(key)` 创建。经它做的注册只对该 Agent 可见，随 Agent 销毁撤销。经带作用域的上下文分发事件时，只放行全局监听器和作用域匹配的监听器。子代理靠这个得到不同的工具集。

### 4.4 监听器异常

`emit` 和 `parallel` 的监听器抛异常时，分发器记录日志并继续，不影响其他监听器。`serial`、`bail`、`waterfall` 是决策链，异常向调用方传播。

### 4.5 取消与重入

- 等待方被取消不会打断加载、卸载或清理。异步的清理在内核自己的任务里进行。
- 提供方卸载前，先等依赖它的插件卸载完。服务被撤回期间名字保留，直到依赖方卸载完成。
- `apply` 和 disposer 不能等待一个正在等它们的 Fiber。这种情况下 `dispose()` 只发出请求，`settled()` 立即返回，应用级的 `ctx.settled()` 抛 `LifecycleError`。

### 4.6 与 Cordis 的差异

- 服务访问用带类型的键和 `use`、`get`，不用属性访问。
- 每个事件只带一个载荷对象。
- 不做文件监听式热重载。保留运行时卸载和重新挂载的能力，测试需要它。
- 卸载时严格逆序、逐个等待。Cordis 的顶层撤销函数是逆序启动、并发执行。
- 不做 `intercept` 配置叠加、mixin、Proxy 属性魔法、`!!js` 表达式。清单里只支持环境变量插值。

## 5. 会话日志

### 5.1 事件信封

```json
{"type": "tool/call", "seq": 42, "time": 1791234567890, "data": {...}}
```

`seq` 等于追加时的日志长度。可选字段 `ignorable`。模型可见的事件另带 `surface_op`，替换型的还带 `source_event_seqs`。

每种事件是一个 pydantic 数据类，类上写明事件名和是否 `ignorable`。事件类型要先在会话服务里注册，插件用 `sessions.declare(ctx, 类)` 注册自己的类型，注册随插件卸载撤销。

`append()` 在入口处依次做五件事。

1. 检查事件类型已注册。
2. 把数据写成 JSON 再读回。读回的副本与原数据不相等就拒绝。日志保存的是这份副本，调用方之后改动原对象不影响日志。
3. 校验消息面约束。
4. 写入存储。存储拒收时事件不进日志，也不通知任何人。
5. 按日志顺序同步通知观察者。观察者抛异常被隔离。观察者在通知里追加的事件排在当前事件之后通知。

事件的字段是冻结的。Python 拦不住对字段里列表和字典的修改，所以约定集合字段一律用元组，事件进日志后不再改动。

### 5.2 核心事件

| 事件 | 载荷要点 | 进入模型历史 |
|---|---|---|
| `turn/start`、`turn/end` | 回合号；`turn/end` 带 `reason` 和可选 `detail` | 否 |
| `step/start`、`step/end` | 回合号、步号 | 否 |
| `system/message` | 渲染后的系统提示，消息面的第 0 个节点 | 是 |
| `user/message` | 人的输入、注入的上下文、压缩摘要，靠 `source` 区分 | 是 |
| `assistant/message` | 装配后的回复、用量、结束原因。结束原因为空表示回复被取消打断 | 是，空内容除外 |
| `assistant/attempt` | 失败的尝试，存已到达的内容、用量和失败原因 | 否 |
| `tool/call` | `call_id`、`name`、模型原样产出的参数字符串 | 否 |
| `tool/result` | 结果内容、可选 `error{code, reason}`、可选 `meta` | 是 |
| `request/header` | 调用配置和工具模式的完整快照，含提供方、模型、上下文窗口、采样参数 | 否 |
| `session/end-seed` | 继承或恢复的历史与本次生命周期的分界 | 否 |

`turn/end.reason` 取值：`completed`、`aborted`、`blocked`、`error`、`max-tokens`、`interrupted`、`forked`。后两个只由崩溃修复和分叉合成。

插件用自己的命名空间追加事件：`agent/inbox/spliced`、`approval/asked`、`approval/decided`、`sandbox/mode`、`approval/policy`、`compaction/start`、`compaction/summary`、`compaction/end`、`todo/write`、`llm/retry`、`hook/invoked`、`hook/result`、`subagent/start`、`subagent/end`。

### 5.3 消息面

`surface_op` 只有两种：`"append"` 和 `{"op": "replace", "start_seq", "end_seq"}`。替换用一个新节点顶替一段现有节点，必须在 `source_event_seqs` 里列出全部被遮蔽的节点。

事件是否进消息面由它的数据类决定，`for_model()` 返回消息的事件才是节点。调用方写 `session.append(data, replaces=[...])` 做替换，被替换的节点必须在当前消息面上相邻，并按当前顺序列出。只有带消息的事件才能替换别的节点。

`session.messages` 是当前消息面上的消息，`session.surface` 是对应事件的 `seq`。两者随追加和替换增量维护，也可以按顺序重放全部事件重建，性质测试检查两种算法结果一致。压缩和工具结果裁剪都只做替换，原始事件永远留在日志里。

### 5.4 持久化

- 路径：`<TACK_HOME>/sessions/<规范化的工作目录>/<会话 id>/session.v1.jsonl`。
- 首行是头：`type: "session"`、`version`、`id`、`created_at`、`cwd`、可选 `parent_session`。之后一事件一行。
- 每条事件追加后写入并刷新缓冲。模型请求前、工具体执行前、回合结束时各做一次落盘屏障，即 `await session.flush()`。
- 新会话在写入第一条事件之前不落盘。
- 同一会话只允许一个写者，靠操作系统对会话目录里 `lock` 文件的锁保证。进程退出时锁自动释放。
- 写入或落盘失败一次后，这个文件不再接受写入。有洞的日志比中断的日志更糟。
- 读到不认识且未标 `ignorable` 的事件，拒绝加载。读到别的格式版本，拒绝加载。不认识但标了 `ignorable` 的事件原样保留，复制日志时原样写回。
- 行格式有误、`seq` 不连续、消息面对不上，都拒绝加载，错误信息给出文件和行号。
- 最后一行没有换行符，说明写到一半进程就死了。这一行不读，续写时把它截掉。

只有一个格式版本，不做迁移链。

### 5.5 恢复与分叉

恢复时如果日志停在回合中途，追加修复事件：给每个没有结果的工具调用补一条错误结果，再补 `step/end` 和 `turn/end{interrupted}`。

修复区分两种状态。有 `tool/call` 记录但没有结果，标 `TOOL_OUTCOME_UNKNOWN`。连调用记录都没有，标 `TOOL_NOT_STARTED`。结果文本告诉模型只有只读或幂等操作才可重试。

分叉复制到指定 `seq` 为止的前缀，追加 `session/end-seed{inherited: true}`，再用同一个修复函数以 `forked` 为原因关闭未闭合的尾部。

### 5.6 状态投影

`Projection(name, initial, apply)` 是一个纯同步的折叠单元。`apply(state, event)` 返回新状态，不改传入的状态，不关心该事件时返回同一引用。`projection.of(session)` 给出该会话当前的状态。状态在被读取时才计算，每条事件对每个会话只折叠一次，恢复出来的会话从第一条事件折起。

会话包自带两个投影：`TURN` 给出回合号、步号和是否在回合中，`USAGE` 给出累计调用次数和 token 用量。inbox、待办、重试计数用同一种办法各自定义。

面向前端的投影注册表放到第 10 阶段，和第一个需要它的前端一起做。

## 6. 模型接入

### 6.1 词汇

- 消息角色：`system`、`user`、`assistant`、`tool`。
- 内容块：`text`、`reasoning`、`tool-call`。`reasoning` 块可带适配器私有的 `replay_state`，下次请求原样带回，只交还给产生它的适配器。
- 流式分片：`block-start`、`text-delta`、`reasoning-delta`、`tool-call-delta`、`block-end`、`usage`、`finish`。
- 结束原因：`stop`、`tool-calls`、`max-tokens`、`error`。结束分片带 `failure` 当且仅当原因是 `error`。
- 用量字段互不重叠：`input_tokens` 只含未命中缓存的输入，`cache_read_tokens`、`cache_write_tokens`、`output_tokens` 单列。

适配器只需实现 `stream(request)`，返回一个产出分片的异步迭代器。取消不用信号对象。读取方的任务被取消或提前离开时，运行时关闭这个迭代器，这次调用没有结束分片。共享的装配器把分片折回内容块。结束原因为 `max-tokens` 时丢弃全部工具调用，因为被截断的调用不安全。工具参数全程保持原始字符串。

### 6.2 失败与重试

所有失败归一成 `LlmFailure{message, code, status?, retry_after_ms?, request_id?}`。适配器可以抛 `LlmError`，也可以发错误结束分片，运行时对外统一成结束分片。无论适配器做了什么，一次调用的分片流都以恰好一个结束分片收尾，适配器的流在这个分片送达之前已经关闭。

读取方的取消不会被吞掉。关闭适配器的流时出错只记日志。取消恰好落在关闭过程中时，关闭被打断，`call.aclose()` 会再关一次。运行时不把关闭放到别的任务里做保护，因为跨任务关闭生成器会弄坏在生成器里持有任务级状态的适配器。清理不能被打断的适配器要自己做保护。

运行时还让结束分片与实际到达的内容一致。原因为 `tool-calls` 时回复里一定有工具调用，原因为 `stop` 时一定没有，不管提供方报的是哪一个。

错误码：`AUTH`、`QUOTA`、`RATE_LIMIT`、`CONTEXT_WINDOW_EXCEEDED`、`INVALID_REQUEST`、`SERVER`、`HTTP_<状态码>`、`TRANSPORT`、`TIMEOUT`、`STREAM_CLOSED`、`MALFORMED_RESPONSE`、`EMPTY_RESPONSE`、`INTERNAL`。

- 结束原因不是 `error` 但没有内容块，算 `EMPTY_RESPONSE`。
- 适配器抛出 `LlmError` 以外的异常，算 `INTERNAL`。流没有结束分片就断了，算 `STREAM_CLOSED`。分片顺序违反协议，算 `MALFORMED_RESPONSE`。
- 流空闲看门狗默认 300 秒，只在等待下一个分片时计时。
- 适配器内部不重试。重试是独立插件，挂在"请求出错"拦截点上。
- 默认策略：最多 5 次，只重试 `EMPTY_RESPONSE`、`RATE_LIMIT`、`SERVER`、`TIMEOUT`、`TRANSPORT`，指数退避 0.5 到 10 秒，抖动 10%，提供方给出的等待时间在上限内优先。
- 每次重试先写 `llm/retry` 事件再等待，等待可取消。重试在同一个打开的步内重跑，不重复追加用户消息。

### 6.3 适配器

| 适配器 | 线格式 | 用途 |
|---|---|---|
| `openai_chat` | Chat Completions，SSE | DeepSeek、Ollama 的 `/v1`、vLLM 及其他兼容服务 |
| `ollama` | Ollama 原生 `/api/chat` | 本地模型。能在每次请求里显式写出 `num_ctx`、`seed`、`repeat_penalty` 等采样参数 |
| `scripted` | 无 | 测试。构造时给一个脚本数组，每次调用消费下一项，记录收到的每个请求 |
| `replay` | 无 | 测试。从录制的会话日志里取出每次调用的回复，重新拆成分片当脚本 |

`anthropic_messages` 放在第 6 阶段之后。DeepSeek Harness 的官方适配器走这个格式，Tack 先做 Chat 格式是因为本地模型都用它。

DeepSeek 的注意事项，来自官方文档：

- 思考模式默认开启。开启时 `temperature` 等采样参数不生效。
- 带工具调用时，之前每一轮的 `reasoning_content` 必须原样回传，否则返回 400。适配器把它存进 `replay_state`。
- 缓存命中的输入单价是未命中的五十分之一。上下文必须只追加，不改前缀。

### 6.4 采样参数显式化

每次请求明确写出温度、种子、重复惩罚、上下文长度，并记进 `request/header`。不依赖任何服务端默认值。

### 6.5 token 估算

`ctx.token_meter` 回放日志得出估算，不调模型。ASCII 文本按 4 个字符 1 个 token，中日韩文字按每字 1 个 token，另加块和角色开销。只有最近一次成功调用的请求信封与当前信封一致时，才用提供方报告的用量作锚点，再加上之后消息面变化的增量。

## 7. Agent 与循环

### 7.1 Agent 接口

| 方法 | 语义 |
|---|---|
| `followup(content)` | 排一个新回合 |
| `steer(content)` | 运行中在最近的步边界被消费，空闲时开新回合 |
| `inject(content)` | 只附带上下文，等别的输入唤醒 |
| `cancel(cause, keep_inbox=False)` | 中止当前活动。`cause` 取 `user`、`parent`、`hook`、`budget`、`disposed`，写进 `turn/end` |
| `run_maintenance(task)` | 在真正空闲时跑非回合任务，手动压缩用它 |
| `when_idle()` | 等到空闲 |

inbox 分 `next-turn` 和 `next-step` 两个队列。回合边界认领全部 `next-step` 加一条 `next-turn`，步之间只认领 `next-step`。每次 inbox 变更落成一条 `agent/inbox/spliced` 事件，排队的输入是日志投影出的状态。

`ctx.agents.create()` 新建，`ctx.agents.resume(session_id)` 恢复。扩展插件只依赖 `tack.agent`，不依赖 `tack.agent_loop`，所以循环可以替换。

### 7.2 回合与步

步是一次模型请求加它触发的工具调用。回合是零个或多个步。

```
turn/start
  进入前：认领输入 → 组装系统提示 → 运行时上下文快照
          → waterfall agent/pre-step ⇒ reject | enter(messages)
     reject ⇒ turn/end{blocked}；首批为空 ⇒ turn/end{completed}，不花模型调用
  step/start
     waterfall agent/request（可替换调用配置）
     对账 system/message；仅首次尝试追加 user/message；写 request/header
     由日志派生并冻结请求
     读流，逐分片发 agent/assistant-stream
     成功 ⇒ assistant/message；失败 ⇒ assistant/attempt
            → waterfall agent/request-error ⇒ 重试 | 保留原错误
     有工具调用 ⇒ 按 8.3 节调度执行
  step/end
  本步给出结束原因且 next-step 队列为空 ⇒ serial agent/turn-stopping
turn/end{reason}
```

`turn/*`、`step/*`、`*/message`、`assistant/attempt`、`tool/*` 是持久事件。`agent/*` 是进程内的实时接口。需要可回放数据的前端消费会话事件，`agent/*` 只管队列、状态、拦截和流式帧。

### 7.3 停止条件

| 条件 | 结果 |
|---|---|
| 回复里没有工具调用 | `completed` |
| 结束原因为 `max-tokens` | `max-tokens`，在回合内有粘性 |
| 工具结果带 `concludes_turn` | 提交完本批结果后结束 |
| `agent/pre-step` 返回 `reject` | `blocked`，不开步 |
| 取消 | `aborted` |
| 异常 | `error` |

回合真正关闭还要求 `next-step` 队列为空。`agent/turn-stopping` 的监听器想阻止结束时，调用 `agent.steer()` 塞一条消息，循环就再跑一步。

循环本体不设步数上限。上限由回合预算插件提供。它在 `agent/pre-step` 上检查步数、累计 token 和墙钟时间，到限后返回 `reject`，`turn/end` 写成 `blocked` 并在 `detail` 里注明是哪项预算。

### 7.4 取消与失败

- 流中途被取消时，已送达的文本落成带 `interrupted: true` 的 `assistant/message`。
- 未派发的工具调用各补一对 `tool/call` 和"未执行"的结果。
- 步失败时，给每个没有结果的工具调用补错误结果。调用与结果在任何情况下都成对。

## 8. 工具

### 8.1 定义

```python
define_tool(
    name="read",
    description="...",
    parameters=ReadArgs,            # pydantic 模型，生成给模型看的 JSON Schema
    execute=read_file,              # async (args, exec) -> 规范值
    render=render_read,             # (args, value) -> 模型可见的内容块
    timeout_ms=None,
    is_concurrency_safe=lambda a: True,
    finalize_content=None,
)
```

发给模型的模式用显式白名单构造，宿主字段不泄漏。`exec` 句柄提供 `signal`、`defer_context(text)`、`conclude_turn()`、`agent`、`session`。

`ctx.tools.register(tool)` 注册，注册是 effect。`ctx.tools.restrict(allow, deny)` 按 Agent 作用域过滤，被过滤的工具既不出现在提示里，也拒绝执行。

### 8.2 执行管线

| 阶段 | 形态 | 能做什么 |
|---|---|---|
| 记录 `tool/call` | 循环在派发前追加 | 审计和界面先看到调用 |
| `tools/pre-execute` | waterfall | 返回 `allow`、`deny`、`cancel`、`ask`。`ask` 交给 `ctx.approval` |
| 守卫 | 注册的函数 | 只能返回拒绝理由，没有"允许"这个返回值 |
| `tools/execute` | 环绕式 waterfall | 超时、度量 |
| 工具体 | `execute` | 产出规范值 |
| `tools/post-execute` | waterfall | `accept`（可替换内容，可附加上下文）或 `block`（变成错误结果） |
| `finalize_content` | 工具自带回调 | 最后一道只改内容的不变式，每种结局恰好调用一次 |
| `tools/result` | emit | 观察冻结后的最终结果 |
| 记录 `tool/result` | 循环按模型顺序追加 | |

两条不变式：

- 参数一旦记录就不能被改写。历史、审计、界面和执行必须一致。
- 一切失败都变成结构化的错误结果返回给模型，回合不中断。错误码：`UNKNOWN_TOOL`、`INVALID_ARGS`、`TOOL_ERROR`、`TOOL_TIMEOUT`、`DENIED`、`APPROVAL_REJECTED`、`APPROVAL_UNAVAILABLE`、`ABORTED_BEFORE_DISPATCH`。

### 8.3 并行

工具用 `is_concurrency_safe(args)` 自报，默认独占。独占调用是顺序屏障。连续的可并行调用进一个滚动池，上限 `max_parallel_tool_calls`，默认 10。前置策略按模型顺序串行，只有工具体重叠执行。结果按模型顺序提交。

### 8.4 内置工具

| 工具 | 说明 | 上限 |
|---|---|---|
| `read` | 带行号读文本，支持 `offset` 和 `limit` | 2000 行，每行 2000 字符，50 KiB |
| `write` | 整文件创建或覆盖 | |
| `edit` | 精确字面替换。必须唯一匹配，否则报错并要求扩大上下文。`replace_all` 可选 | |
| `glob` | 文件名匹配，按修改时间排序 | 100 条 |
| `grep` | 内容搜索。有 ripgrep 就用它，没有就用纯 Python 实现 | 250 条 |
| `bash` / `pwsh` | 每次在全新进程里执行一条命令。Windows 默认 `pwsh`，其他平台默认 `bash` | 默认超时 60 秒，上限 600 秒 |
| `todo_write` | 整体替换本会话的任务清单。条目有 `content` 和三态 `status` | |
| `skill` | 按名字加载一个技能的完整说明 | |
| `subagent` | 委派给全新上下文的子代理 | 深度 1 |
| `ask_user_question` | 向用户提问并等待回答，只在交互式前端可用 | |

写和改之前必须先读过该文件。读后文件变了就拒绝并提示重读。这条规则挂在文件系统的意图事件上，不写在工具里。

可选插件：`apply_patch`（Codex 的补丁格式，带四级模糊定位）、`web_fetch`。

### 8.5 输出上限

| 层 | 机制 | 默认值 |
|---|---|---|
| 子进程流 | 内存里保留头尾各一半，丢弃中段并插入省略标记。完整流写入私有的溢出文件 | 64000 字节 |
| 结果级 | `tools/post-execute` 监听器。超预算的结果换成头尾预览，加全文的定位符和读取提示。保存失败就保留原文 | 12500 token |
| 压力期 | 只在压缩触发后，把超大的旧工具结果换成头 4096 加尾 1024 字符 | 阈值 8192 字符 |

子进程结果把正交的事实分开上报：`timed_out`、`exit_code`、`signal` 各自成字段。

## 9. 安全与权限

### 9.1 两个旋钮

| 旋钮 | 取值 | 存储 |
|---|---|---|
| 沙箱模式 | `read-only`、`workspace-write`、`danger-full-access` | 会话事件 `sandbox/mode`，回放即恢复 |
| 审批策略 | `ask`、`never` | 会话事件 `approval/policy` |

三个预设把两个旋钮打包：`read-only`（只读加询问）、`workspace-write`（工作区可写加询问，默认）、`full-access`（完全访问加从不）。

### 9.2 文件工具

文件工具在进程内强制执行沙箱模式。`read-only` 禁止写。`workspace-write` 只能写工作区根和临时目录。`.git` 和 `.tack` 目录在任何受限模式下都受保护。路径先解析符号链接再判断。

### 9.3 Shell 命令

沙箱接缝 `ctx.sandbox` 的 `confine(argv, policy)` 返回包装后的命令和约束强度。提供者：

| 提供者 | 强度 | 阶段 |
|---|---|---|
| `none` | `none` | 第 3 阶段 |
| `docker` | `full`。每个会话或任务一个用完即弃的容器，只挂载工作目录，默认断网 | 第 7 阶段 |
| `bwrap` | `full`，用于 Linux 和 WSL | 第 7 阶段，可选 |

强度为 `none` 时，shell 命令无法被约束。因此在 `read-only` 和 `workspace-write` 模式下，shell 调用一律要审批或命中允许规则。审批策略为 `never` 时直接拒绝。当前强度作为运行时上下文告诉模型，也显示在界面上。

没有可用提供者而策略要求约束时，`confine()` 抛 `SANDBOX_UNAVAILABLE`，不静默放行。

子进程环境做清洗：变量名匹配 `KEY`、`PASSWORD`、`SECRET`、`TOKEN` 的不传给子进程。

### 9.4 规则

规则语法取自 Claude Code：`工具名` 或 `工具名(匹配串)`，例如 `bash(git status*)`、`edit(src/**)`、`read(./.env)`、`mcp__github__*`。

- 来源：用户目录的 `permissions.yml`、项目的 `.tack/permissions.yml`、会话内授权。
- 求值顺序固定为 `deny`、`ask`、`allow`，首个命中生效。
- 复合命令按 `&&`、`||`、`;`、`|` 拆开逐段判断，取最严格的结果。
- 规则实现为 `tools/pre-execute` 的监听器。钩子返回的允许不能绕过规则的拒绝。

文档写明：命令规则按文本匹配，不是安全边界。隔离靠容器。

### 9.5 审批

`ctx.approval` 的结果是封闭集合：`allowed-once`、`rejected`、`cancelled`、`unavailable`。只有 `allowed-once` 放行，并且只对这一次调用有效。没有应答者、应答者抛错或返回非法值，一律归为 `unavailable`。

每次询问写一对事件 `approval/asked`、`approval/decided`，不进模型上下文。

CLI 提供三个选项：允许一次、本会话允许、拒绝。"本会话允许"是在规则层追加一条会话级允许规则，审批接缝本身仍然只有一次性授权。

## 10. 上下文管理

### 10.1 系统提示

`ctx.system_prompt` 是段落注册表加一条 `system-prompt/assemble` waterfall。贡献分三类：段落（带 `order` 的文本）、变量（`{{name}}`）、工具模式提供者。段落按 `order` 升序、再按名字排序后拼接。

渲染是严格的。未知变量或无值变量直接抛错。渲染结果作为 `system/message` 进日志。文本没变就不写新事件。

### 10.2 运行时上下文

沙箱模式、审批策略、工作目录这类会变的事实不进系统提示。它们合成一条用户角色的快照消息，只在内容变化或被压缩遮蔽时追加新快照。目的是保住前缀缓存。

日期默认不注入。

### 10.3 指令文件

- 候选文件名默认是 `AGENTS.md`、`CLAUDE.md`。项目根由 `.git` 标记。
- 首个请求注入一条持久的用户消息：先是用户级 `<TACK_HOME>/AGENTS.md`，再是从项目根到工作目录的链，由宽到窄。
- 字节预算 65536。超预算时先丢更宽的文件，最后才截断最具体的文件，并给出可见提示。
- 文件内容里出现包裹标签的字面量时转义。
- 文件变了，下一次请求发一条替换消息。

### 10.4 溢出存盘

`ctx.spill_store.save_text()` 一个方法。本地提供者写到 `<TACK_HOME>/spill/session-<哈希>/`，目录仅属主可访问，文件随机命名、独占创建。

### 10.5 压缩

压缩是可选能力，接缝为 `ctx.compaction`，提供者 `compaction_basic`，手动入口是 `/compact` 命令。

| 项 | 内容 |
|---|---|
| 主动触发 | 在 `agent/pre-step` 上，估算 token 超过 `floor(min(W×0.8, W−O−B))`。`W` 是上下文窗口，`O` 是输出上限，`B` 是余量 |
| 余量 | `B = min(65536, W×0.25)`。原版固定 65536，窗口小的本地模型会永远触发不了 |
| 被动触发 | `agent/request-error` 收到 `CONTEXT_WINDOW_EXCEEDED`。只有消息面确实前进才授权重试 |
| 第一步 | 裁剪超大的旧工具结果，不调模型。裁完够用就跳过摘要 |
| 选区 | 从第一个非系统节点开始的最老区间。边界保持工具调用与结果成对 |
| 保留 | 最新的约 `(W−O)×16%` 原样保留 |
| 摘要调用 | 一次额外的模型请求。原样回放系统提示、工具声明和被压缩区的消息，最后追加压缩指令。指令要求写一份给接手者的交接说明：进展与决定、约束与偏好、剩余步骤、关键数据 |
| 落盘顺序 | `compaction/start`，`compaction/summary`，一条替换型的 `user/message`，最后 `compaction/end` |
| 崩溃语义 | 最后才释放锁。中途崩溃留下可检测的孤儿 `compaction/start` |

压缩后，指令文件、运行时上下文和待办清单因为被遮蔽而由各自的插件重新追加。

### 10.6 待办

`todo_write` 每次整体替换清单，状态存为 `todo/write` 事件。只影响模型看到的内容和界面显示。

## 11. 扩展机制

### 11.1 技能

- 文件格式：`<name>/SKILL.md`。frontmatter 必填 `name`、`description`，可选 `disable-model-invocation`、`user-invocable`。
- 发现根，按优先级：`<项目>/.tack/skills`、`<项目>/.agents/skills`、`<TACK_HOME>/skills`、`~/.agents/skills`，以及配置的额外目录。
- 渐进披露三层。首次请求注入一条目录消息，只含名称和最多 500 字符的简介。模型调用 `skill` 工具时才加载正文。脚本和参考文件只在正文明确引用时由模型自己去读。

### 11.2 钩子

原生的钩子就是内核事件。另提供兼容桥，读 Claude Code 格式的 `hooks.json`，只支持命令型。

| 钩子事件 | 映射到 | 能做什么 |
|---|---|---|
| `SessionStart` | `agent/created` | 附加上下文 |
| `UserPromptSubmit` | `agent/pre-step` | 拦截提示或附加上下文 |
| `PreToolUse` | `tools/pre-execute` | 拦截或请求审批 |
| `PostToolUse` | `tools/post-execute` | 拦截结果或附加上下文 |
| `Stop` | `agent/turn-stopping` | 给一个理由，强制再跑一步 |

退出码 2 表示拦截，其他非零只记日志。多个钩子的结果按拒绝、询问、允许的严格程度合并。每次运行写一对 `hook/invoked`、`hook/result` 事件。任何失败都降级处理，不向循环抛异常。

### 11.3 子代理

- 接缝 `ctx.subagents`，提供者 `spawn`：进程内，空白上下文。
- 每个子代理是一个独立会话，头里记 `parent_session` 和 `delegation_depth`。子代理拿到全新的作用域，不继承父代理的注册。
- 结果是子代理最后一条非空回复。非 `completed` 的结果映射成工具错误，不把部分输出当成功。
- 子代理类型用 Markdown 加 frontmatter 定义，放在 `.tack/agents/`。可指定 `tools`、`max_steps`、`model`。
- 上限：委派深度 1，同时运行数默认 4。子代理固定为审批策略 `never`，不能调用 `ask_user_question`。

### 11.4 MCP 客户端

- 每个服务器一个清单条目。传输是 stdio 或 streamable HTTP，协议层用官方 `mcp` SDK。
- 工具名 `mcp__<服务器名>__<原名>`，命名空间取本地配置的服务器名。
- MCP 工具走和原生工具相同的管线。单次调用超时默认 60 秒。断线后指数退避重连。
- 服务器的说明作为带归属的系统提示段，有字节上限。

### 11.5 命令与插件

人用的斜杠命令走 `ctx.commands`，直接执行，不产生模型消息：`/help`、`/compact`、`/model`、`/permissions`、`/tools`、`/todos`、`/sessions`、`/fork`、`/exit`。

树外插件是普通的 Python 模块，在清单里用导入路径引用。

## 12. 前端

### 12.1 启动器

- `tack`：进入终端 CLI。
- `tack run "任务"`：无界面，跑一个任务。
- `tack bench ...`：跑基准。
- `tack sessions list|show|fork`：管理会话。
- `tack doctor`：自检密钥、模型可达性、shell、ripgrep、容器。
- 通用参数：`--profile`、`--patch`、`--model`、`--permission`、`--dump-config`。

### 12.2 无界面

最终回答写到标准输出。`--json` 输出逐行事件：`session`、`status`、`text`、`thinking`、`tool_call`、`tool_result`、`final`。`final` 带结束原因和用量。`--session-id` 续接已有会话。退出码成功为 0，其余为 1。

### 12.3 终端 CLI

- 流式文本、工具调用卡片（名称、参数摘要、状态、耗时）、编辑的差异预览、待办面板。
- 审批提示：允许一次、本会话允许、拒绝。
- 状态行：模型、上下文占用、沙箱模式与强度、缓存命中率。
- 运行中输入的内容作为 `steer` 插入。Esc 取消当前动作。
- 输入用 `prompt_toolkit`，渲染用 `rich`。

### 12.4 之后

第 10 阶段加 JSON-RPC 2.0 的 stdio 服务（请求 `initialize`、`session/prompt`、`shutdown`，通知 `session.event`、`session.status`）和 Web 界面。Web 的设计届时单独做。

## 13. 质量保障

### 13.1 测试分层

| 层 | 做法 |
|---|---|
| 单元 | 内核、日志折叠、路径隔离、编辑工具、截断、权限规则、分片装配等 |
| 循环 | 用脚本化适配器驱动真实的工具管线和 JSONL 持久化。只替换模型、网络和时钟 |
| 性质 | 用 hypothesis 随机生成输入与取消的时序。不变量：每条输入在日志里恰好出现一次，回合号严格递增，工具调用与结果始终成对，状态只在空闲和运行之间切换 |
| 回放 | 录制的真实会话既是输入也是期望输出，不需要密钥 |
| 请求快照 | 把每次请求渲染成"序号、角色、内容"的行并存成快照，断言第 n 次请求里模型看到了什么 |
| 卸载 | 每个注册表一条测试：销毁贡献它的插件实例，断言清理干净 |
| 适配器恢复 | 本地模拟 SSE 服务器，编排断流、挂起、畸形 JSON、429、5xx |
| 真实接口 | 有密钥时对真实模型跑，没有就跳过 |

两条纪律。断言要重新读文件或重跑命令来验证结果，不信 Agent 的自述。产品可见的插件必须有一条经真实清单加载的测试。

### 13.2 闸门

- `ruff` 检查和格式化。
- `pyright` 严格模式。
- `src/tack/kernel`、`session`、`llm`、`agent`、`agent_loop`、`tools` 的行覆盖率不低于 90%。
- GitHub Actions 矩阵：Windows、Ubuntu、macOS，Python 3.11 和 3.12。

## 14. 基准

### 14.1 验收基准

四类任务，每类都能自动判分。每个任务在全新的临时工作区里运行。

| 类别 | 内容 | 判分 | 规模 |
|---|---|---|---|
| 代码修改 | 小型 Python 仓库，给出问题描述和可见的失败测试 | 运行隐藏测试 | 约 15 个 |
| 文件与命令行操作 | 整理目录、改配置、合并文件等 | 检查最终的文件树和内容 | 约 12 个 |
| SQL 数据查询 | 带种子生成的 SQLite 库和自然语言问题。工具有查表结构、只读查询、提交答案 | 比对结果集；另在一份扰动过的库上复核，防止硬编码 | 约 15 个 |
| 多步工具计算 | 由 ReliQuant 的程序生成。题面中的数值藏在查询工具后面，必须调用工具才能得到 | 核对最终答案和必需的查询调用 | 可大样本 |

每个任务带参考解。校验命令检查两件事：初始状态判为失败，应用参考解后判为通过。

每次发布前在 DeepSeek Flash 上跑，成功率不得低于上一版。

### 14.2 部件开关表

对每个可选部件，比较开与关时的成功率和 token 成本，一张表。部件包括压缩、待办、工具调用修复、重复调用提醒、溢出预览。用途是决定默认开哪些。

### 14.3 本地模型支持矩阵

Qwen2.5 和 Llama 的各量化档在验收基准上的成功率，标出哪些档位可用。用独立的 Ollama 实例，模型目录指向 `D:\ollama-models`，不改动现有的 Ollama 设置。

### 14.4 性能与成本

- 性能：冷启动时间、一千步合成长会话的恢复与折叠耗时、内存占用。输入全部由常量合成。设预算并放进持续集成。
- 成本：每个任务的 token 数和缓存命中率。

## 15. 交付物与阶段

### 15.1 交付物

- GitHub 公开仓库和 PyPI 包，MIT 协议，中英文 README，演示动图，更新日志。
- 文档：架构说明、配置参考、插件开发指南。
- 一份 4 到 6 页的工程报告，讲架构、设计取舍和基准结果。
- Web 界面。

### 15.2 阶段

| 阶段 | 内容 | 结束标志 |
|---|---|---|
| 1 | 仓库、工具链、内核 | 内核测试全绿，三平台持续集成跑通 |
| 2 | 会话日志、模型适配、工具管线、循环、无界面模式 | `tack run` 用 DeepSeek Flash 完成一次带工具调用的任务；同一流程有脚本化测试 |
| 3 | 文件与命令工具、权限与审批、系统提示、指令文件 | 无界面模式能读、改、测一个小仓库 |
| 4 | 终端 CLI | 第一个可日常使用的版本 |
| 5 | 上下文管理、恢复与分叉 | 长会话能自动压缩；进程被杀后能恢复 |
| 6 | 技能、钩子、子代理、MCP | 四项各有一个端到端示例 |
| 7 | 容器沙箱 | shell 工具在容器内执行，断网 |
| 8 | 基准与本地模型支持矩阵 | 三张结果表 |
| 9 | 打包发布、文档、工程报告 | PyPI 上可安装 |
| 10 | JSON-RPC 服务与 Web 界面 | 之后由作者做 SUS |

## 16. 风险与未决事项

| 事项 | 处理 |
|---|---|
| Windows 上 shell 没有操作系统级约束 | 第 7 阶段前靠审批和规则。界面如实显示强度为"无" |
| 现有的 Ollama 读的是 C 盘空目录 | 基准阶段单独起一个指向 D 盘的实例 |
| DeepSeek 思考模式下采样参数不生效 | 基准中记录每次请求的完整配置；是否关闭思考在基准阶段决定 |
| 规模约一万行，周期数周 | 每个阶段结束都能运行和演示 |
| PyPI 上 `tack` 已被一个 2017 年的占位包占用（版本 0.0.0） | 发行包名用 `tack-harness`，导入名仍是 `tack`。发布前再确认一次名字可用 |
| 本地小模型在手工任务上可能几乎全错 | 任务分难度档；先在 Qwen2.5-7B 上试跑，把难度调到可区分的范围 |

## 17. 参考资料

两个参考仓库的本地克隆放在本仓库目录之外（`../_reference/`），只读，不入库。

- `docs/research/dsh-digest.md`：DeepSeek Harness，提交 `5badb15`。
- `docs/research/codex-digest.md`：Codex CLI，提交 `8c818cbed`。
- `docs/research/claude-code-digest.md`：Claude Code 公开文档与 Anthropic 工程文章。
- *A Programming Paradigm for Spatiotemporal Composability*，arXiv:2608.25512。Cordis 的设计论文。
- *Harness Engineering: Anatomy, Architecture, and Evolution of Coding Agents*，arXiv:2609.00006。十一个系统的源码研究。
- *Dive into Claude Code: The Design Space of Today's and Future AI Agent Systems*，arXiv:2604.14228。
- DeepSeek API 文档：https://api-docs.deepseek.com/
