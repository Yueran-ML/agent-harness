# DeepSeek Harness (dsh) 设计摘要

读取版本：commit `5badb15009ae1756c3afe0ae0cef1faafc290ccc`（见 `.git/shallow`），根 `package.json` 版本 `0.2.1-alpha.1`。全程只读，未执行仓库内任何代码。

1. dsh 是 TypeScript monorepo。一切能力都是 Cordis 插件，包括模型适配器、工具注册表、会话日志和 agent 循环本身。
2. Cordis 内核很小，约 2700 行。核心只有四件事：Context、服务注入、可撤销的 effect、五种事件派发模式。
3. 默认产品是一份 94 个条目的 YAML 清单（`dsh-base`）。用户用补丁按 id 替换或插入条目。
4. 会话是只追加的事件日志。模型上下文和 UI 状态都由日志折叠得出，不另存一份消息历史。
5. 循环驱动只有约 690 行。结构是 turn 包含若干 step，每个 step 是一次模型请求加它触发的工具调用。
6. 循环没有内置步数上限，也没有强制的循环检测。防护靠外挂插件和少量拦截点。
7. 官方 DeepSeek 适配器走 Anthropic Messages 线格式，不是 OpenAI Chat Completions。
8. 安全模型是两个独立旋钮：文件效果沙箱模式和一次性审批。没有命令黑白名单。
9. Windows 有沙箱后端，但只约束写和删除，并且自报强度为 `partial`。
10. 默认组合会把会话日志增量上传到 DeepSeek API 端点（`dsh_session_log`，默认开启）。

阅读说明：
- 来源写仓库相对路径，代码带行号。只来自文档或 README、没有打开对应源码的结论标"仅文档"。
- 文档与代码冲突时以代码为准，汇总在附录 A。没能核实的内容汇总在附录 B。
- 规模（`wc -l` 实测）：`vendor/cordis/src` 9 个文件 2696 行，`packages/core/agent-loop/src` 7 个文件 2425 行，`packages/core/session/src` 3153 行，`packages/core/tools/src` 5670 行。`packages/` 下 54 个分组目录，约 319 个子包目录。

## 1. 内核（Cordis）

Cordis 是 dsh 底下的插件框架，源码以 vendored 方式放在仓库里（上游 `cordis` 4.0.0-rc.7，见 `vendor/README.md` 的清单表）。产品的每个部分都是插件，没有需要打补丁的特权核心（`docs/architecture.md:11-13`）。

### 1.1 内核提供什么
| 概念 | 作用 | 来源 |
|---|---|---|
| Context | 服务容器，本身是 Proxy。`extend()` 用原型继承造子上下文，`isolate(name)` 让某个服务名在子树内独立解析，`intercept()` 叠加服务配置 | `vendor/cordis/src/context.ts:74,99-145` |
| Service | `ctx.provide(name, value)` 把实现挂到 `ctx.<name>`。注册是当前 fiber 的 effect，同一作用域重复注册抛错 | `vendor/cordis/src/reflect.ts:277-305` |
| Plugin 与 Fiber | 插件是函数、类或带 `apply(ctx, config)` 的对象，可声明 `inject` 和 `Config`。每次加载得到一个 Fiber，状态为 `PENDING/LOADING/ACTIVE/FAILED/UNLOADING/DISPOSED` | `vendor/cordis/src/registry.ts:316-336`，`vendor/cordis/src/fiber.ts:147-154` |
| Effect | `ctx.effect(fn)` 立即执行并收集 disposer。同一个 effect 内按注册逆序撤销，重复调用 disposer 无副作用 | `vendor/cordis/src/fiber.ts:418-442` |
| Events | `ctx.on()` 注册监听，监听本身也是 effect。派发时可带一个过滤器，只放行部分监听器 | `vendor/cordis/src/events.ts:165-175,254-302` |

| 派发模式 | 行为 | 来源（`vendor/cordis/src/events.ts`） |
|---|---|---|
| `emit` | 同步通知，忽略返回值 | `:194-196` |
| `parallel` | 并发执行并等待全部结束，失败聚合成 `AggregateError` | `:183-187` |
| `serial` | 按序 await，遇到非空返回值就停 | `:204-209` |
| `bail` | 同步按序，遇到非空返回值就停 | `:217-222` |
| `waterfall` | 环绕式中间件。监听器收到 `next`，不调用 `next()` 就是短路或否决 | `:234-243`，`docs/cordis-primer.md:31-35` |

依赖由服务名驱动，不靠手工排启动顺序：
- 插件声明 `inject`。fiber 的 epoch 是它所需各服务的提供者 fiber uid 串，缺一个就是 `INACTIVE`（`vendor/cordis/src/fiber.ts:611-623`）。
- epoch 变化触发卸载再加载（`vendor/cordis/src/fiber.ts:625-639,646-696`）。提供者被撤销时，`provide` 的 disposer 先删条目，再通知并等待所有依赖方（`vendor/cordis/src/reflect.ts:297-303,314-336`）。
- 没声明 `inject` 就读服务会抛错（`vendor/cordis/src/reflect.ts:144-166`）。缺提供者的插件停在 `PENDING`，不报错（`docs/cordis-tutorial/06-composition-and-hmr.md:63`）。

### 1.2 能力接缝（capability seam）
一个接缝有三个角色：Service Definition（拥有 `ctx.<key>` 和词汇类型，是抽象类或注册表，不是 TS interface）、Service Provider（实现）、Consumer（通常是面向模型的工具）。只有一个角色不算接缝（`docs/glossary.md:9`，`docs/architecture.md:133`）。规则是扩展插件只依赖 Definition，不依赖具体 Provider（`packages/README.md:100`）。

| 接缝 | Definition | Provider | Consumer |
|---|---|---|---|
| `ctx.shell` | `dsh-shell` | `dsh-bash-local`、`dsh-bash-sandbox`、`dsh-pwsh-local` | `dsh-tool-bash`、`dsh-tool-pwsh`、两个 hook 桥 |
| `ctx.llm` | `dsh-llm` | `dsh-llm-deepseek`、`dsh-llm-pi-ai`、`dsh-llm-replay`（测试） | `dsh-agent-loop`、`dsh-compaction-basic` |
| `ctx.sandbox` | `dsh-sandbox` | `dsh-sandbox-local`、`dsh-sandbox-ssh` | `dsh-bash-sandbox`、`dsh-terminal-bash` |
| `ctx.fs` | `dsh-fs` | `dsh-fs-local`、`dsh-fs-sandbox`、`dsh-fs-ssh` | `dsh-tool-fs` |
| `ctx.sessionPersistence` | `dsh-session-persistence` | `dsh-session-persistence-jsonl` | `dsh-agent-loop` 等 |
| `ctx.subagents` | `dsh-subagent` | spawn、fork、acp、codex、claude-code、dsh-sdk 六个后端 | `dsh-tool-subagent` 等 |

来源是 `docs/capability-seams.md:572-665` 的生成表。效果是换一个 Provider 就整体换执行世界：文件系统和子进程指向远端后，Bash、PTY、LSP 一起迁移（`docs/architecture.md:135`）。

### 1.3 组合与热增删
- 运行中的 dsh 是一棵启动时组合出的插件树。profile 列出若干 bundle，再叠加 profile 级、home 级和 `--patch` 补丁（`docs/architecture.md:17-27`）。
- bundle 是一份 YAML 条目清单。`dsh-base` 一次插入 94 个条目。补丁按 `id` 定位条目，整体替换它的 `config`，或插入新条目（`packages/bundle/base/cordis.patch.yml:1-13`，条目数为 `grep -c` 实测）。
- 条目顺序没有加载语义，激活由服务可用性驱动（`packages/bundle/base/cordis.patch.yml:12-13`）。`disabled: !!js <表达式>` 在每次挂载决策时求值，用来做平台分支（`docs/cordis-primer.md:39`，`packages/bundle/base/cordis.patch.yml:237,243`）。
- 配置文件变更时，loader 按 `id` 做差分，只挂载、卸载或重配变化的条目。HMR 就是卸载旧实例再加载新代码（`docs/cordis-tutorial/06-composition-and-hmr.md:25,59`）。

热增删安全的原因：
1. 所有注册（工具、提示段、适配器、监听器）都走 effect 或 `ctx.on()`，卸载时自动撤销（`docs/cordis-primer.md:13,45`）。
2. 提供者消失时依赖方先卸载，提供者回来后自动重载（见 1.1）。
3. fiber 处于 `UNLOADING` 时拒绝新建 effect，清理期的注册逃不出卸载快照（`vendor/cordis/src/fiber.ts:419-422`，`vendor/README.md` 本地修改第 6 条）。
4. `dispose()` 等到异步清理真正结束才返回（`vendor/cordis/src/fiber.ts:293-295`）。
5. 标记为 volatile 的配置字段可以不重挂插件直接生效，例如 `maxParallelToolCalls`（`vendor/README.md` 本地修改第 22 条，`packages/core/agent-loop/README.md:57`）。
6. 每个注册表都要有一个 HMR 安全测试：销毁贡献方 fiber，断言清理干净（`docs/testing.md:9`）。

一个容易踩的细节：同一 fiber 的多个顶层 effect 在卸载时按逆序启动，但并发执行，不串行等待（`vendor/cordis/src/utils.ts:27-31`，`vendor/cordis/src/fiber.ts:675-686`）。需要严格顺序的清理必须放进同一个 effect（`docs/cordis-primer.md:45`）。

每个 agent 还有自己的 scope。`createScope` 为 agent 挂一个空插件 fiber，并给上下文打标签。经 `agent.ctx` 做的注册只对该 agent 可见，随 agent 销毁一起撤销。事件派发用 `scopeTarget` 过滤监听器（`packages/core/scope/src/index.ts:137-147,170-185`，`docs/glossary.md:13-21`）。

### 1.4 用约 300 行 Python 复刻同样形状的最小概念集（本小节是分析，不是仓库事实）
1. `Context`：带 `parent` 链的对象，持有 `owner`（当前 fiber）。服务查找沿链向上。
2. `Fiber`：`inject` 列表、状态机（至少 `PENDING/ACTIVE/DISPOSED`）、一个 disposer 栈。`contextlib.AsyncExitStack` 正好是逆序撤销的 effect 栈。
3. `provide(name, value)` 与 `get(name)`：注册即 effect。服务增删时重算依赖方是否满足，满足就 `apply`，不满足就卸载。
4. 事件总线：`on(name, fn, prepend=False)` 返回 disposer。实现 `emit`、`serial`、`waterfall` 三种就够，`waterfall` 用闭包传 `next`。
5. 派发时的 scope 过滤：监听器带一个可选的 agent 标签，派发时只放行无标签和标签匹配的监听器。

可以不做：Proxy 属性魔法、`isolate` 标签、`intercept` 配置、mixin、调用栈追踪、YAML 补丁 loader、文件监听式 HMR。

## 2. Agent 生命周期

### 2.1 turn 与 step
step 是一次模型请求加上它触发的工具调用。turn 是零个或多个 step，在认领第一条输入之前打开，在没有待办时关闭（`docs/architecture.md:86`）。goal round 这类 round 是更外层的策略迭代，不属于循环本身（`docs/glossary.md:37-39`）。

驱动的实际结构（行号指 `packages/core/agent-loop/src/agent.ts`）：

```text
kick():        while (await turn()) {}                                        # :252-265
turn():        append turn/start                                              # :305
  loop:
    preStep(): 认领 inbox -> systemPrompt.assemble -> 运行时上下文快照
               -> waterfall agent/pre-step => reject | enter(messages)        # :267-286
    reject => turn/end{blocked}；首批为空 => turn/end{completed}，不花模型调用   # :317-327
    append step/start                                                         # :329
    step():    waterfall agent/request -> llm.prepareCall                     # :547-596
               对账 system/message；仅首次尝试追加 user/message                # :410-423
               按需追加 request/header、request/context；派生并冻结请求         # :599-687
               读流，逐 chunk 发 agent/assistant-stream                        # :436-443
               成功 => append assistant/message；失败 => append assistant/attempt
                      -> waterfall agent/request-error => retry | 抛错        # :487-529
               有 tool-call => executeToolCalls                               # :532-538
    append step/end                                                           # :356
    本步给出结束原因且 next-step 队列为空 => serial agent/turn-stopping         # :359-363
finally:       append turn/end{reason}                                        # :385
```

循环本身是插件。`AgentLoop` 是普通的 Cordis Service，`inject` 为 `agents/sessions/llm/tools/systemPrompt/sessionProjections`，启动时用一个 effect 把自己注册成 `ctx.agents` 的工厂（`packages/core/agent-loop/src/index.ts:330-331,369`）。`dsh-base` 把它列为一个普通条目（`packages/bundle/base/cordis.patch.yml:511-514`）。扩展插件只依赖 `dsh-agent`（接口、注册表、`agent/*` 事件），不依赖 `dsh-agent-loop`，所以循环可以替换（`docs/subsystems/core.md:20`）。

### 2.2 输入、转向与中断
| 方法 | 目标队列 | 唤醒驱动 | 语义 | 来源（`agent.ts`） |
|---|---|---|---|---|
| `followup` | `next-turn` | 是 | 排一个新 turn，独占该 turn 的普通输入 | `:163-165` |
| `steer` | `next-step` | 是 | 运行中在最近的 step 边界被消费，空闲时开新 turn | `:167-169` |
| `inject` | `next-step` | 否 | 只附带上下文，等别的输入唤醒 | `:171-173` |

- turn 边界认领全部 `next-step` 加一条 `next-turn`，step 之间只认领 `next-step`（`docs/subsystems/core.md:275`）。
- 每次 inbox 变更都落成一条持久的 `agent/inbox/spliced` 事件。排队的输入是日志投影出的状态，没有活的 agent 也能读到（`packages/core/agent-loop/src/inbox.ts:198-243`，`docs/architecture.md:111`）。
- `cancel(cause, {keepInbox})` 中止当前活动，默认清空 inbox。cause 是封闭联合 `user/parent/hook/disposed`，会写进 `turn/end`（`agent.ts:80-95,175-181`）。
- 流中途被取消时，已送达的文本和推理前缀落成带 `interrupted: true` 的 `assistant/message`。未派发的工具调用各补一对 `tool/call` 和 `ABORTED_BEFORE_DISPATCH` 结果（`agent.ts:448-465`，`packages/core/agent-loop/src/tool-calls.ts:250-260`）。
- `runMaintenance(task)` 在真正空闲时跑非 turn 任务，手动压缩用它（`agent.ts:183-204`，`docs/subsystems/compaction.md:84`）。

### 2.3 停止条件、继续与 goal 模式
| 条件 | 结果 | 来源 |
|---|---|---|
| assistant 消息没有 tool-call | step 返回 `completed` | `agent.ts:532-533` |
| finish 为 `max-tokens` | step 返回 `max-tokens`，这个原因在 turn 内有粘性 | `agent.ts:336-341,530` |
| 工具结果带 `concludesTurn` | 提交完本批结果后结束 | `agent.ts:534-538`，`docs/subsystems/tools.md:243-251` |
| `agent/pre-step` 返回 `reject` | `turn/end{blocked}`，不开 step | `agent.ts:317-320` |
| 取消或异常 | `turn/end{aborted}` 或 `turn/end{error}` | `agent.ts:366-381` |

- turn 真正关闭还要求 `next-step` 队列为空。`agent/turn-stopping` 的监听器若想阻止结束，调用 `agent.steer()` 塞一条消息，循环就再跑一步。决定权在数据，不在监听器顺序（`docs/subsystems/core.md:1035-1057`）。Claude Code 的 `Stop` hook 就是这样映射的（`packages/hooks/hooks-claude-code/README.md:85` 起的 Hook point mapping 一节）。
- 循环没有内置的 turn 预算或步数上限。要限制失控的 turn，只能从扩展点取消（`packages/core/agent-loop/README.md:217`）。
- goal 模式是循环之外的策略。`dsh-goal` 把一个持久目标存成 `goal/change` 事件。`dsh-goal-round-driver` 在 agent 空闲、目标 active 且已 armed、轮数未用完时排入一条 goal 来源的消息。默认上限 256 轮。恢复或 fork 后目标保持 disarmed，要人授权才继续（`packages/goal/goal/README.md:12`，`packages/goal/goal-round-driver/README.md:12`，`docs/glossary.md:25-27`，仅文档）。

## 3. 会话模型

### 3.1 事件信封与事件类型
`Session` 是只追加的类型化事件日志，是唯一事实来源。模型消息历史由日志派生，不另存（`docs/subsystems/session.md:5`）。每条事件有 `type`、`seq`（等于追加时的日志长度）、`time`（毫秒）、`data`，可选 `ignorable`。只有产生消息的事件带 `surfaceOp` 和 `sourceEventSeqs`（`packages/core/session/src/index.ts:740-746`，`docs/subsystems/session.md:266-289`）。

`append()` 在入口处校验：数据必须是无损 JSON，快照后深冻结，再校验 surface 约束，然后同步通知观察者。观察者抛错被隔离，不影响已提交的事件（`packages/core/session/src/index.ts:718-769`）。

核心包声明 14 种事件（`packages/core/session/src/types.ts:288-427`）：

| 事件 | 载荷要点 | 进入模型历史 |
|---|---|---|
| `turn/start`、`turn/end` | `turn`；`turn/end` 带 `reason` | 否 |
| `step/start`、`step/end` | `turn`、`step` | 否 |
| `system/message` | 渲染后的系统提示，surface 节点 0 | 是 |
| `user/message` | 人的输入、注入的上下文、goal 轮次，靠 `source.kind` 区分 | 是 |
| `developer/message` | 工具增删的增量说明 | 是 |
| `assistant/message` | 组装后的消息、`usage`、原始 chunk 流 `stream`、可选 `interrupted` | 是（空内容除外） |
| `assistant/attempt` | 失败、重试、取消的尝试，只存 `stream` | 否 |
| `tool/call` | `callId`、`name`、模型原样产出的 `arguments` 字符串 | 否 |
| `tool/result` | 结果消息、可选 `error{name,code,reason}`、可选 `meta` | 是 |
| `request/header` | 调用配置加工具 schema 的完整快照，带 `reason` | 否 |
| `request/context` | 路由的 provider、model、`contextWindow` | 否 |
| `session/end-seed` | 继承或恢复的历史与本生命周期工作的分界 | 否 |

- `turn/end.reason` 取值：`completed/aborted/blocked/error/max-tokens/interrupted/forked`。后两个只由崩溃修复和 fork 种子合成，循环不会现场发出（`docs/subsystems/session.md:698-726`）。
- 插件用声明合并追加事件类型。生成的目录共列出 59 种持久事件（`docs/persistence-catalog.md` 根表的 `event:` 行，`grep` 计数）。除上表外的族：`agent/inbox/spliced`、`approval/*`、`command/*`、`compaction/*`、`goal/change`、`hook/*`、`image/offload`、`llm/retry*`、`model/selection`、`permission/preset`、`plan/mode`、`sandbox/mode`、`schedule/change`、`session/title*`、`subagent/*`、`team/*`、`todo/write`、`tool/ptc-dispatch*`、`tool-workflow/*`、`workspace/changes` 等。

### 3.2 surface：从事件到模型上下文
- `surfaceOp` 只有两种：`'append'` 和 `{op:'replace', startSeq, endSeq}`。replace 用新节点顶替一段现有节点，并且必须在 `sourceEventSeqs` 里引用全部被遮蔽的节点（`docs/subsystems/session.md:333-338`）。
- `deriveMessages()` 沿 surface 节点顺序把事件投影成消息。纯尾部增长是增量的，出现 replace 或内容投影变化时才重建。返回的消息深冻结（`packages/core/session/src/index.ts:856-877`）。
- 压缩和工具结果裁剪都只做 replace，原始事件永远留在日志里。面向人的完整对话是另一条投影，读 append 来源的事件（`docs/subsystems/session.md:358`）。
- 请求信封也是日志状态：`request/header` 记录配置和工具 schema，系统提示是 `system/message`。所以每个请求都是日志的纯函数（`docs/subsystems/session.md:181-183`）。设计者的原话是 "Model-visible means logged."（`docs/architecture.md:127`）。

### 3.3 磁盘布局、格式与迁移
- 唯一的一方后端是 `dsh-session-persistence-jsonl`。布局为 `<root>/--<规范化 cwd>--/<转义后的 session id>/session[.vN].jsonl[.zstd]`（该包 README 的 On-disk layout 一节）。`dsh-base` 的 root 是 `dshHomePath('sessions')`（`packages/bundle/base/cordis.patch.yml:130-133`）。
- 文件名规则：v0 是 `session.jsonl`，之后是 `session.vN.jsonl`（`packages/session/session-format/src/filename.ts:14-17`）。
- 首行是头，之后一事件一行。头的形状见测试夹具 `snapshots/sdk/multi-turn/session.v2.jsonl:1`：一个带 `type: "session"`、`version`、`id`、`createdAt`、`cwd`、`isSeeded` 的 JSON 对象。夹具省略了事件行的 `seq` 和 `time`（`docs/testing.md:17`）。
- 默认每个追加批次写成一个带校验和的 Zstandard 帧并 `fsync`。`compression: 'none'` 时是可直接阅读的文本行。物化是惰性的，空会话不落盘（同 README 的 Durability 一节，仅文档）。
- 写入是后台批处理，`session/flush` 是持久化屏障。`dsh-session-checkpoint-policy` 在模型请求前、顶层工具体执行前、下一步派生前各 flush 一次，flush 失败就不继续（`docs/subsystems/persistence.md:106`，`packages/session/session-checkpoint-policy/README.md:12`，仅文档）。
- 当前写入格式版本是 4（`packages/core/session/src/types.ts:89`）。已发布的最高版本记录为 3（`docs/session-format-status.md:44-47`）。
- 迁移是一串相邻步骤，每步一个包：`session-format-v0-to-v1` 到 `session-format-v3-to-v4`。旧代文件不改名、不覆盖、不删除，写打开时在旁边发布新一代文件。遇到未来版本直接拒绝（`docs/architecture.md:125`）。
- 读者遇到不认识、又没有 `ignorable: true` 的事件，必须拒绝重建会话（`docs/subsystems/session.md:274-284`）。

### 3.4 投影（UI 状态与派生状态）
`ctx.sessionProjections` 是统一的折叠接缝。每个领域注册一个纯同步单元：`init`、`apply(state, event)`、可选的 `wire.view(state)`、`stateVersion`。注册表只订阅一次 `session/event`，把每条已提交事件喂给所有单元。不关心该事件的单元必须返回同一引用，引用不变就没有下游工作。客户端不自己折叠领域事件，只收算好的整值（`docs/subsystems/session-projection.md:5,22-67,102-106`，仅文档）。

用到它的地方：inbox 和 turn 边界（`packages/core/agent-loop/src/index.ts:364-365`）、重试计数（`packages/llm/llm-retry/src/index.ts:125-138`），以及文档提到的 goal、plan、permissions、子代理目录、token 用量。投影状态按会话做检查点缓存，base 配置为每 200 个事件或 5 秒写一次（`packages/bundle/base/cordis.patch.yml:182-186`）。

### 3.5 fork、resume、回滚与 session-query
- fork：`ctx.sessions.fork(source, boundary?, childId?)` 复制到 `boundary`（含）为止的前缀，追加带 `inherited: true` 的 `session/end-seed`，再用合成事件关闭未闭合的尾部（`packages/core/session/src/index.ts:1251-1270`，`packages/core/session/src/fork.ts:21-30`）。
- resume：`ctx.agents.resume()` 以写句柄打开存储，读出物理有效的日志。崩溃在 turn 中途时追加 `interruptedTurnClosers`：给未答复的工具调用补错误结果，再补 `step/end` 和 `turn/end{interrupted}`（`packages/core/session/src/repair.ts:53-98,209-211`，`docs/subsystems/persistence.md:110-112`）。
- 修复时区分两种工具状态：有 `tool/call` 记录但没有结果是 `TOOL_OUTCOME_UNKNOWN`，连调用记录都没有是 `TOOL_NOT_STARTED`。结果文本告诉模型只有只读或幂等操作才可重试（`packages/core/session/src/repair.ts:15-41,157-197`）。
- 单写者：同一 session 的第二个写打开会被拒绝（`docs/subsystems/persistence.md:11`）。
- 回滚：在 `docs/subsystems/` 以及 `packages/core`、`packages/session`、`packages/api/session-controller` 的源码和 README 中搜索 `rewind`、`undo`，没有会话级回滚或撤销 API。等价做法是从更早的 `seq` 边界 fork 出新会话。
- session-query：`ctx.sessionQuery` 提供跨会话的精确读取、过滤和血缘追踪。SQLite 提供者增加全文检索。`dsh-base` 把它配成 `openAt: never`，即默认不建全文索引（`docs/subsystems/session-query.md:5`，`packages/bundle/base/cordis.patch.yml:141-153`）。面向模型的五个 `session_*` 工具是可选包（`docs/tool-catalog.md:39`）。

## 4. LLM 层

### 4.1 内部词汇
- 消息由内容块组成：`text`、`reasoning`、`image`、`file`、`tool-call`，外加 developer 消息专用的 `tool-addition`、`tool-removal`。工具结果是独立的 tool 角色消息（`docs/subsystems/llm-streaming.md:23-34`）。
- 适配器只需实现 `stream(options)`，产出 `StreamChunk`：`block-start`、`text-delta`、`reasoning-delta`、`tool-call-delta`、`block-end`、`usage`、`finish`（`docs/subsystems/llm-streaming.md:203-216`）。
- 共享的 `BlockAssembler` 把 chunk 折回内容块。`finish` 为 `max-tokens` 时丢弃全部工具调用，因为被截断的调用不安全（`packages/llm/llm/src/assembler.ts:137`，`docs/subsystems/llm-streaming.md:368`）。
- finish 原因：`stop`、`tool-calls`、`max-tokens`、`aborted`、`error`（`docs/subsystems/llm-streaming.md:651-657`）。

### 4.2 线格式与适配器
| 适配器 | 线格式 | 来源 |
|---|---|---|
| `dsh-llm-deepseek`（官方路由 `deepseek-official`） | Anthropic Messages。默认根地址 `https://api.deepseek.com/anthropic`，POST 到 `<根>/v1/messages`，头 `anthropic-version: 2023-06-01`，SSE 事件 `message_start`、`content_block_*`、`message_delta`、`message_stop` | `packages/llm/llm-deepseek/src/config.ts:106`，`messages-api.ts:14-17`，`adapter.ts:120-131`，`translate.ts:104-166` |
| `dsh-llm-pi-ai`（多提供方） | 借第三方库 `@earendil-works/pi-ai`，支持 `openai-completions`、`openai-responses`、`anthropic-messages` 三种 | `packages/llm/llm-pi-ai/src/provider.ts:23-25,48-50` |
| `dsh-llm-replay`、`MockAdapter` | 测试用，见第 13 节 | `packages/test-support/llm-replay/README.md:12` |

### 4.3 推理、工具调用与用量
- 推理：`thinking` 块映射为 `reasoning` 块。签名 `signature` 放进适配器私有的 `replayState`，下次请求原样带回（`packages/llm/llm-deepseek/src/translate.ts:51-54,78-80`，`serialize.ts:32-33`）。`replayState` 只交还给产生它的同一个适配器实例（`docs/subsystems/llm-streaming.md:306`）。
- 推理强度：`off/low/high/max`。`off` 发 `thinking.type: disabled`，其余发 `output_config.effort`。生成标题的请求强制关闭推理（`packages/llm/llm-deepseek/src/serialize.ts:146-155`）。
- 工具参数全程保持原始 JSON 字符串（`docs/subsystems/llm-streaming.md:299`）。
- 没有 JSON 修复。DeepSeek 适配器在非截断结束时校验每个工具参数是合法的 JSON 对象，否则抛 `MALFORMED_RESPONSE`（`packages/llm/llm-deepseek/src/translate.ts:151-157`）。循环侧解析失败时把原文当字符串传下去，再由工具的参数校验变成 `INVALID_ARGS` 错误结果（`packages/core/agent-loop/src/tool-calls.ts:104-111`，`docs/subsystems/tools.md:159`）。
- 用量字段互不重叠：`inputTokens` 只含未命中缓存的输入，缓存读写单列（`docs/subsystems/llm-streaming.md:336`，映射见 `packages/llm/llm-deepseek/src/translate.ts:34-43,159`）。

### 4.4 错误、超时与重试
- 所有失败归一成 `LlmFailure{message, code, status?, providerRetryAfterMs?, requestId?}`。适配器可以抛异常，也可以用 `finish{kind:'error'}` 结束，运行时对外统一成终止 chunk（`docs/subsystems/llm-streaming.md:234-252,300`）。
- DeepSeek 适配器的稳定错误码：`AUTH`、`QUOTA`、`RATE_LIMIT`、`CONTEXT_WINDOW_EXCEEDED`、`INVALID_REQUEST`、`SERVER`、`HTTP_<status>`、`TRANSPORT`、`ABORTED`、`TIMEOUT`、`STREAM_CLOSED`、`MALFORMED_RESPONSE`、`EMPTY_RESPONSE`（`packages/llm/llm-deepseek/README.md:118`，仅文档；`TIMEOUT/ABORTED/TRANSPORT` 的映射见 `packages/llm/llm-deepseek/src/adapter.ts:63-66`）。
- 空回复算错误：`stop` 结束但没有内容块时抛 `EMPTY_RESPONSE`（`packages/llm/llm-deepseek/src/translate.ts:149`）。
- 流空闲看门狗默认 300 秒，只在等待下一个 chunk 时计时（`packages/llm/llm-deepseek/src/defaults.ts:4`，`adapter.ts:54-63`）。
- 一次适配器调用就是一次提供方尝试，适配器内部不重试（`docs/subsystems/llm-streaming.md:301`）。
- 重试由独立插件 `dsh-llm-retry` 在 `agent/request-error` 上执行。默认策略：最多 5 次，只重试 `EMPTY_RESPONSE/RATE_LIMIT/SERVER/TIMEOUT/TRANSPORT`，指数退避 500 毫秒到 10 秒，抖动 10%（`packages/llm/llm/src/retry-policy.ts:14-24`）。
- 每次重试先写 `llm/retry` 事件再等待，等待可取消。重试计数由日志投影得出，在 `step/start` 和 `turn/end` 清零。提供方的 `Retry-After` 在上限内优先（`packages/llm/llm-retry/src/index.ts:59-64,125-138,188-191,215-240`）。
- 重试在同一个打开的 step 内重跑，不重复 `agent/pre-step`，也不重复追加用户消息（`packages/core/agent-loop/src/agent.ts:406-424`）。

### 4.5 DeepSeek 专属的线扩展
| 位置 | 名称 | 内容 | 来源 |
|---|---|---|---|
| 请求头 | `x-deepseek-harness-user-id` | 每个 Harness home 一个匿名 UUID | `packages/llm/llm-deepseek/src/adapter.ts:128` |
| 请求头 | `x-deepseek-harness-session-id`、`x-deepseek-harness-compact` | 会话 id；压缩请求标记为 `1` | `packages/llm/llm-deepseek/src/adapter.ts:129-130` |
| 请求体 | `dsh_plugin_packages` | 当前激活的插件包名和版本 | `docs/deepseek-llm-api-wire-extensions.md:41-72` |
| 请求体 | `dsh_session_log` | 会话日志的增量后缀，单次上限 8 MiB，默认开启 | `packages/session/session-log-deepseek/src/index.ts:52-53` |
| beta 头 | `files-api-2025-04-14`、`mid-conversation-tool-changes-2026-07-01` | 图片走 Files 引用；对话中途增删工具 | `packages/llm/llm-deepseek/src/messages-api.ts:4,7`，`adapter.ts:114-127` |

- `dsh_session_log` 会把工作目录、系统提示、用户与助手内容、工具参数和结果发给配置的端点，包括 `baseURL` 指向的网关（`docs/deepseek-llm-api-wire-extensions.md:160`）。最小 profile `sdk-minimal` 也挂了这个插件（`packages/bundle/sdk-minimal/cordis.patch.yml` 的 `session-log-deepseek` 条目）。
- 另有两项按模型声明的能力：`systemPromptUpdate: in-history`（在历史中途追加新系统提示）和 `toolUpdate`（中途增删工具），目的都是保住前缀缓存（`packages/llm/llm-deepseek/README.md:52,84`，仅文档）。

## 5. 工具

### 5.1 定义
`ToolDefinition` 等于面向模型的 `ToolSchema`（`name`、`description`、`parameters`，可选 `deferLoading`）加上宿主侧字段：`output{schema, render, presentationMeta?}`、`execute(args, exec)`、`timeoutMs?`、`isConcurrencySafe?(args)`、`projectContent?`、`finalizeContent?`、`presentCall?`、`presentResult?`。发给模型的 schema 用显式白名单构造，宿主字段不会泄漏（`docs/subsystems/tools.md:9-103`）。

- 作者用 `defineTool` 写类型化参数和输出。参数不合格抛 `ToolArgsError`（`INVALID_ARGS`），输出不合 schema 抛 `ToolOutputError`，都走普通的错误结果（`docs/subsystems/tools.md:159`）。
- `execute` 返回规范 JSON 值，`output.render` 把它投影成模型可见的内容块。日志只存渲染后的内容、错误和 `meta`，不存规范值（`docs/subsystems/tools.md:392`）。
- 工具体用 `exec.signal` 协作取消，用 `exec.deferContext()` 把附加上下文挂到结果之后，用 `exec.concludeTurn()` 让本轮在这一步结束（`docs/subsystems/tools.md:235-252`）。
- `ctx.tools.restrict({allow, deny})` 按 agent scope 过滤全局工具。被过滤的工具既不出现在提示里，也拒绝执行（`docs/subsystems/tools.md:163-178`）。

### 5.2 执行管线
| 阶段 | 形态 | 能做什么 | 来源 |
|---|---|---|---|
| 记录 `tool/call` | 循环在派发前追加 | 审计与 UI 先看到调用 | `packages/core/agent-loop/src/tool-calls.ts:168,263-266` |
| `tools/pre-execute` | waterfall | 返回 `allow/deny/cancel/ask`。`ask` 交给 `ctx.approval`，只有 `allowed-once` 放行 | `packages/core/tools/src/index.ts:153,1506` |
| guards | 注册的函数 | 只能返回拒绝理由，没有"允许"这个返回值，所以监听器顺序无法把拒绝翻回来 | `docs/subsystems/tools.md:325-336`，`packages/core/tools/src/index.ts:769` |
| `tools/execute` | 环绕式 waterfall | 超时、重试、度量；可替换 signal | `packages/core/tools/src/index.ts:164,1606` |
| `projectContent` | 工具自带回调 | 装入执行期准备好的内容 | `packages/core/tools/src/index.ts:246` |
| `tools/post-execute` | waterfall | `accept`（可替换内容或值，可附加上下文）或 `block`（变成错误结果） | `packages/core/tools/src/index.ts:176,1783` |
| `finalizeContent` | 工具自带回调 | 最后一道只改内容的不变式，对每种结局恰好调用一次 | `packages/core/tools/src/index.ts:258,1688-1690` |
| `tools/result` | 同步通知 | 观察冻结后的最终结果 | `packages/core/tools/src/index.ts:198,1704` |
| 记录 `tool/result` | 循环按模型顺序追加 | 引用对应 `tool/call` 的 `seq` | `packages/core/agent-loop/src/tool-calls.ts:269-290` |

参数一旦记录就不能被改写，因为历史、审计、UI 和执行必须一致。未知工具和抛异常的工具都变成结构化的错误结果，调用失败但 turn 不结束（`docs/subsystems/tools.md:428-430`）。

### 5.3 并行调用
- 工具用 `isConcurrencySafe(args)` 自报是否可并行。只有返回 `true` 才算，默认独占（`packages/core/tools/src/index.ts:1303-1307`）。
- 调度：独占调用是顺序屏障。连续的可并行调用进一个滚动池，上限 `maxParallelToolCalls`，默认 10（`packages/core/agent-loop/src/tool-calls.ts:85-102,199-214`，`packages/core/agent-loop/src/constants.ts:6`）。
- pre 策略按模型顺序串行，只有工具体重叠执行。结果和附加上下文按模型顺序提交（`packages/core/agent-loop/src/tool-calls.ts:146-161,216-231`）。
- 分类是一元的。需要比较兄弟调用或资源才能判断安全性的，必须保持独占（`packages/core/agent-loop/README.md:213`）。

### 5.4 输出上限与 spill
| 层 | 机制 | 默认值 | 来源 |
|---|---|---|---|
| 子进程流 | 每个流有内存上限，超出保留尾部，完整流写私有 spill 文件 | bash `maxOutputBytes` 64000 | `packages/shell/bash-local/src/index.ts:104-105`，`docs/subsystems/subprocess.md:28-36` |
| 工具自身 | `read` 最多 2000 行、每行 2000 字符、50 KiB；`glob` 100 条；`grep` 250 条 | 见左 | `packages/fs/tool-fs/src/read.ts:15`，`packages/fs/tool-fs/src/read-render.ts:11,14`，`packages/fs/tool-fs-search/src/glob.ts:25`，`packages/fs/tool-fs-search/src/grep.ts:29` |
| 结果级 spill 策略 | `tools/post-execute` 监听器。超预算的结果换成头尾预览，加完整结果的定位符和读取提示。保存失败就保留原文 | `maxInlineTokens` 12500 | `packages/bundle/base/cordis.patch.yml:407-410`，`packages/spill/spill-policy/README.md:12,49-57` |
| spill 存储 | `ctx.spillStore.saveText()` 一个方法。本地后端写到 `session-<sha256 前 12 位>` 目录，目录权限 0700，文件用 `'wx'` 加 0600 打开 | | `packages/spill/spill-local/src/store.ts:79,113,115` |
| 压力期裁剪 | 只在压缩触发后，把超大的旧工具结果换成头 4096 加尾 1024 字符 | 阈值 8192 字符 | `packages/bundle/base/cordis.patch.yml:418-423` |

### 5.5 内置工具
"默认"一列：`base` 指 `dsh-base` 中启用的条目（`packages/bundle/base/cordis.patch.yml`）。`web` 指 Web 的 `standard` 预设额外挂载的（`packages/bundle/web-app/presets/standard.patch.yml`）。其余为可选包。名称和用途取自 `docs/tool-catalog.md:16-48` 及各工具小节。

| 工具 | 一句话 | 默认 |
|---|---|---|
| `bash` / `pwsh` | 每次在全新 shell 进程里执行一条命令。Windows 启用 `pwsh`，其他平台启用 `bash` | base |
| `job_output`、`job_list`、`job_kill` | 读取、列出、终止后台任务（后台命令、PTY、子代理） | base |
| `read`、`write`、`edit`、`read_image` | 带行号读文本、整文件创建或替换、字面量替换编辑、读图片 | base |
| `glob`、`grep` | 文件名匹配与内容搜索，用打包的 ripgrep | base |
| `skill` | 按名字加载一个 skill 的完整说明 | base |
| `exit_plan_mode` | 提交计划供用户审阅，通过后退出 plan 模式 | base |
| `subagent`、`subagent_fork` | 委派给全新上下文的子代理，或带父会话已完成历史的子代理 | base |
| `send_message`、`interrupt_agent`、`list_agents` | 给可继续的子代理发消息、打断、列出 | base |
| `workflow` | 运行一段 JS 编排脚本，批量扇出子代理 | base |
| `todo_write` | 整体替换本会话的任务清单 | base |
| `create_goal`、`get_goal`、`update_goal` | 管理同会话的持久目标 | base |
| `web_search`、`web_fetch` | 联网搜索与抓取 | base |
| `list_mcp_resources`、`list_mcp_resource_templates`、`read_mcp_resource` | MCP 资源，只在配置了服务器时出现 | base |
| `ask_user_question` | 向用户提问并等待回答 | web |
| `schedule_create`、`schedule_list`、`schedule_update`、`schedule_delete` | 在本会话创建和管理定时提醒 | web |
| `present` | 把已有文件声明为交付物 | web |
| `ralph` | 用一串全新子代理迭代同一个目标 | base 中禁用 |
| `plugin_manager` | 管理 profile 的插件和 bundle | base 中禁用 |
| `run_code` | PTC 模式下用一段 TypeScript 程序调用其他工具 | 仅 `mode: ptc/both` |
| `str_replace_editor`、持久版 `bash`/`pwsh`、`terminal_*` 六个、`lsp`、`session_*` 五个、`cordis_inspect_*` 两个、`load_workspace_dependencies`、`stagehand_*` 六个、Agent Teams 九个 | 见 `docs/tool-catalog.md` | 可选 |

## 6. 安全

仓库对自己的定位：实验性开发者预览，未做安全审计。沙箱和审批只能降低风险，不保证隔离（`SAFETY.md`）。

### 6.1 两个独立旋钮与权限预设
| 旋钮 | 取值 | 存储 | 来源 |
|---|---|---|---|
| 沙箱模式 `sandbox/mode` | `read-only`、`workspace-write`、`danger-full-access`。只管文件效果，不管网络和进程可见性 | 会话日志事件，回放即恢复 | `docs/subsystems/sandbox.md:11-20`，`packages/sandbox/sandbox-policy/README.md` 的 per-session store 一节 |
| 审批策略 `approval/policy` | `ask`（交给应答者）或 `never`（一律 `rejected`，不询问任何人） | 会话日志事件 | `docs/subsystems/approval.md:33-46` |

- 权限预设把两个旋钮打包成一个选择器。`dsh-base` 配了三个：`read-only`（read-only 加 ask）、`workspace-write`（workspace-write 加 ask）、`danger-full-access`（danger-full-access 加 never）（`packages/bundle/base/cordis.patch.yml:250-262`）。新会话默认 `workspace-write`，可用环境变量 `DSH_PERMISSION_MODE` 改（同文件 `:229-233,245-248`）。
- 当前策略作为运行时上下文快照发给模型（`docs/subsystems/approval.md:49`）。plan 模式只是软性引导，不参与任何强制（`docs/subsystems/plan.md:5`）。

### 6.2 审批模型
- 结果是封闭集合：`allowed-once`、`rejected`、`cancelled`、`unavailable`。只有 `allowed-once` 放行，并且只对这一次调用有效（`packages/interaction/user-approval/src/index.ts:55`，`docs/subsystems/approval.md:21`）。
- 失败即关闭：没有应答者、应答者抛错或返回非法值，一律归为 `unavailable`（`packages/interaction/user-approval/src/index.ts:283-288`）。`never` 策略在派发前就判定，后注册的监听器绕不过去（同文件 `:270-275`）。
- 每次询问写一对审计事件 `approval/asked`、`approval/decided`，不进模型上下文。请求里不带工具参数，UI 用 `callId` 把提示挂到已经展示的调用上（`docs/subsystems/approval.md:53,86`）。
- 授权粒度只有一次性的 `allowed-once`。结果集合里没有"本会话始终允许某工具"这类持久授权。

### 6.3 沙箱后端
| 平台 | 运行器 | 做法 | 自报强度 | 来源 |
|---|---|---|---|---|
| Linux | `bwrap`，备选 Landlock 启动器 | 只读绑定根目录、新 `/dev`、私有 PID 命名空间；`workspace-write` 再加 tmpfs `/tmp` 和可写的工作区绑定 | `full`（旧 Landlock ABI 为 `partial`） | `packages/sandbox/sandbox-local/src/index.ts:160-162,178-181`，`packages/sandbox/sandbox-local/src/profiles.ts:16-36` |
| macOS | Seatbelt（`sandbox-exec`） | 默认允许，拒绝 `file-write*`，再按可写根放行 | `full` | `packages/sandbox/sandbox-local/src/index.ts:162`，`profiles.ts:51-58` |
| Windows | `windows-acl` 受限令牌运行器 | `WRITE_RESTRICTED` 令牌，加能力 SID 的允许 ACE、对父目录 `FILE_DELETE_CHILD` 的拒绝项、Low 完整性标签；子进程放进 `KILL_ON_JOB_CLOSE` 的 Job | `partial` | `packages/sandbox/sandbox-local/src/index.ts:163-166,182-188`，`packages/sandbox/sandbox-windows-acl/README.md:12,98,110` |

Windows 的现状要点（`packages/sandbox/sandbox-windows-acl/README.md:122-133` 的 Verified boundaries 一节，仅文档，未运行验证）：
- 只约束写和删除。读、网络、进程可见性都不受限，所以 `read-only` 模式下子进程仍能读调用者可读的任何文件。
- 硬链接是文件对象的别名，授权会透到工作区外的别名路径。
- 工作区目录上的 ACE、拒绝项和 Low 标签是常驻修改，不随会话撤销。
- `dsh-base` 在 win32 上启用 `tool-pwsh` 和 `pwsh-sandbox`，禁用 `tool-bash` 和 `bash-sandbox`（`packages/bundle/base/cordis.patch.yml:235-243,267-273`）。

通用规则：
- 没有可用运行器时 `confine()` 抛 `SANDBOX_UNAVAILABLE`，不会静默放行（`docs/subsystems/sandbox.md:156`）。
- 每次包装返回强度、拒绝特征串和运行器失败规则，调用方据此区分"命令被拦"和"沙箱自己坏了"（`packages/sandbox/sandbox-local/src/index.ts:207-242`）。
- 文件工具走进程内的 `dsh-fs-sandbox`，shell 走操作系统沙箱。两者读同一个 `ctx.sandboxPolicy`，避免约束到不同的根（`docs/capability-seams.md:644`，`packages/fs/fs-sandbox/README.md:12`）。

### 6.4 guard、命令策略与其他
- 没有命令黑名单、白名单或危险命令分类器。在 `packages/shell`、`packages/sandbox`、`packages/interaction`、`packages/guard` 的源码中搜索 `allowlist`、`denylist`、`command policy`，没有相关命中。`packages/guard` 只有两个包：重复调用提醒和工具超时（见第 11 节）。
- 命令需要更宽权限时，模型在同一次调用里带 `sandbox_permissions` 和 `justification`，经 `ctx.approval` 批准后只对这一次调用放宽（`packages/shell/tool-bash/src/index.ts:217,243-248,396-404`，`packages/sandbox/sandbox/src/escalation.ts:41`）。
- 自定义策略挂在 `tools/pre-execute`。现成的有 Claude Code 和 Codex 的 hook 桥，以及实验性的 Auto review：用当前模型评估每次调用，默认关闭（`packages/experimental/auto-review/README.md:12`）。
- 读后写：`dsh-fs-observation-policy` 要求先读再写或编辑，文件在读后变化就拒绝并提示重读。观察记录不持久化，恢复会话后要重读（`packages/fs/fs-observation-policy/README.md:12`）。
- 子进程环境清洗：变量名匹配 `/KEY|PASSWORD|SECRET|TOKEN/i` 的不传给子进程（`packages/subprocess/subprocess/src/index.ts:47`）。

## 7. 上下文管理

### 7.1 系统提示的组装
- `ctx.systemPrompt` 是注册表加一条 `system-prompt/assemble` waterfall。贡献分三类：section（带 `order` 的文本）、variable（`{{name}}`）、工具 schema 提供者。section 按 `order` 升序、再按名字排序后拼接（`docs/subsystems/system-prompt.md:44-72`）。
- 固定位置：身份开场白 order -1000，`personaPrefix` order 0，`personaSuffix` order 10200，`plan:policy` order 50（`packages/core/system-prompt/README.md` 的配置表，`docs/subsystems/plan.md:29`）。
- 渲染是严格的。未知变量或无值变量直接抛错，不发残缺的提示（同 README 的 Assembly and rendering 一节）。
- 渲染结果作为 `system/message` 事件进日志，是 surface 节点 0。文本没变就不写新事件。文本变了，默认原位 replace；路由声明 `in-history` 时改为在历史末尾追加（`packages/core/agent-loop/src/runtime-context.ts:88-103`）。

### 7.2 动态上下文不进系统提示
- 沙箱模式、审批策略这类会变的事实由 `PromptContext` 贡献，合成一条 user 角色的运行时上下文快照。只有内容变化或被压缩吃掉时才追加新快照（`packages/core/agent-loop/src/runtime-context.ts:152-163`，`docs/subsystems/system-prompt.md:76`）。
- 目的在于保住前缀缓存。每个包的 README 都有 Model Experience 一节，逐项写明模型看到什么、token 影响、KV Cache 影响（`packages/README.md:107`，例如 `packages/core/agent-loop/README.md:160-162`）。

### 7.3 项目记忆文件
`dsh-agent-instructions`（`packages/context/agent-instructions/README.md`，仅文档）：
- 候选文件名默认是 `AGENTS.md`、`CLAUDE.md`，本地覆盖层是 `AGENTS.local.md`、`CLAUDE.local.md`。项目根由 `.git` 标记。
- 首个请求注入一条持久的 user 消息：先是用户级 `$DSH_HOME/AGENTS.md`，再是从项目根到会话 cwd 的链，由宽到窄。内容相同的兄弟文件只渲染一次。
- 不持续监听文件。`read`、`write`、`edit` 成功触达更深目录后，下一次请求补上新适用的文件。文件变了发替换，没了发移除通知。
- 字节预算 `maxBytes`（base 为 65536，`packages/bundle/base/cordis.patch.yml:289-292`）。超预算时先丢更宽的文件，最后才截断最具体的文件，并给出可见提示。
- 文件内容里的 `</system-reminder>` 字面量会被转义，防止仓库内容闭合插件自己的包裹标签。

### 7.4 压缩
来源：`packages/compaction/compaction-basic/README.md:62-76,104-125`，`docs/subsystems/compaction.md:11-21,101`。默认值核对于 `packages/compaction/compaction-basic/src/config.ts:20,23,75`。区间选择的实现 `region.ts` 未逐行核对。

| 项 | 内容 |
|---|---|
| 主动触发 | 在 `agent/pre-step` 上，token 压力超过 `floor(min(W×0.8, W-O-B))`。`W` 是上下文窗口，`O` 是输出上限，`B` 是 65536 的余量 |
| 被动触发 | `agent/request-error` 收到 `CONTEXT_WINDOW_EXCEEDED`，做一次尽可能大的头部压缩。只有 surface 确实前进才授权重试 |
| 手动触发 | `/compact` 命令，以空闲期维护任务运行 |
| 第一步 | 可选的工具结果裁剪，不调模型。裁完够用就跳过摘要 |
| 选区 | 从第一个非 `system/message` 节点开始的最老平衡区间。边界保持工具调用与结果成对，但不保证整个 turn |
| 保留 | 最新的约 `(W-O)×16%` 原样保留 |
| 摘要调用 | 一次额外的模型请求。原样回放系统提示、工具声明和被压缩区的消息，最后追加压缩指令，尽量复用提供方的热前缀。只取返回的文本 |
| 落盘顺序 | `compaction/start`（锁），`compaction/summary`（摘要、被遮蔽的 seq、token 估算、调用信封），一条 `surfaceOp: replace` 的 `user/message`（用 `<compacted-summary>` 包裹），最后 `compaction/end` |
| 崩溃语义 | 最后才释放锁。中途崩溃留下可检测的孤儿 `compaction/start`。早于最新 `session/end-seed` 的孤儿视为陈旧，忽略 |

压缩不能缩小系统提示和工具声明，也拆不开单个巨大的工具调用（`packages/compaction/compaction-basic/README.md:12`）。

### 7.5 预算核算与附件
- `ctx.tokenMeter` 回放日志得出估算，不调模型。文本用固定启发式：4 个字符 1 个 token，加块和角色开销。只有当最近一次成功调用的请求信封与当前信封一致时，才用提供方报告的 usage 作锚点，再加上之后 surface 变化的带符号增量。README 自己指出 CJK 文本和 JSON schema 在这个启发式下会被严重低估（`packages/llm/token-meter/README.md:12,43,68,89`，仅文档）。
- 图片和文件的字节不进会话日志。`ctx.attachments` 先把对象持久化到 `<DSH_HOME>/attachments/v1`，事件里只放内容寻址的引用和元数据（`docs/subsystems/attachment.md:5-7`）。
- 请求超出图片预算时，适配器报 `IMAGE_OFFLOAD_REQUIRED`。`image/offload` 事件记录把哪些旧图换成占位文本，然后重试（`docs/subsystems/compaction.md:25-28`，`docs/subsystems/llm-streaming.md:246-250`）。

## 8. 规划与长任务

| 机制 | 是什么 | 状态存哪 | 谁驱动 | 来源 |
|---|---|---|---|---|
| todo | 模型维护的清单。每次整体替换，条目只有 `content` 和三态 `status` | `todo/write` 事件 | 模型自己 | `docs/subsystems/todo.md:22-32` |
| plan 模式 | 一段只在激活时出现的系统提示，软约束。`exit_plan_mode` 始终留在工具表里，避免切换模式时改动工具声明 | `plan/mode` 事件 | 用户用 `/plan` 或 UI 进入，退出要用户审阅计划 | `docs/subsystems/plan.md:5,11,33` |
| goal | 同一会话内的一个持久目标，带修订号和阶段 `active/paused/blocked/complete`，有轮数上限 | `goal/change` 事件（整快照） | `goal-round-driver` 在空闲时排下一轮 | `docs/subsystems/goal.md:25-30,93` |
| workflow | 模型写的 JS 编排脚本，用 `agent()`、`parallel()`、`pipeline()`、`phase()`、`log()` 扇出子代理，返回一个 JSON 值 | 父会话里的 `tool-workflow/*` 展示记录 | 前台工具调用。引擎在受会话文件策略约束的 Node 进程里跑脚本 | `docs/subsystems/workflow.md:5-7,65,124`，`packages/workflow/workflow/README.md:12` |
| ralph | 固定脚本的 workflow：每轮一个全新子代理，只带上一轮的有界报告 | 同上 | 仅在人明确要求时使用。base 中禁用，`maxRounds` 64 | `packages/workflow/tool-ralph/README.md:12`，`packages/bundle/base/cordis.patch.yml:447-452` |
| jobs | 后台工作的通用注册表：后台命令、PTY 发送、子代理。统一的 id、归属、输出环和终止 | 进程内，不持久 | 生产者注册，模型用 `job_*` 读取。完成时通知或唤醒属主 | `docs/subsystems/jobs.md:9,22,26`，`packages/jobs/tool-jobs/README.md:12` |
| schedule | 墙钟提醒：一次性、固定间隔、每日、每周、cron。到期后作为 follow-up 送回原会话，必要时冷恢复该会话 | 宿主的存储域，与会话是否激活无关 | 宿主定时器 | `packages/schedule/schedule/README.md:12` |

区别的主线：todo 和 plan 只影响模型看到的内容。goal 让同一个会话自动多跑几轮。workflow 和 ralph 把工作分给别的会话。jobs 管的是进程里还在跑的东西。schedule 管的是时间。本节的行为描述均为仅文档，只有引用到的配置文件行核对过。

## 9. 子代理与团队

- 接缝是 `ctx.subagents`，按名字并存多个提供者：进程内的 `spawn`（空白上下文）和 `fork`（带父会话已完成 turn 的前缀），以及进程外的 ACP、Codex、Claude Code、dsh SDK 子进程（`docs/subsystems/subagent.md:5-7`）。
- 两种委派形态（`docs/subsystems/subagent.md:124-146,346-348`）。one-shot：前台等待，返回一个 `SubagentRun`，消费者 await `result` 后必须 `dispose()`。continuable：后台启动，立即返回 `childId`；子会话是持久的，可被 `send_message` 再次唤醒，进程重启后可冷恢复。
- 上下文隔离：每个子代理是一个独立 Session，头里记 `parentSession` 和 `delegationDepth`。子代理拿到全新的扁平 scope，不继承父代理的注册。fork 的种子是父日志截至最后一个 `turn/end` 的平衡前缀，进行中的 turn 不带（`docs/subsystems/subagent.md:457-462`）。
- 结果回传：one-shot 返回 `SubagentResult{output, structured?, diagnostic?, stopReason}`。`output` 是子代理最后一条非空 assistant 消息。非 `completed` 的结果映射成工具错误，不把部分输出当成功（`docs/subsystems/subagent.md:286-320`）。continuable 子代理结束一段驻留时，管理器给父代理投递一条 `subagent-settled` 通知，带结束方式和最后输出的文本（`docs/subsystems/subagent.md:202-221`）。
- 结构化输出：调用方给一个对象根的 JSON Schema，进程内后端用一个强制的捕获工具实现（`docs/subsystems/subagent.md:40`）。
- 权限：`send_message` 只允许直接父子之间互发，权威来自确切的活体发送者对象（`docs/subsystems/subagent.md:150`）。父代理是 read-only 或 workspace-write 时，子代理沿用继承的沙箱模式并固定 `approval: never`（`docs/subsystems/subagent.md:459`）。活着的子代理不能调用 `ask_user_question`（`packages/interaction/tool-ask-user/README.md:12`）。
- 上限：委派深度默认 1，可继续子代理的同时存活数默认 8（`packages/subagent/subagent/src/index.ts:200-201`）。workflow 引擎另有并发上限（0 表示自动取 `min(16, max(1, 核数-2))`）和单次运行总数上限 1000（`packages/workflow/workflow-ptc/src/index.ts:35-38,106-110`）。
- 模型选择：`subagent` 工具可按配置暴露可选的 provider、model 和推理强度。`subagent_fork` 固定沿用父路由，让继承的历史还能命中缓存（`packages/bundle/base/cordis.patch.yml:377-388`）。

Agent Teams 是实验性功能，默认关闭。它在可继续子代理之上加三样持久状态：名册、点对点邮箱（先排队后确认送达）、共享任务 DAG（整快照、用修订号做 CAS、`blockedBy` 无环、`writeScopes` 只是建议性的路径前缀）。全部由根会话的 `team/*` 事件回放得出（`docs/subsystems/agent-team.md:9-74,123-125`，`docs/tool-catalog.md:42`，仅文档）。

## 10. 扩展机制

### 10.1 Skills
- 文件格式：目录包 `<name>/SKILL.md`，或顶层的 `<name>.md`。不递归发现嵌套的 `SKILL.md`。名字是 kebab-case（`docs/subsystems/skills.md:85`）。
- frontmatter：必填 `name`、`description`。可选 `whenToUse`、`metadata`、`disable-model-invocation`、`user-invocable`（`packages/skill/skill-filesystem/src/index.ts:814-815,834,1004-1005`）。
- 发现根与优先级（数字小者优先）：`<项目根>/.dsh/skills` 100，`<项目根>/.agents/skills` 200，自定义目录 300，`<dshHome>/skills` 400，`<agentsHome>/skills` 500，打包目录 600（`docs/subsystems/skills.md:66-77`）。
- 渐进披露分三层（`docs/subsystems/skills.md:229-233`）：
  1. 会话目录：首个 `agent/pre-step` 注入一条 user 角色的 `<system-reminder>`，只含排序后的 `name` 和最多 500 字符的 `description`，不含正文和路径。
  2. 正文：模型调用 `skill({name})`，工具当场重读文件，返回 `<skill_content>`、`<skill_resources>`、`<skill_instructions>`。
  3. 资源：脚本和参考文件只在正文明确引用时由模型自己去读，工具不枚举目录。
- 目录变化时按渲染内容的摘要比较，变了才追加一条完整的替换消息。只改正文不产生目录消息。
- 注册表是 `ctx.skills`，可以有非文件系统的提供者（`docs/subsystems/skills.md:11`）。

### 10.2 Hooks
dsh 原生的 hook 就是 Cordis 事件（第 2、5 节里的 waterfall）。`packages/hooks` 提供的是兼容桥：读取现有的 Claude Code 或 Codex `hooks.json`，只跑 command 类型的 hook（`packages/hooks/hook-protocol/README.md:12`）。

| hook 事件 | 映射到的扩展点 | 能做什么 |
|---|---|---|
| `SessionStart` | `agent/created` | 附加上下文 |
| `UserPromptSubmit` | `agent/pre-step` | 拦截提示或附加上下文 |
| `PreToolUse` | `tools/pre-execute` | 拦截。Claude Code 桥还可以请求审批 |
| `PostToolUse` | `tools/post-execute` | 用反馈拦截结果，或附加上下文 |
| `Stop` | `agent/turn-stopping` | 给一个理由，强制再跑一步 |
| `SubagentStart`、`SubagentStop` | `subagent/start`、`subagent/end` | 前者给进程内子代理附加上下文，后者只观察。只有 Claude Code 桥支持 |

来源：`packages/hooks/hooks-claude-code/README.md:54,85` 起的两节，`packages/hooks/hooks-codex/README.md` 的同名表。协议要点：退出码 2 表示拦截，其他非零只记日志；多个 hook 的结果按 `deny > ask > allow` 合并；每次运行写一对 `hook/invoked`、`hook/result` 事件；任何失败都降级处理，不向循环抛异常（`packages/hooks/hook-protocol/README.md:36-41,72`）。

### 10.3 MCP 客户端
- 每个服务器一个 `dsh-mcp-client` 条目。传输是 `stdio` 或 `streamable-http`，协议协商交给官方 SDK（`packages/mcp/mcp-client/README.md` 的配置表）。
- 工具名是 `mcp__<serverName>__<rawName>`。命名空间取本地配置的 `serverName`，不取远端自报的名字。有损规范化时追加 12 位 SHA-256 十六进制，防止不同身份撞名（`packages/mcp/mcp-client/src/index.ts:4`，同 README 的 Design philosophy 一节）。
- 工具表按"代"原子切换：刷新失败保留上一代，注册冲突整代回滚。单次调用超时默认 60 秒。断线后指数退避重连，500 毫秒到 30 秒，连续 10 次失败后摘掉工具（同 README，仅文档）。
- MCP 工具走和原生工具相同的管线。服务器的 instructions 作为带归属的系统提示段，有字节上限。不支持 prompts、elicitation、tasks 和资源订阅（`docs/subsystems/mcp.md:41-45,67,101`）。

### 10.4 插件与其他扩展点
- 新行为一律挂到已有扩展点。`docs/architecture.md:143-164` 有一张"想做什么，用哪个机制"的表，例如加模型提供方就在 `ctx.llm` 注册适配器，加工具就在 `ctx.tools` 注册。
- 树外插件装进 profile 目录，由 `dsh plugin` 转发给 pnpm 管理（`apps/cli/README.md` 的 Entry modes 表）。
- 人用的斜杠命令走 `ctx.commands`，直接执行，不产生模型消息（`docs/subsystems/commands.md:5`）。
- agent 预设：每个会话可以从一份 YAML 预设组合出自己的工具和提示，预设 id 记在会话头里（`docs/subsystems/persistence.md:177-183`）。另有 `packages/extensions` 让 agent 在运行时查询 Cordis 树并挂载模型写的插件（`docs/subsystems/extensions.md:5`，未深入核对）。

## 11. 防御性设计

`docs/defensive-patterns.md` 的七条规则，文档说每条都对应真实出过或险些出过的缺陷：

| 规则 | 含义 | 行 |
|---|---|---|
| 独立上报正交结果 | 进程可以既超时又退出码 0。`timedOut`、`signal`、`exitCode` 各自成字段 | 7-9 |
| 公共契约两边都要守 | 适配器可以抛异常也可以发错误 finish，运行时对外只暴露一种形态 | 11-13 |
| 异步状态不是同步状态 | `followup()` 没有逐消息的完成信号，不要把 `whenIdle()` 当成某条消息的结果 | 15-17 |
| dispose 要达到静止 | 清理必须 await 子任务退出，并且先关通知注册表再杀进程 | 19-21 |
| 在派发器里兜住回调异常 | 一个坏监听器不能拖垮核心生命周期 | 23-25 |
| 不给不可信输出环境变量和可预测路径 | 子进程环境清洗；临时文件用私有目录、随机名、独占打开 | 27-29 |
| 链接形态的路径用 unlink | 先 `lstat` 再 `unlink`，不递归删除。Windows junction 尤其如此 | 31-33 |

循环与工具层面的防护：

| 机制 | 行为 | 来源 |
|---|---|---|
| 重复调用提醒 | 同一工具加规范化后完全相同的参数连续出现 3、5、8 次时，在结果之后追加提醒。只提醒不拦截。被拒绝的调用也计数。新的用户消息清零。状态只在内存 | `packages/guard/repeat-tool-reminder/README.md:12,81-91`，`packages/guard/repeat-tool-reminder/src/index.ts:53-56` |
| 工具超时 | 工具自己声明 `timeoutMs`。到期后中止派生的 signal，等工具收敛后返回 `TOOL_TIMEOUT` 错误。不能硬杀不配合的工具 | `packages/guard/timeout-policy/README.md:12,69` |
| shell 超时转后台 | 前台命令超时后默认转成后台 job，而不是杀掉。执行器默认超时 120 秒，上限 600 秒；`dsh-base` 把 bash 的默认超时配成 60 秒 | `packages/shell/tool-bash/src/index.ts:212-215`，`packages/shell/bash-local/src/index.ts:102-103`，`packages/bundle/base/cordis.patch.yml:235-239` |
| 失败 step 的工具配平 | step 失败时给每个未答复的工具调用补错误结果，保证后续请求里调用与结果成对 | `packages/core/agent-loop/src/agent.ts:342-353` |
| 追加即校验 | 坏事件在 `append` 处失败，不拖到后端 flush | `packages/core/session/src/index.ts:728-748` |
| goal 与 workflow 上限 | goal 默认 256 轮；模型自报 blocked 需连续 3 轮；workflow 单次最多 1000 个子代理 | `packages/goal/goal/README.md:12`，`packages/goal/tool-goal/README.md:12`，`packages/workflow/workflow-ptc/src/index.ts:108` |
| 其余 | 模型请求重试与流空闲超时见 4.4；崩溃与 fork 修复见 3.5；持久化检查点见 3.3；压缩锁见 7.4；MCP 重连见 10.3；审批与沙箱的失败即关闭见第 6 节 | |

没有的东西：循环内置的最大步数、强制性的死循环终止、工具参数 JSON 修复（`packages/core/agent-loop/README.md:217`，4.3 节）。

## 12. 内核与前端的分离

所有前端都通过同一个启动器 `dsh` 选择 profile。profile 决定挂哪个应用 bundle，内核树是共用的（`docs/architecture.md:19-25,45`）。

| 前端 | 协议 | 要点 | 来源 |
|---|---|---|---|
| SDK（`dsh --profile sdk`） | 换行分隔的 JSON-RPC 2.0，走 stdio | 3 个请求：`initialize`、`session/prompt`、`shutdown`。4 个通知：`session.event`、`session.status`、`subagent.started`、`subagent.finished`。stdout 只放协议帧 | `packages/sdk/protocol/src/types.ts:108-118`，`packages/sdk/server/README.md:12` |
| Python SDK | 同上 | wheel 里打包 `dsh` 可执行文件，客户端以子进程启动它。Python 侧只镜像线上的形状，不嵌入内核 | `python/README.md:5-16`，`docs/architecture.md:49` |
| ACP（`dsh --profile acp`） | 标准 Agent Client Protocol v1，走 stdio | 只面向自动化，刻意不带 dsh 专有的展示数据。不支持删除、fork、对话回放 | `packages/acp/acp/README.md:12`，仅文档 |
| headless（`dsh --profile headless "任务"`） | 无协议 | 跑一个任务，最终答案写 stdout，退出码 0 或 1。`--json` 输出按行的事件流（`session`、`status`、`text`、`thinking`、`tool_call`、`tool_result`、`final`）。`--session-id` 续接已有会话 | `packages/bundle/headless/README.md:12`，仅文档 |
| Web（`dsh web`） | HTTP `/api` 桥，加多路复用 WebSocket `/api/remote.mux` | 宿主服务用 `@Remote` 装饰器标出可远程调用的方法，构建时生成客户端类型（Typert）。浏览器拿到一段连续的持久事件窗口加临时的 live chunk，自己组装对话节点。领域状态由宿主投影算好再推送 | `docs/api-gateway.md:5-13,58`，`docs/subsystems/conversation.md:11`，`docs/subsystems/session-projection.md:5` |
| Desktop | Electron 壳 | 用 Electron 的 Node 模式启动同一套 CLI profile 运行器和完整的 Web 应用 | `docs/architecture.md:51-55` |

- 给前端的两条事件通道分工明确：需要可回放数据的消费 `session/event`（持久事实）。`agent/*` 是进程内的实时协调接口，管队列、状态、拦截和流式帧（`docs/agent-lifecycle.md:89`）。
- 流式文本走进程内的 `agent/assistant-stream`，分 `start/chunk/end` 帧。`end` 帧携带已提交的持久事件的 `seq`，前端据此从临时态切到持久态（`docs/subsystems/core.md:155-191`）。
- SDK 的 `session/prompt` 只返回入队回执 `messageId`，不代表某条回复或某个 turn 的结束。调用方要自己定义"一次运行"的区间（`packages/sdk/protocol/README.md` 的 Payload semantics 一节，`docs/defensive-patterns.md:15-17`）。
- 仓库没有内置的终端 TUI。CLI 文档里的 `tui` profile 只是示例，前提是另行安装（`apps/cli/README.md:28`）。
- 最小内核样例：`sdk-minimal` profile 不叠 `dsh-base`，自己列出完整的树，只带一个持久 shell 工具，没有压缩、文件工具、skills 和子代理（`packages/bundle/sdk-minimal/README.md:12`，`packages/bundle/sdk-minimal/cordis.patch.yml`）。

## 13. 测试与基准

### 13.1 循环如何做到确定性测试
| 手段 | 做法 | 来源 |
|---|---|---|
| 脚本化 `MockAdapter` | 构造时给一个脚本数组，每次模型调用消费下一项。项可以是 chunk 数组、根据请求算 chunk 的函数、`'hang'`、`'hang-slow'`（50 毫秒后才响应取消）或 `{hangAfter}`。记录收到的每个请求供断言。脚本耗尽即抛错 | `packages/core/agent-loop/tests/mock-adapter.ts:72-139` |
| 只 mock 昂贵或不确定的边界 | 模型适配器、网络、时钟之外全部用真实现。工具注册表、管线、JSONL 持久化都是真的 | `docs/testing.md:29` |
| 基于性质的测试 | 用 fast-check 生成输入调度。不变量：每条发出的消息在日志里恰好出现一次，turn 编号严格递增，状态只在 idle 和 running 之间切换。靠 `agent/status` 信号同步，不用墙钟睡眠 | `packages/core/agent-loop/tests/properties.spec.ts:1-10` |
| 录制会话回放 | `dsh-llm-replay` 从录好的 Session JSONL 里取出每次模型调用的 chunk 流当脚本。同一份文件既是输入也是期望输出，不需要 API key | `packages/test-support/llm-replay/README.md:12`，`docs/testing.md:14` |
| 快照套件 | `snapshots/` 下 `session`、`sdk`、`acp`、`web` 四个目录。场景经真实的 `dsh` 子进程启动。改动文件系统的场景另外比对整棵 `workspace.expected/` 目录 | `docs/testing.md:14,55`，`packages/test-support/session-snapshot/README.md:12` |
| 模拟 HTTP 服务器 | `dsh-llm-mock-server` 提供 Messages 兼容的 SSE 端点，可编排连接重置、中途断流、挂起、畸形 JSON、429、5xx，用来测真实适配器的恢复 | `packages/test-support/llm-mock-server/README.md:12` |
| 真 API e2e | 有 key 时对真实模型跑，没 key 自动跳过 | `docs/testing.md:11,25` |
| 覆盖率闸门 | `packages/*/*/src` 逐文件 100% 行覆盖 | `docs/testing.md:10` |

两条纪律：断言要重新读文件或重跑命令来验证世界，不信 agent 的自述。产品可见的插件必须有一个经真实 loader 组合的测试，手工 `ctx.plugin()` 的单测不够（`docs/testing.md:35,39`）。

### 13.2 `benchmarks/` 实际测什么
是性能闸门，不是 agent 解题能力的评测。`benchmarks/AGENTS.md:3-9`：按被测的用户路径组织；输入全部由审过的常量合成，不用录制会话、用户材料或网络；计时代码在纯 Node 的独立子进程里跑；强制时间、堆内存和伸缩性预算。

| 目录 | 测量对象 | 核实程度 |
|---|---|---|
| `agent-continuation` | 长历史的请求处理、冷启动的重工具续跑。SDK 变体经 `sdk-minimal` 跑 100 个 turn、800 次真实读文件 | `benchmarks/agent-continuation/README.md:7` |
| `session-corpus` | 1000 到 3000 个会话的合成语料上的列表、内容搜索、fork | `benchmarks/session-corpus/README.md:7` |
| `session-open` | 打开一个合成的已发布 v0 格式会话 | 由文件名 `synthetic-released-v0-session.ts` 推断，未核实 |
| `conversation-fold`、`long-session-browser`、`active-stream-reconnect`、`terminal-io` | 客户端对话折叠、长会话浏览、流重连、终端 IO | 由目录名推断，未核实 |

根目录 `BENCHMARK.md` 只有一句话，指向用 Python SDK 的 `jsonrpc-agent` 最小变体跑外部基准。仓库内没有任务型评测集。

## 14. claude-code-mods.md

`docs/subsystems/claude-code-mods.md` 是一张兼容性对照表，对象是实验包 `experimental-claude-code-mods`。这个包把 Claude Code 的 mods（由 hooks 模块组成的一种插件形态）桥接到 dsh 上运行。文档逐项列出同一个 mod 经这座桥运行时，与在 Claude Code 2.1.287 下的行为差异（第 5 行）。文档没有说 dsh 派生自 Claude Code。

| 方面 | 文档的说法 | 行 |
|---|---|---|
| 打包 | mod 就是一个 dsh 插件，用 `defineMod({...})` 包住 hooks 模块，经 cordis.yml 挂载。不读 `plugin.json` 和 `hooks.json` | 11 |
| 权限 | mod 在进程内运行，拥有进程的全部权限，不加沙箱 | 16 |
| 事件 | `session.start/end`、`prompt.submit`、`turn.start/complete`、`tool.call`、`command.run` 有对应。`ui.render` 只支持 `AbovePrompt`。`turn.step`、`tool.check` 等一批事件可以登记但从不触发，加载时给警告 | 24-35 |
| 语义差异 | `tool.call` 在权限决定之后才运行。改写参数或改派到别的工具的 hook 会被跳过，因为已记录的调用必须按记录执行。`next` 只生效一次 | 28,41 |
| `$` 宿主 API | 提供 `ui`、`command`、`tool`、`prompt`、`session`、`state`、`store`、`clock`、`fs`、`process`、`http`、`env` 的一部分。`$.model`、`$.agent`、`$.mcp` 等直接拒绝 | 49-64 |
| 样例 | 三个取自博文的示例 mod 可运行。`anthropics/claude-code` 仓库里的 `diff`、`agents-md`、`sec-default`、`telemetry` 四个 mod 不可运行，并注明该仓库的条款不允许复制成测试夹具 | 90-98 |

dsh 与 Claude Code 生态的其他兼容点，都是显式的桥或沿用命名，没有看到共享代码：
- 桥：`dsh-hooks-claude-code` 读 Claude Code 的 `hooks.json`（10.2 节）；`dsh-subagent-claude-code` 把 Claude Code 作为进程外的子代理后端（`packages/subagent/subagent-claude-code/README.md:12`）。
- 命名：MCP 工具名 `mcp__<server>__<tool>` 采用与 Claude Code 和 Codex 相同的形状（`packages/mcp/mcp-client/README.md` 的 Tool naming 一节）；workflow 的 `meta` 字段词汇与 Claude Code 的 dynamic workflows 一致（`docs/subsystems/workflow.md:41`）。
- 文件约定：项目记忆同时认 `AGENTS.md` 和 `CLAUDE.md`；skill 用 `SKILL.md` 加 `disable-model-invocation`、`user-invocable`（7.3、10.1 节）。

## 15. 值得小型 Python harness 借鉴的设计

以下是基于前文事实的判断，"来源"列指向对应的事实。

| # | 做法 | 为什么值得 | 来源 |
|---|---|---|---|
| 1 | 只追加的事件日志是唯一事实来源，消息历史用纯函数折叠得出 | 恢复、fork、回放、调试、测试共用同一份数据，不会出现两份状态不一致 | 3.1，`packages/core/session/src/index.ts:856-877` |
| 2 | 凡是模型能看到的都必须先落日志，包括系统提示、请求配置和工具 schema | 任何一次请求都能从日志重建。这是回放测试和排查缓存失效的前提 | `docs/architecture.md:127`，`docs/subsystems/session.md:181-183` |
| 3 | surface 只有 `append` 和 `replace` 两种操作，replace 必须引用被遮蔽的事件 | 压缩和裁剪不删历史，可审计，UI 仍能展示完整对话 | 3.2 |
| 4 | `assistant/message` 内嵌原始 chunk 流，失败的尝试单独记成 `assistant/attempt` | 录一次真实会话，就得到一份免 key 的确定性回放夹具 | `docs/subsystems/session.md:87-101`，13.1 |
| 5 | 崩溃恢复和 fork 共用一个尾部修复函数，并区分"结果未知"与"从未开始" | 约 100 行代码解决悬空的工具调用，还把是否可重试的判断明确交给模型 | `packages/core/session/src/repair.ts:15-98` |
| 6 | inbox 分 `next-turn` 和 `next-step` 两个队列，对应 followup、steer、inject 三个动词，inbox 本身持久 | 用户插话、后台完成通知、上下文注入用同一套机制，不必打断模型流 | 2.2 |
| 7 | 少量固定的拦截点加类型化的决定：`agent/pre-step`、`agent/request`、`agent/request-error`、`agent/turn-stopping`，以及三个 `tools/*` waterfall | 压缩、AGENTS.md、重试、检查点、hook、plan 模式都能外挂，循环本体保持在 700 行以内 | 2.1，`docs/architecture.md:109` |
| 8 | 工具管线分层：可重排的 pre 策略、只能拒绝的 guard、环绕式的 execute、post 策略、最终不变式。参数记录后不可改写，一切失败都变成错误结果 | 策略顺序出错也不会放大权限。工具故障不会终止 turn | 5.2 |
| 9 | 工具自报 `isConcurrencySafe(args)`，独占调用作屏障，结果按模型顺序提交 | 并行读、串行写，日志顺序稳定，实现只要一个滚动池 | 5.3 |
| 10 | 重试放在 step 边界而不是流内部，重试计划先落日志再等待，空回复算可重试的错误 | 已经流出去的 chunk 不会被悄悄重放。取消和崩溃时状态仍然一致 | 4.4 |
| 11 | 动态事实做成只在变化时追加的 user 角色快照，系统提示保持字节稳定 | 保住前缀缓存，同时每个状态变化都有据可查 | 7.2 |
| 12 | 输出分三层设防：流级尾部加 spill 文件，结果级头尾预览加定位符，压力期才裁剪旧结果 | 大输出不撑爆上下文，模型又总能按需取回全文 | 5.4 |
| 13 | 压缩是带锁的日志事务，摘要调用复用热前缀，溢出恢复走 `agent/request-error` | 压缩中途崩溃可检测。主动压缩与被动恢复共用一套区间事务 | 7.4 |
| 14 | 审批结果是封闭集合且失败即关闭。沙箱强度是上报的事实（`full/partial`）。拒绝与运行器故障分开分类 | 调用方不会把"没人回答"或"沙箱坏了"误当成"允许"或"命令失败" | 6.2，6.3 |
| 15 | shell 超时后转后台 job，而不是杀进程 | 长构建和长测试不会因超时白跑，模型可以稍后取输出 | 第 11 节 |

顺带两条低成本的做法。重复调用提醒只在第 3、5、8 次追加一句话，不拦截（第 11 节）。AGENTS.md 作为持久的 user 消息注入，按内容摘要刷新，并转义包裹标签（7.3 节）。

### 不建议照搬的部分
| 部分 | 为什么对单人 Python 项目过重 | 来源 |
|---|---|---|
| 完整的 Cordis：Proxy 上下文、`isolate`、`intercept`、YAML 补丁层、profile 与 bundle、文件监听式 HMR | 价值在于几百个包和多个产品形态之间的可替换性。单人项目用一个显式的组合根加 1.4 节的最小集就够 | 1.3，`vendor/README.md` 列出的 23 条本地修改 |
| 每个能力拆成 Definition、Provider、Consumer 三个包，全仓约 319 个子包 | 包边界、生成的目录文档、双语 README 的维护成本远超收益 | `packages/README.md:29-84` |
| 会话格式的多代迁移链：不可变的代文件、Zstandard 分帧、Worker 线程校验、类型指纹目录 | 已发布格式的兼容义务才需要这些。新项目一个版本号加"未知的必需事件即拒绝"就够 | 3.3 |
| surface 的全部细节：历史中途的系统提示节点、`request/context`、`developer/message` 的工具增删、请求深冻结 | 都是为特定提供方的前缀缓存和中途改工具服务的。先实现 append、replace 和节点 0 的系统提示 | `packages/core/agent-loop/README.md:118-120` |
| Windows ACL 受限令牌沙箱 | 需要 FFI、SID、完整性标签和大量边界验证。README 自己列了十多条残留边界，而且仍然只约束写 | 6.3 |
| 可继续子代理的激活管理器、Agent Teams、workflow 引擎、PTC `run_code` | 每个都是独立子系统，带各自的持久状态和生命周期。one-shot 的 spawn 子代理能覆盖大部分需求 | 第 9 节 |
| Typert 生成式 RPC、63 个客户端包的 Web 插件体系、Electron 壳 | 属于产品前端。一个按行输出 JSON 事件的 headless 模式加 JSON-RPC stdio 就能驱动任何前端 | 第 12 节 |
| 逐文件 100% 覆盖率闸门，生成并校验的工具目录、配置目录、持久化目录 | 团队规模的治理工具。保留脚本化 mock、不变量测试和录制回放三样即可 | 13.1 |
| 把会话日志随请求上传给模型提供方，以及匿名用户 id 请求头 | 属于厂商产品的遥测选择，对个人工具是隐私负担 | 4.5 |
| Files API 图片上传、图片 offload、按路由的视觉 token 定价 | DeepSeek 专属的多模态细节 | `packages/llm/llm-deepseek/README.md:94-100` |

## 附录 A：文档与代码不一致

| # | 文档说法 | 代码事实 | 取舍 |
|---|---|---|---|
| 1 | `docs/subsystems/core.md:350` 说核心事件有十三种，列表里没有 `developer/message` | `packages/core/session/src/types.ts:288-427` 声明了 14 个键，含 `developer/message` | 以代码为准 |
| 2 | `docs/subsystems/session.md:681-683` 说 `fork()` 要求前缀结束在 turn 之外，否则拒绝 | `packages/core/session/src/index.ts:1235-1270` 与 `packages/core/session/src/fork.ts:21-30` 接受任意已存在的 seq，并用 `forked` 原因的合成事件关闭未闭合的尾部 | 以代码为准 |
| 3 | `packages/session/session-persistence-jsonl/README.md` 的布局图只列到 `session.v3.*` 并标为 current | 写入版本常量是 4（`packages/core/session/src/types.ts:89`），文件名由 `packages/session/session-format/src/filename.ts:14-17` 生成，应为 `session.v4.*`。`docs/session-format-status.md:31-33,44-47` 记录 finalized 为 4、released 为 3 | 以代码为准 |
| 4 | `docs/subsystems/permission-presets.md:11` 说默认预设表有两个 | `packages/bundle/base/cordis.patch.yml:250-262` 配了三个。前者是服务自身的默认值，后者是 bundle 配置，不算矛盾，但实际产品是三个 | 以 bundle 为准 |
| 5 | `docs/cordis-api/fiber.md:301` 说 fiber 卸载时 disposer 按逆序运行 | `vendor/cordis/src/fiber.ts:675-686` 对顶层 disposer 用 `Promise.all`，逆序启动但并发执行 | 以代码为准 |
| 6 | `docs/glossary.md:13` 说 scope 只有两层且扁平 | `packages/core/scope/src/index.ts:39,72-82` 支持父链（预设作为外层 scope） | 以代码为准 |

## 附录 B：未核实与盲区

- 没有运行任何代码。所有行为结论来自阅读文档和源码，没有实测。Windows 沙箱的实际效果尤其如此。
- 只依据文档或 README、没有打开对应源码的部分：压缩的区间选择与摘要器、token meter 的折叠、JSONL 后端的分帧与恢复、投影注册表、jobs 运行时、schedule、goal 驱动器、ACP 服务器、headless 运行器、Web 与 Desktop 的传输、Agent Teams、Auto review、agent-instructions、MCP 客户端的连接逻辑、hook 桥、Windows ACL 运行器内部。
- 没有阅读：`docs/config-catalog.md`、`docs/module-graph.md`、`docs/event-producer-consumer.md`、`.agents/notes/` 下的设计决策记录、`docs/cookbook/`、`docs/user/`。
- 工具表的"默认"列只核对了 `dsh-base` 和 Web 的 `standard` 预设。headless、sdk、acp 各自的 bundle 是否增减工具未核实。
- `docs/tool-catalog.md` 里的 `bash` schema 没有 `sandbox_permissions` 字段。推测原因是生成器用默认配置启动，没有挂沙箱执行器。该字段的存在以 `packages/shell/tool-bash/src/index.ts:396-404` 为准。
