# Codex CLI harness 设计摘要 (源码级)

读取版本: `openai/codex` 本地克隆, commit `8c818cbed4bc83d30f5d515674b809646fe2676d` (2026-10-08). 只读源码与仓库内文档, 未构建, 未运行.

## 十行摘要

1. 核心在 `codex-rs/core`. 对外只有两条队列: 提交 `Op` 进, 事件 `EventMsg` 出. TUI 与 `codex exec` 都经进程内 app-server 访问核心.
2. 一个回合是一个后台任务. 循环为: 采样请求, 流式接收, 完成的输出项立刻落历史并派发工具, 流结束后按序回填工具结果, 直到模型不再要求跟进且没有待处理输入.
3. 模型层只剩 Responses API. `wire_api = "chat"` 在反序列化时报错. 请求 `store=false`, HTTP 传输下每次重发完整输入, 并带回加密推理项.
4. 工具面的核心是 `exec_command` 加 `write_stdin` (unified exec) 和 freeform 的 `apply_patch`. 旧的 `shell`, `local_shell` 工具已不再派发.
5. 审批策略 4 种, 沙箱模式 3 种加 1 种外部沙箱. `ToolOrchestrator` 统一执行 "审批, 选沙箱, 运行, 被拒后升级".
6. Windows 未配置时沙箱级别为 `Disabled`. 可选三种后端: 受限令牌 (unelevated), 独立账户加 ACL 加 WFP (elevated), 微软 MXC 容器 (开发中).
7. 上下文是追加式历史. 环境, 权限, AGENTS.md 等按分节 diff 追加. 自动压缩阈值是上下文窗口的 90%.
8. 持久化以 JSONL rollout 为唯一事实来源. 压缩记录自带替换历史, 恢复时从最新压缩点回放.
9. 扩展点: skills, hooks (12 个事件), MCP 客户端, plugins, 动态工具, `ext/*` 扩展 crate, 子代理 v1 与 v2.
10. 与常见假设不符: Chat Completions 已移除, 本快照里没有 MCP server 模式, 没有 "已知安全命令" 白名单, 回合内没有迭代次数上限, Plan 模式没有工具级写保护.

**路径约定**: 无前缀的路径相对 `codex-rs/`. 以 `ROOT/` 开头的路径相对仓库根. 行号对应上述 commit.

**未核实项**: 沙箱的实际运行效果 (只读了源码与 README); Seatbelt 策略文件的逐条内容; `ext/queue`, `ext/history-notes`, `ext/image-generation`, realtime 语音的内部实现; 记忆两阶段的选取与合并细节 (以 `memories/README.md` 为准, 只抽查了启动条件与合并子代理的配置); 受信任与未受信任项目在启动时各自落到哪种沙箱模式; `codex exec` 除 0 和 1 之外是否还有其他退出码; 远端模型目录与仓库自带的 `models.json` 是否一致.

## 1. 总体结构

Workspace 有 155 个成员 (`Cargo.toml:2-158`). 与 harness 相关的分组:

| 分组 | crate |
|---|---|
| 协议 | `protocol`, `app-server-protocol`, `exec-server-protocol`, `code-mode-protocol` |
| 核心 | `core`, `core-api`, `tools`, `prompts`, `context-fragments`, `history`, `features`, `config` |
| 模型 | `codex-api`, `codex-client`, `http-client`, `model-provider`, `model-provider-info`, `models-manager` |
| 执行与沙箱 | `apply-patch`, `execpolicy`, `shell-command`, `sandboxing`, `linux-sandbox`, `bwrap`, `windows-sandbox-rs`, `mxc-sandbox`, `network-proxy`, `exec-server`, `utils/pty`, `utils/output-truncation` |
| 持久化 | `rollout`, `rollout-trace`, `thread-store`, `state`, `message-history` |
| 扩展 | `hooks`, `skills`, `codex-mcp`, `rmcp-client`, `plugin`, `core-plugins`, `memories/{read,write}`, `agent-roles`, `agent-graph-store`, `ext/*` |
| 前端 | `tui`, `exec`, `cli`, `app-server`, `app-server-client`, `app-server-transport`, `app-server-daemon`; SDK 在 `ROOT/sdk/` |

核心与前端的分离:

- `Session::spawn` 创建容量 512 的有界提交通道和无界事件通道 (`core/src/session/mod.rs:514`, `:602-603`). `submission_loop` 逐条取出 `Submission { id, op, trace, ... }` 并按 `Op` 分派 (`core/src/session/submission.rs:9-20`, `core/src/session/handlers.rs:439-754`).
- 对外句柄是 `CodexThread`, 提供 `submit(op)` 与 `next_event()` (`core/src/codex_thread.rs:208`, `:258`, `:765`). `core-api` 把 `ThreadManager` 等重导出为公共门面 (`core-api/src/lib.rs:1`, `:31-60`).
- 事件信封是 `Event { id, msg }`, `id` 等于触发它的提交 id (`protocol/src/protocol.rs:1382-1387`).
- TUI 不依赖 `codex-core`, 只依赖 app-server 客户端与协议 crate (`tui/Cargo.toml:32-34`, `:58`). `codex exec` 使用 `InProcessAppServerClient` (`exec/src/lib.rs:19-25`). 两者共用一个进程内 app-server, 走带类型的内存通道 (`app-server-client/README.md:1-38`).
- app-server 向 IDE 与 SDK 暴露 thread, turn, item 三层模型. 消息形似 JSON-RPC 2.0 但不带 `"jsonrpc"` 字段 (`app-server-protocol/src/rpc.rs:1-2`). 传输支持 stdio, unix socket, websocket (`app-server-transport/src/transport/mod.rs:81-86`, `:100`).
- 文档与代码不一致: `docs/protocol_v1.md:20`, `:69` 仍写 `Op::ConfigureSession` 与 `Op::UserTurn`, 第 30 行说新输入会中止当前任务. 代码里是 `Op::TurnInput`, 其 `StartOrSteer` 模式把新输入并入正在运行的回合, 而不是中止它. 文档第 5 行自己声明可能与代码不符. 以代码为准. `ROOT/docs/*.md` 多数是三行的外链占位 (例如 `ROOT/docs/sandbox.md:1-3`).

主要 `Op` (`protocol/src/protocol.rs:588-806`):

| Op | 含义 |
|---|---|
| `TurnInput { request, mode, reply }` | 提交输入. `mode` 为 `StartOrSteer`, `StartIfIdle`, `ContinueIfIdle`, `Steer` (`:641-645`) |
| `Interrupt`, `InterruptIfNoPendingInput` | 中止当前回合. 不终止后台终端 (`:589-599`) |
| `ExecApproval`, `PatchApproval` | 审批答复, 携带 `ReviewDecision` (`:693-709`) |
| `UserInputAnswer`, `RequestPermissionsResponse`, `DynamicToolResponse`, `ResolveElicitation` | 答复工具侧正在等待的请求 (`:713-751`) |
| `ThreadSettings`, `TurnSettings` | 修改线程级或当前回合的设置 (`:666-681`) |
| `Compact`, `Review`, `RunUserShellCommand` | 手动压缩, 进入审查, 用户 `!cmd` (`:765`, `:784`, `:800-805`) |
| `InterAgentCommunication` | 代理间邮件 (`:686-689`) |
| `RecoverTurn`, `SuspendTurnAndShutdown` | 恢复或挂起未完成的回合 (`:649-659`) |
| `CleanBackgroundTerminals`, `RefreshMcpServers`, `ReloadUserConfig`, `Shutdown`, `RealtimeConversation*` | 其余控制操作 (`:603-792`) |

`TurnInputMode` 与提交结果 `TurnInputSubmission { Started, Steered, NotSubmitted }` 定义在 `protocol/src/turn_input.rs:157-167`, `:214-230`.

主要 `EventMsg` (`protocol/src/protocol.rs:1400-1620`):

| 类别 | 事件 |
|---|---|
| 回合 | `TurnStarted` (线上名 `task_started`), `TurnComplete` (`task_complete`), `TurnAborted`, `TokenCount`, `TurnDiff` |
| 输出流 | `ItemStarted`, `ItemCompleted`, `AgentMessage`, `AgentMessageContentDelta`, `AgentReasoning*`, `ReasoningContentDelta`, `PlanDelta`, `RawResponseItem` |
| 工具 | `ExecCommandBegin`, `ExecCommandOutputDelta`, `ExecCommandEnd`, `TerminalInteraction`, `PatchApplyBegin`, `PatchApplyEnd`, `McpToolCallBegin`, `McpToolCallEnd`, `WebSearchBegin`, `WebSearchEnd`, `ViewImageToolCall`, `PlanUpdate`, `DynamicToolCallRequest` |
| 审批与询问 | `ExecApprovalRequest`, `ApplyPatchApprovalRequest`, `RequestPermissions`, `RequestUserInput`, `ElicitationRequest`, `GuardianAssessment` |
| 错误 | `Error`, `Warning`, `StreamError`, `DeprecationNotice` |
| 上下文与模式 | `SessionConfigured`, `ContextCompacted`, `ThreadGoalUpdated`, `EnteredReviewMode`, `ExitedReviewMode`, `HookStarted`, `HookCompleted` |
| 多代理 | `CollabAgentSpawnBegin`, `CollabAgentSpawnEnd`, `CollabWaitingBegin`, `CollabWaitingEnd`, `SubAgentActivity` 等 |

## 2. 回合生命周期

- **入口**. `Op::TurnInput` 进入 `start_or_steer`. 先尝试 `steer_input` 把输入并入活动回合. 没有活动回合时才 `spawn_task(RegularTask)` (`core/src/session/turn_input.rs:323-446`, `:794-888`).
- **任务**. `spawn_task` 先以 `Replaced` 中止旧任务, 再用 `CancellationToken` 启动一个 tokio 任务 (`core/src/tasks/mod.rs:272-425`). 任务接口是 `SessionTask { kind, run, abort }` (`:180-220`).
- **任务种类**. `TaskKind` 只有 `Regular`, `Review`, `Compact` (`core/src/state/turn.rs:65-70`). 用户 shell 命令任务也报告为 `Regular` (`core/src/tasks/user_shell.rs:75-77`). `Review` 与 `Compact` 回合不接受并入输入 (`core/src/session/turn_input.rs:823-835`).
- **外层循环**. `RegularTask::run` 发出 `TurnStarted`, 然后在仍有待处理输入时反复调用 `run_turn` (`core/src/tasks/regular.rs:40-125`).
- **`run_turn` 的步骤** (`core/src/session/turn.rs:164-839`):
  1. 采样前压缩检查 (`:184-223`, `:1304-1332`).
  2. 捕获 `StepContext`. 它是一次请求的设置与工具快照, 保证上下文, 已公布的工具, 工具调用看到同一视图 (`:259-281`, `:460-461`).
  3. 记录上下文更新, skill 与 plugin 注入, 以及用户输入. 输入先过 `UserPromptSubmit` 钩子 (`:283-404`, `:865-913`).
  4. 进入循环: 取出待处理输入并写入历史 (`:429-448`), 由历史生成请求输入 (`:524-530`), 调用 `run_sampling_request` (`:532-541`).
  5. 采样后: `needs_follow_up = 模型要求跟进 或 有待处理输入` (`:574`). 需要跟进且达到 token 上限时, 先做回合中压缩再继续 (`:609-655`). 不需要跟进时运行 `Stop` 钩子. 钩子可以注入一段提示并让回合继续 (`:657-703`). 之后可选做回合末压缩, 然后退出 (`:719-759`).
  6. 出错: `TurnAborted` 向上返回. 其他错误发出 `EventMsg::Error` 后退出循环, 用户可以继续对话 (`:762-834`).
- **提示组装**. `build_prompt` 汇总输入项, 工具, `parallel_tool_calls: true`, base instructions, 可选的输出 schema (`core/src/session/turn.rs:1563-1592`). 请求体没有 `instructions` 字段. base instructions 作为 developer 消息插在 `input` 最前面 (`core/src/client.rs:988-998`, `codex-api/src/common.rs:279-304`). 快照显示首个请求的布局是: 模型指令 (developer), skills 与权限说明 (developer), AGENTS.md 与环境上下文 (user), 用户消息 (`core/tests/suite/snapshots/all__suite__model_visible_layout__model_visible_layout_cwd_change_refreshes_agents.snap:9-18`).
- **流式处理** (`core/src/session/turn.rs:2515-3194`). `OutputItemAdded` 开启一个 turn item. `OutputTextDelta` 转成 `AgentMessageContentDelta`. `OutputItemDone` 交给 `handle_output_item_done` (`:2681-2813`). `Completed` 记录 token 用量并结束接收 (`:2949-2998`). 流在 `response.completed` 之前关闭会报 `CodexErr::Stream` 并进入重试 (`:2661-2665`).
- **工具派发**. `ToolRouter::build_tool_call` 把 `FunctionCall`, `CustomToolCall`, 客户端执行的 `ToolSearchCall` 转成 `ToolCall` (`core/src/tools/router.rs:257-309`). 工具调用项立即写入历史. 工具 future 压入 `FuturesOrdered`, 在流继续时并发执行 (`core/src/stream_events_utils.rs:325-358`, `core/src/session/turn.rs:2784-2786`). 流结束后 `drain_in_flight` 按顺序收集输出并写入历史 (`:2462-2490`, `:3159-3168`).
- **完成**. `on_task_finished` 发出 `TurnComplete { last_agent_message, ... }`, 刷新 rollout, 再检查是否有待启动的回合 (`core/src/tasks/mod.rs:631-891`).
- **中断**. `Op::Interrupt` 调用 `abort_all_tasks(Interrupted)` (`core/src/session/mod.rs:5063-5070`). 流程是: 取消 token, 最多等 100 ms, 强制中止任务, 向历史写入 `<turn_aborted>` 标记, 刷新 rollout, 发出 `TurnAborted` (`core/src/tasks/mod.rs:71`, `:925-1030`). 被取消的工具调用返回一条 "aborted by user" 形式的输出 (`core/src/tools/parallel.rs:365-382`).
- **回合中引导 (steer)**. 输入被追加到活动回合的 `pending_input` (`core/src/session/turn_input.rs:877-883`), 在下一次请求前写入历史. `preempt` 令牌可以提前截断当前采样, 以便尽快投递 (`core/src/session/turn.rs:1614-1621`, `:2636-2654`).

## 3. 模型层

- **Wire API**. `WireApi` 只有 `Responses` (`model-provider-info/src/lib.rs:107-111`). 配置写 `"chat"` 会得到一条明确的错误 (`:100`, `:130`). 这与 "同时支持 Chat Completions" 的假设不符.
- **Provider**. 内置 `openai`, `amazon-bedrock`, `amazon-bedrock-runtime`, `ollama` (端口 11434), `lmstudio` (端口 1234) (`model-provider-info/src/lib.rs:653-692`). 用户在 `config.toml` 的 `model_providers` 里追加. 除 Bedrock 外不能覆盖内置项 (`:694-734`). `ModelProviderInfo` 的字段包括 `base_url`, `env_key`, `auth` (命令取 token), `http_headers`, `env_http_headers`, `query_params`, 各项重试与超时, `supports_websockets`, `capabilities` (`:139-208`).
- **默认值**. `request_max_retries` 4, `stream_max_retries` 5, 流空闲超时 300000 ms, websocket 连接超时 15000 ms. 用户配置的重试次数上限为 100 (`model-provider-info/src/lib.rs:66-75`).
- **HTTP 重试**. 重试 5xx 与传输错误, 不重试 429. 基础延迟 200 ms, 每次翻倍, 加 ±10% 抖动. 服务器给出 Retry-After 时以它为准 (`model-provider-info/src/lib.rs:454-460`, `codex-client/src/retry.rs:22-52`, `:84-118`).
- **流重试**. 错误是否可重试由 `CodexErr::retry_delay` 一处决定 (`protocol/src/error.rs:397-446`). 不可重试的包括 `ContextWindowExceeded`, `UsageLimitReached`, `QuotaExceeded`, `InvalidRequest`, `Fatal`, `Sandbox`. 可重试的包括 `Stream`, `Timeout`, `ConnectionFailed`, `InternalServerError`, `RateLimitExceeded`. 无服务器建议时用 `200ms * 2^(n-1)` 加抖动 (`async-utils/src/backoff.rs:7-17`). 重试时向前端发 `StreamError` 事件 (`core/src/responses_retry.rs:141-167`).
- **断网**. `unbounded_connection_retries` 默认开启 (`features/src/lib.rs:1401`). 连接失败时从 5 s 起等待, 每次翻倍到 60 s 封顶, 不消耗重试预算 (`core/src/responses_retry.rs:23-24`, `:93-118`).
- **传输降级**. 重试预算用尽后, 从 WebSocket 切到 HTTPS 并把计数清零 (`core/src/responses_retry.rs:120-139`).
- **SSE 解析**. 工具调用只在 `response.output_item.done` 到达时作为完整项处理. 文本与推理摘要的增量只用于流式展示. `response.function_call_arguments.delta` 被忽略. 终止事件是 `response.completed`, `response.failed`, `response.incomplete` (`codex-api/src/sse/responses.rs:344-500`). 空闲超时产生 `Stream` 错误 (`:540`, `:567`).
- **请求体**. `store: false`, `stream: true`, `tool_choice: "auto"`, `include: ["reasoning.encrypted_content"]`, 带 `prompt_cache_key` (`core/src/client.rs:1018`, `:1035`, `:1053-1069`). 模型元数据带 `use_responses_lite` 时走另一种形态: 工具定义作为一条 `additional_tools` 项放进 `input`, 顶层 `tools` 为空, 并关闭并行工具调用 (`:972-987`, `:1058`).
- **推理项**. `ResponseItem::Reasoning { summary, content, encrypted_content }` 留在历史里并随请求重发 (`protocol/src/models.rs:1051-1063`, `core/src/context_manager/history.rs:1043`). 估算 token 时对较早的加密推理项单独计数, 除非服务器声明已包含 (`core/src/context_manager/history.rs:893-951`). 走 WebSocket 时可以只发增量并引用 `previous_response_id` (`core/src/client.rs:11-18`, `:286-287`, `:1454-1519`).
- **提示缓存**. `prompt_cache_key` 默认取会话 id (`core/src/client.rs:638-650`). 每次请求重新生成的项使用确定性的 UUIDv5 id, 源码注释说明目的是保住缓存 (`core/src/client.rs:965-970`, `core/src/context_manager/normalize.rs:18-19`, `:140-153`). 上下文变化以追加消息的方式表达, 不改写前缀 (见第 6 节). 请求体把路由字段放在最前 (`codex-api/src/common.rs:280-281`).
- **模型元数据**. `ModelInfo` 来自模型目录, 携带 `truncation_policy`, `context_window`, `tool_mode`, `apply_patch_tool_type` 等 (`protocol/src/openai_models.rs:406-513`). 未知模型使用兜底值: 上下文 272000, 截断 10000 字节, 不提供 `apply_patch` 工具 (`models-manager/src/model_info.rs:98-149`).

## 4. 工具

**声明方式**. `ToolSpec` 序列化后就是 Responses 的工具对象, 有 `function`, `custom` (freeform, 带语法), `namespace`, `tool_search`, `web_search` 五种 (`tools/src/tool_spec.rs:20-56`). `ToolExecutor` trait 把 `spec()`, `handle()`, `supports_parallel_tool_calls()`, `exposure()` 绑在同一个对象上 (`tools/src/tool_executor.rs:105-130`). 暴露级别有 `Direct`, `Deferred`, `DirectModelOnly`, `DeferredModelOnly`, `CodeModeOnly`, `Hidden` (`:50-80`). 注册由 feature 开关和模型元数据共同决定 (`core/src/tools/spec_plan.rs:1195-1215`).

| 工具 | 一句话 | 可并行 | 来源 |
|---|---|---|---|
| `exec_command` | 在 PTY 或管道里运行 shell 命令. 返回输出, 或返回仍在运行的会话 id | 是 | `core/src/tools/handlers/shell_spec.rs:95-114` |
| `write_stdin` | 向已有会话写字符. 空字符串表示只轮询 | 是 | `:117-158` |
| `apply_patch` | freeform 工具, 输入受 Lark 语法约束 | 否 | `core/src/tools/handlers/apply_patch_spec.rs:9-28` |
| `update_plan` | 更新步骤清单 | 否 | `core/src/tools/handlers/plan_spec.rs:42-56` |
| `view_image` | 读取本地图片并作为图像输入返回 | 是 | `core/src/tools/handlers/view_image_spec.rs:16-50` |
| `request_user_input` | 向用户提出带选项的问题. 受配置开关与协作模式限制 | 否 | `core/src/tools/handlers/request_user_input_spec.rs:9`, `:108-113`, `core/src/tools/spec_plan.rs:1349-1356` |
| `request_permissions` | 申请额外的文件或网络权限 (开发中) | 否 | `core/src/tools/handlers/shell_spec.rs:161-196` |
| `list_mcp_resources`, `list_mcp_resource_templates`, `read_mcp_resource` | MCP 资源访问 | 是 | `core/src/tools/handlers/mcp_resource_spec.rs:25`, `:54`, `:83` |
| `tool_search` | 用 BM25 检索延迟加载的工具 | 是 | `core/src/tools/handlers/tool_search.rs:14-17` |
| `web_search` | Responses 托管工具, 不在本地执行. 对 Responses Lite 模型不下发, 改用扩展工具 `web.run` | 托管 | `core/src/tools/hosted_spec.rs:14-46`, `core/src/tools/spec_plan.rs:705-734` |
| `multi_agent_v1.*` | `spawn_agent`, `send_input`, `resume_agent`, `wait_agent`, `close_agent` | 否 | `core/src/tools/spec_plan.rs:1587-1607` |
| 多代理 v2 | `spawn_agent`, `send_message`, `followup_task`, `wait_agent`, `interrupt_agent`, `list_agents` | 否 | `:1497-1578` |
| `clock.sleep`, `clock.curr_time` | 休眠 1 ms 到 12 h, 读当前时间 | 否 | `core/src/tools/handlers/sleep.rs:26-28`, `core/src/tools/handlers/current_time.rs:24-25` |
| `exec`, `wait` | code mode: 模型写 JavaScript 调用其他工具 | 否 | `code-mode-protocol/src/lib.rs:54-55` |
| `new_context`, `get_context_remaining` | token 预算实验功能 | 否 | `core/src/tools/spec_plan.rs:1401-1404` |
| `get_goal`, `create_goal`, `update_goal` | 线程目标 (扩展) | 否 | `ext/goal/src/spec.rs:9-11` |
| `memories.*`, `web.run`, `image_gen.imagegen` | 记忆, 独立网页搜索, 图像生成 (扩展) | 视工具 | `ext/memories/src/lib.rs:18-22`, `ext/web-search/src/tool.rs:41-42`, `ext/image-generation/src/lib.rs:8-9` |

**`apply_patch` 格式**. 语法在 `core/assets/tools/apply_patch.lark:1-19`. 外层是 `*** Begin Patch` 与 `*** End Patch`. 文件操作有 `*** Add File:`, `*** Delete File:`, `*** Update File:` (可带 `*** Move to:`). 更新块用 `@@` 给出定位上下文, 行首用 `+`, `-`, 空格. 解析是宽松的, 会剥掉 heredoc 包装 (`apply-patch/src/parser.rs:47-53`, `:225-254`). 定位上下文分四级匹配: 精确, 忽略行尾空白, 忽略两端空白, 归一化 Unicode 标点 (`apply-patch/src/seek_sequence.rs:35-108`). 应用不是原子的. 失败时返回已经落盘的那部分变更 (`apply-patch/src/lib.rs:278-286`). 模型若在 shell 里调用 `apply_patch`, `exec_command` 会拦截并走同一条路径 (`core/src/tools/handlers/unified_exec/exec_command.rs:365`, `core/src/tools/handlers/apply_patch.rs:436-449`).

**为什么不用整文件写入**. 仓库给出的理由只有一句: 该格式 "designed to be easy to parse and safe to apply" (`core/gpt_5_2_prompt.md:256`). 以下是从代码推断的理由, 不是仓库的原话: (a) freeform 加语法约束, 模型不必把代码转义进 JSON (`core/src/tools/handlers/apply_patch_spec.rs:18-27`); (b) 补丁在执行前先对照文件系统校验, 审批界面能展示确切变更 (`apply-patch/src/invocation.rs:141-166`); (c) 可以按路径判断是否落在可写根内, 从而自动放行 (`core/src/safety.rs:146-180`); (d) 输出 token 只与改动量成正比.

**Shell 与 exec**.

- 只剩 unified exec. `ConfigShellToolType` 只有 `UnifiedExec` 与 `Disabled`, 旧名字是别名 (`protocol/src/openai_models.rs:316-320`). `LocalShellCall` 不再被转成工具调用 (`core/src/tools/router.rs:257-309`).
- 参数: `cmd`, `workdir`, `tty`, `yield_time_ms`, `max_output_tokens`, `shell`, `login`, `sandbox_permissions`, `justification`, `prefix_rule` (`core/src/tools/handlers/shell_spec.rs:35-93`, `:232-278`).
- 让出而不是超时. `exec_command` 默认等待 10000 ms 后返回, 范围夹在 250 到 30000 ms, Windows 下限 10000 ms. 进程继续运行并返回会话 id. `write_stdin` 写入时默认等 250 ms. 空轮询至少 5000 ms, 上限默认 300000 ms (`core/src/unified_exec/mod.rs:68-74`, `:212-219`, `core/src/tools/handlers/unified_exec.rs:60-66`).
- 受管策略禁用 unified exec 时改用一次性变体, `timeout_ms` 默认 10000 (`core/src/tools/spec_plan.rs:1295-1303`, `core/src/tools/handlers/unified_exec/exec_command.rs:305`, `core/src/exec.rs:63`).
- 采集缓冲: `HeadTailBuffer` 最多保留 1 MiB, 头尾各一半, 丢弃中段并插入 `... N bytes omitted ...` (`core/src/unified_exec/head_tail_buffer.rs:5-19`, `:106-124`, `core/src/unified_exec/mod.rs:75`, `:225-227`).
- 面向模型的截断: 预算取 `max_output_tokens` (默认 10000 token) 与模型 `truncation_policy` 中较小者. 从中间截断, 头尾各占一半, 按 4 字节一个 token 估算, 中间放 `…N tokens truncated…`, 开头加一行说明原始 token 数与总行数 (`core/src/tools/context.rs:482-522`, `utils/output-truncation/src/lib.rs:23-41`, `utils/string/src/truncate.rs:4`, `:62-99`, `:156-167`).
- 输出文本带固定头部: chunk id, 墙钟时间, 退出码或仍在运行的会话 id, 原始 token 数 (`core/src/tools/context.rs:524-548`).
- 最多 64 个进程. 超出时按最近最少使用淘汰, 保护最近的 8 个, 优先淘汰已退出的 (`core/src/unified_exec/mod.rs:77`, `core/src/unified_exec/process_manager.rs:1711-1782`).
- 管道排空最多等 2 s, 防止孙进程占着管道导致挂死 (`core/src/exec.rs:87-94`).

**并行工具调用**. 请求里 `parallel_tool_calls` 为真, Responses Lite 模型除外 (`core/src/session/turn.rs:1577`, `core/src/client.rs:1058`). 运行时用一把读写锁: 声明可并行的工具拿读锁, 其余拿写锁 (`core/src/tools/parallel.rs:213-217`). 默认不可并行 (`tools/src/tool_executor.rs:122-124`). MCP 工具只有标记为只读时才并行 (`core/src/tools/handlers/mcp.rs:142-145`).

**`update_plan`**. 参数是 `{ explanation?, plan: [{ step, status }] }`, `status` 为 `pending`, `in_progress`, `completed` (`protocol/src/plan_tool.rs:9-29`). 处理器不保存状态, 只发出 `EventMsg::PlanUpdate` 并回一句确认 (`core/src/tools/handlers/plan.rs:22`, `:93-98`). 工具描述要求同时只有一个 `in_progress`, 但代码不校验 (`core/src/tools/handlers/plan_spec.rs:44-47`, `core/src/tools/handlers/plan.rs:108-112`).

**`view_image`, 网页搜索, code mode**. `view_image` 接受本地路径, 可选 `detail` (`core/src/tools/handlers/view_image_spec.rs:16-40`). 网页搜索模式为 `Disabled`, `Cached` (默认), `Indexed`, `Live` (`protocol/src/config_types.rs:376-381`). `ToolMode` 有 `Direct`, `CodeMode`, `CodeModeOnly` (`protocol/src/openai_models.rs:348-352`). 模型目录里的 `tool_mode` 优先于 feature 开关 (`core/src/tools/mod.rs:63-85`). 仓库自带的 `models-manager/models.json` 中 11 个模型有 10 个声明 `code_mode_only`, 所以 code mode 虽然 feature 标为开发中 (`features/src/lib.rs:1137`), 实际是新模型的默认工具面. code mode 的 `exec` 也是 freeform 工具 (`core/src/tools/code_mode/execute_spec.rs:5-27`).

## 5. 审批与沙箱

审批策略 (`protocol/src/protocol.rs:1028-1051`, `:1054-1068`):

| 配置值 | 枚举 | 语义 |
|---|---|---|
| `untrusted` | `UnlessTrusted` | 未被 execpolicy 规则明确允许的命令都要审批. 注释称其为未受信任项目的内部策略 |
| `on-request` (别名 `on-failure`) | `OnRequest`, 默认 | 由模型决定何时请求审批 |
| `granular` | `Granular { sandbox_approval, rules, skill_approval, request_permissions, mcp_elicitations }` | 分类开关. 某类为 false 时自动拒绝, 不提示 |
| `never` | `Never` | 从不询问. 失败直接返回给模型 |

命令行 `--ask-for-approval` 只开放 `on-request` 与 `never` (`utils/cli/src/approval_mode_cli_arg.rs:9-16`).

沙箱模式 (`protocol/src/protocol.rs:1114-1162`, `protocol/src/config_types.rs:104-114`):

| 配置值 | 语义 |
|---|---|
| `read-only` | 全盘可读, 不可写. 网络默认关闭 |
| `workspace-write` | 全盘可读. 可写 cwd, `writable_roots`, `/tmp`, `$TMPDIR`. 可写根下的 `.git`, `.agents`, `.codex`, `.aws` 仍受保护 (`protocol/src/permissions.rs:40-51`). 网络默认关闭 |
| `danger-full-access` | 无限制 |
| `external-sandbox` | 声明外部已有沙箱. 全盘可写, 网络按设置 |

较新的内部表示是 `PermissionProfile { Managed, Disabled, External }` 加上逐路径的 `Read`, `Write`, `Deny` 条目 (`protocol/src/models.rs:421-435`, `protocol/src/permissions.rs:120-126`, `:233-245`). 三个内置预设: `read-only` (OnRequest 加只读), `auto` (OnRequest 加工作区可写), `full-access` (Never 加无沙箱) (`utils/approval-presets/src/lib.rs:28-60`).

**组合方式**. 所有需要审批或沙箱的工具都经过 `ToolOrchestrator::run` (`core/src/tools/orchestrator.rs:127-607`):

1. 求审批要求: `Skip`, `NeedsApproval`, `Forbidden` (`core/src/tools/sandboxing.rs:154-173`). shell 命令由 execpolicy 评估 (`core/src/exec_policy.rs:337-462`). 其他工具用默认规则 (`core/src/tools/sandboxing.rs:196-232`).
2. 第一次尝试在选定的沙箱里运行. 只有两种情况直接不进沙箱: 命令的每一段都被 execpolicy 明确允许, 或者模型请求了 `require_escalated` 且已获批 (`core/src/tools/sandboxing.rs:249-282`, `core/src/exec_policy.rs:440-454`).
3. 沙箱拒绝后: 只有 `UnlessTrusted` 与开启了 `sandbox_approval` 的 `Granular` 会询问是否去掉沙箱重试. `Never` 与 `OnRequest` 把拒绝结果原样返回给模型 (`core/src/tools/orchestrator.rs:439-463`, `core/src/tools/sandboxing.rs:345-352`). 在 `OnRequest` 下, 提示词要求模型带上 `sandbox_permissions: "require_escalated"` 和 `justification` 重新发起 (`prompts/templates/permissions/approval_policy/on_request.md:26-32`). 在不允许提示的策略下 (例如 `Never`), 这类越权请求在工具处理器里就被拒绝, 并作为工具错误返回 (`core/src/tools/handlers/unified_exec/exec_command.rs:330-339`).
4. 判定沙箱拒绝靠启发式: 特定退出码加输出中的关键词 (`sandboxing/src/denial.rs:13-70`).
5. 审批结果是 `ReviewDecision`: `Approved`, `ApprovedExecpolicyAmendment`, `ApprovedForSession`, `ApprovedMcpPolicyAmendment`, `NetworkPolicyAmendment`, `Denied`, `TimedOut`, `Abort` (`protocol/src/protocol.rs:4181-4216`). 会话级批准按请求键缓存 (`core/src/tools/sandboxing.rs:41-118`).
6. 审批人可以是用户, 也可以是自动审查子代理: `approvals_reviewer = user | auto_review` (`protocol/src/config_types.rs:183-190`).

没有规则命中的命令如何裁决 (`core/src/exec_policy.rs:770-855`):

| 策略 | 危险命令 | 普通命令, 受限文件系统 | 普通命令, 不受限文件系统 |
|---|---|---|---|
| `Never` | 禁止 | 允许 (靠沙箱兜底) | 允许 |
| `UnlessTrusted` | 提示 | 提示 | 提示 |
| `OnRequest`, `Granular` | 提示 | 允许. 若模型请求越权则提示 | 允许 |

"危险命令" 的判定很窄: 带 force 选项的 `rm`, 并穿透 `sudo`, `env`, `trap`, `sh -c`, 另有 Windows 专用规则 (`shell-command/src/command_safety/is_dangerous_command.rs:123-205`). 非测试代码里已找不到 "已知安全命令" 白名单: 全库 grep `is_known_safe_command`, `is_safe_command`, `known_safe` 只命中几个测试函数名 (`core/src/exec_policy_tests.rs:1207`). 在 Windows 上, 如果沙箱后端被禁用而配置又带文件系统限制, 所有未命中的命令按危险命令处理 (`core/src/exec_policy.rs:785-807`).

`apply_patch` 的裁决: 所有目标路径都在可写根内且沙箱可用则自动放行. 否则询问用户. 在 `Never` 下直接拒绝. `UnlessTrusted` 一律询问 (`core/src/safety.rs:67-125`).

各操作系统的实现:

| 系统 | 实现 | 来源 |
|---|---|---|
| macOS | Seatbelt, 经 `/usr/bin/sandbox-exec`, 策略为 `.sbpl` 文件 | `core/README.md:23-35`, `sandboxing/src/seatbelt_base_policy.sbpl` |
| Linux | bubblewrap: `--ro-bind / /` 后叠加可写根, 再把受保护子路径重新只读挂载; 隔离 user, pid, 无网络时隔离 net 命名空间; 进程内加 `PR_SET_NO_NEW_PRIVS` 与 seccomp. 辅助程序靠 arg0 `codex-linux-sandbox` 从同一个二进制启动. 旧的 Landlock 路径已弃用. WSL1 不支持 | `linux-sandbox/README.md:24-58`, `:81-89`, `core/README.md:37-63` |
| Windows, 未配置 | 无沙箱. `WindowsSandboxLevel` 默认 `Disabled`, 没有配置项或旧 feature 时保持该值 | `protocol/src/config_types.rs:297-302`, `core/src/windows_sandbox.rs:125-143` |
| Windows, `unelevated` | `CreateRestrictedToken`, 标志含 `WRITE_RESTRICTED`. 可写根靠能力 SID 加 ACL. 不能限制读. 断网只靠环境变量 (代理指向 `127.0.0.1:9`) 和 `ssh`, `scp` 的桩脚本 | `windows-sandbox-rs/src/token.rs:42-44`, `:500`, `windows-sandbox-rs/src/lib.rs:744-752`, `windows-sandbox-rs/src/env.rs:126-177`, `windows-sandbox-rs/src/spawn_prep.rs:123-145` |
| Windows, `elevated` | 需要一次提权安装. 创建本地账户 `CodexSandboxOffline` 与 `CodexSandboxOnline`, 用 ACL 控制读写, 用 WFP 过滤器按账户断网 | `windows-sandbox-rs/src/setup.rs:53-54`, `windows-sandbox-rs/src/wfp.rs:79-83`, `core/README.md:72-79` |
| Windows, `mxc` | 微软 MXC 容器运行器. `prefer_mxc` 标为开发中 | `mxc-sandbox/README.md:1-38`, `features/src/lib.rs:1365` |

Windows 后端由 `[windows] sandbox = "elevated" | "unelevated" | "mxc"` 选择 (`config/src/types.rs:187-199`). 无法被所选后端执行的策略会直接失败, 不会降级运行 (`core/README.md:87-92`).

**execpolicy**. 规则文件是各配置层目录下的 `rules/*.rules`. 默认文件是 `$CODEX_HOME/rules/default.rules` (`core/src/exec_policy.rs:54-56`, `:662-716`, `:867-869`). 语法是 Starlark: `prefix_rule(pattern, decision, justification, match, not_match)` 与 `host_executable(name, paths)` (`execpolicy/README.md:5-37`). 匹配按 token 前缀顺序进行, `pattern` 中的列表元素表示多选一. `match` 与 `not_match` 是加载时校验的样例. 多条规则命中时取最严格的: `forbidden` 高于 `prompt` 高于 `allow` (`execpolicy/README.md:7-9`, `:95`). `bash -lc` 脚本先拆成多条普通命令再逐条评估 (`core/src/exec_policy.rs:876-904`). 用户批准并选择记住时, 前缀被追加到 `default.rules` (`:464-512`). 过宽的前缀建议被禁止, 例如 `python`, `bash -c`, `node`, `git` (`:57-133`).

## 6. 上下文管理

- **内存表示**. `ContextManager` 持有 `items: Arc<Vec<ResponseItemEnvelope>>`, 以及 token 信息, 参照上下文, world state 基线 (`core/src/context_manager/history.rs:93-127`). `system` 角色的消息不进历史 (`:1028-1051`).
- **工具输出截断**. 写入历史时, 函数输出按模型的 `truncation_policy` 乘 1.2 截断 (`core/src/context_manager/history.rs:565-574`, `utils/output-truncation/src/lib.rs:19-21`). 注释说明这只作用于内存历史, rollout 保留工具产出的原项 (`core/src/context_manager/history.rs:515`).
- **请求前归一化**. 为没有输出的调用补一条内容为 `aborted` 的合成输出. 删除没有对应调用的输出. 模型不支持时剥掉图片与音频 (`core/src/context_manager/history.rs:963-977`, `core/src/context_manager/normalize.rs:21-138`, `:155-225`).
- **token 计数**. 用上一次响应报告的 `total_tokens`, 加上之后本地新增项的估算值 (`core/src/context_manager/history.rs:934-951`).
- **自动压缩的阈值**. `auto_compact_token_limit()` 取配置值与上下文窗口 90% 中较小者 (`protocol/src/openai_models.rs:527-538`). 另有硬上限: 窗口乘 `effective_context_window_percent`, 默认 95 (`core/src/session/context_window.rs:84-86`, `:106-110`).
- **触发时机**. 回合开始前 (`core/src/session/turn.rs:1304-1332`). 回合中需要跟进时 (`:609-655`). 可选的回合末百分比阈值 (`:719-757`). 切到更小窗口的模型时, 用旧模型先压缩 (`:1346-1427`). 手动 `Op::Compact` (`core/src/tasks/compact.rs:29-110`).
- **本地压缩路径** (provider 不支持远端压缩时). 把摘要提示作为一条用户消息追加到历史后发给模型. 提示要求写一份给下一个模型的交接摘要, 包含进展与决定, 约束与偏好, 剩余步骤, 关键数据 (`prompts/templates/compact/prompt.md:1-9`). 压缩请求自己超窗时, 删除最旧的一项后重试 (`core/src/compact.rs:338-350`). 新历史由三部分组成: 最近的用户消息, 从新到旧累计到 20000 token 为止, 超出的那条被截断 (`:63`, `:686-750`); 一条以固定前缀开头的摘要消息 (`:383-386`, `:759-761`); 重新注入的初始上下文, 插在最后一条真实用户消息之前 (`:615-671`). 压缩后向用户发一条警告, 提示长线程会降低准确度 (`:421-424`).
- **远端压缩路径**. OpenAI 与 Azure 默认使用 V2: 发送 `compaction_trigger` 项, 服务器返回不透明的压缩项. 保留消息的预算是 64000 token (`model-provider/src/capabilities.rs:31-41`, `model-provider-info/src/capabilities.rs:23-28`, `core/src/compact_remote_v2.rs:73-77`).
- **边界记录**. rollout 里写入 `Compacted { message, replacement_history, window_number, window_id, ... }` (`history/src/lib.rs:287-307`). 前端收到 `ContextCompaction` 类型的 item. `PreCompact` 与 `PostCompact` 钩子可以中止 (`core/src/compact.rs:193-233`).
- **AGENTS.md**. 全局: `$CODEX_HOME/AGENTS.override.md`, 其次 `AGENTS.md` (`codex-home/src/instructions/mod.rs:12-13`, `:43`). 项目: 由 `project_root_markers` (默认 `.git`) 找到项目根, 从根到 cwd 逐级收集. 每个目录按 `AGENTS.override.md`, `AGENTS.md`, 配置的后备文件名的顺序取第一个存在的文件 (`core/src/agents_md.rs:192-296`). 总预算 32 KiB (`config/src/config_toml.rs:75`). 未受信任的项目不加载项目级文件 (`core/src/agents_md.rs:64-66`). 用户级与项目级之间插入分隔行 (`:49`, `:386-419`). 内容变化时, 追加一条声明 "替换此前全部指令" 的新消息 (`core/src/context/world_state/snapshots/codex_core__context__world_state__agents_md__tests__snapshots.snap`).
- **环境上下文**. 一条 `<environment_context>` 用户消息, 含每个环境的 cwd 与 shell, 以及日期, 时区, 网络与文件系统权限 (`protocol/src/protocol.rs:124-125`, `core/src/context/world_state/snapshots/codex_core__context__world_state__environment__tests__snapshots.snap`).
- **World state**. 上下文被拆成若干节: AGENTS.md, 模型指令, 协作模式, 权限, 环境, 工具, 插件等 (`core/src/context/world_state/mod.rs:1-21`). 每一步把当前值与基线比较, 只把变化的节渲染成新消息追加 (`core/src/session/turn.rs:283-284`, `:506-507`). 快照显示第二回合换目录后只追加了新的 AGENTS.md 与环境块 (`core/tests/suite/snapshots/all__suite__model_visible_layout__model_visible_layout_cwd_change_refreshes_agents.snap:19-25`).
- **记忆**. `memories` 功能默认关闭 (`features/src/lib.rs:1251`). 写路径在根会话启动时后台运行. 临时会话, 功能关闭, 子代理会话都会跳过 (`memories/write/src/start.rs:24-38`). 流程分两阶段: 逐条 rollout 抽取, 再由内部子代理做全局合并. 该子代理审批策略为 `Never`, 关闭多代理, 无网络, 产物在 `~/.codex/memories` (`memories/write/src/phase2.rs:314-336`, `memories/README.md:29-121`). 读路径注入记忆摘要, 并提供 `memories.{list,read,search,add_ad_hoc_note}` (`ext/memories/templates/memories/read_path_v2.md:1-44`, `ext/memories/src/lib.rs:18-22`). 文档与代码不一致: `memories/README.md:5-6` 说编排代码在 `core/src/memories/`, 该目录不存在, 实际在 `memories/write/src/`.

## 7. 会话持久化

- **位置与命名**. `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<时间戳>-<thread_id>.jsonl`, 日期用本地时间 (`rollout/src/recorder.rs:1739-1761`, `rollout/src/rollout_file_name.rs:63-75`). 归档目录是 `archived_sessions` (`rollout/src/lib.rs:87-88`).
- **行格式**. 每行一个 JSON 对象: `timestamp`, 可选 `ordinal`, 再平铺 `type` 与 `payload` (`history/src/lib.rs:362-368`, `history/src/rollout_payload.rs:31-72`).

| `type` | 内容 |
|---|---|
| `session_meta` | 线程 id, cwd, 来源, CLI 版本, base instructions, 动态工具, fork 来源 (`protocol/src/protocol.rs:3184-3241`) |
| `response_item` | 一条模型可见的历史项, 可带 harness 元数据 |
| `turn_context` | 该回合的 cwd, 审批策略, 沙箱策略, 模型, 推理强度 (`protocol/src/protocol.rs:3355-3392`) |
| `compacted` | 压缩检查点, 含替换历史 |
| `world_state` | world state 快照或增量 |
| `token_usage_record` | token 用量 |
| `event_msg` | 需要持久化的事件 |
| `inter_agent_communication`, `retained_context`, `security_risk_score`, `realtime_item` | 多代理邮件与其他专用记录 |

- **写入策略**. 历史项除请求控制项外都写入. 事件只写 `TurnStarted`, `TurnComplete`, `TurnAborted`, `TokenCount`, `ItemCompleted` 等. 增量, 审批请求, 开始类事件不写 (`rollout/src/policy.rs:22-51`, `:81-102`, `:130-160`). 持久化的命令输出上限 64 KiB (`:16-19`).
- **刷盘屏障**. 发出回合终止事件前后各刷一次. 中断标记写入后先刷盘再发 `TurnAborted`, 因为有客户端会在收到事件后立刻重读文件 (`core/src/tasks/mod.rs:384-399`, `:885-887`, `:986-990`).
- **恢复**. 逐行读取. 解析失败的行被跳过并计数, 不让整个会话失效 (`rollout/src/recorder.rs:1104-1170`). 结果是 `InitialHistory::Resumed` (`:1172-1188`). 重建时选最新且带 `replacement_history` 的压缩记录作为基底, 只回放其后的记录. 没有这样的记录就全量回放 (`core/src/session/rollout_reconstruction.rs:64-91`, `:206-238`). 同时恢复上一回合设置, 参照上下文, world state 基线, 窗口编号 (`:14-27`).
- **Fork**. `ForkSnapshot` 有两种: 截到第 n 条用户消息之前, 或者按 "此刻被中断" 截取并补上中断标记 (`core/src/thread_manager.rs:199-218`). 结果是 `InitialHistory::Forked(items)` (`history/src/lib.rs:381-386`). `thread/revert` 保持线程 id 不变, 新建一个文件名带 `_<rollout_id>` 后缀的 rollout (`rollout/src/recorder.rs:102-108`).
- **history 文件**. `~/.codex/history.jsonl`, 每行 `{session_id, ts, text}`. 整行一次写入, 减少多进程交错 (`message-history/src/lib.rs:1-15`, `:52`).
- **SQLite**. `state` crate 把 rollout 元数据镜像进本地 SQLite, 供列表与检索使用 (`state/src/lib.rs:1-5`). 历史本身仍以 JSONL 为准 (`thread-store/README.md:22-28`).
- **命令**. `codex resume`, `codex fork`, `codex exec resume`, `codex exec fork` (`cli/src/main.rs:206-225`, `exec/src/cli.rs:173-182`).

## 8. 规划与长任务

- **计划工具**. 见第 4 节. 它是一个展示用的清单. 状态只存在于对话历史与前端.
- **Plan 模式**. 协作模式只有 `Plan` 与 `Default` (`protocol/src/config_types.rs:674-684`). Plan 模式靠一份模板提示约束模型只做不修改仓库的探索 (`collaboration-mode-templates/templates/plan.md`). 流解析器把 `<proposed_plan>` 块从助手文本里剥出来, 作为 `PlanDelta` 发给前端 (`utils/stream-parser/src/assistant_text.rs:22`, `core/src/session/turn.rs:2604-2606`, `docs/protocol_v1.md:77`). 该模式下 `update_plan` 被拒绝 (`core/src/tools/handlers/plan.rs:87-91`), `request_user_input` 可用 (`protocol/src/config_types.rs:700-702`). 在 `core/src` 非测试代码中, `ModeKind::Plan` 只出现 7 处. 其中位于 `core/src/tools` 下的只有 `core/src/tools/handlers/plan.rs:87` 与 `core/src/tools/handlers/request_user_input.rs:83`, 没有一处拦截写类工具. 所以 "Plan 模式不改文件" 是提示层约束.
- **审查模式**. `Op::Review` 的目标可以是未提交变更, 相对某分支, 某次提交, 自定义指令 (`protocol/src/protocol.rs:3499-3521`). `ReviewTask` 启动一个一次性的子线程: 审查准则作为 base instructions, 审批策略固定为 `Never`, 关闭网页搜索与多代理 (`core/src/tasks/review.rs:99-143`). 子线程的最终消息按 JSON 解析成 `ReviewOutputEvent { findings, overall_correctness, overall_explanation, overall_confidence_score }`. 解析失败时先截取首尾花括号之间的子串再试, 仍失败就把全文当作说明 (`:188-208`, `protocol/src/protocol.rs:3534-3539`). 父线程历史里只写回一对用户与助手的汇总消息 (`core/src/tasks/review.rs:212-272`).
- **目标 (goal)**. 线程级持久目标, 状态为 `Active`, `Paused`, `Blocked`, `UsageLimited`, `BudgetLimited`, `Complete`, 可带 token 预算 (`protocol/src/protocol.rs:4121-4158`). 线程空闲且目标仍为 `Active` 时, 运行时用 `start_turn_if_idle` 自动发起续作回合, 输入是一份续作提示 (`ext/goal/src/runtime.rs:425-506`, `ext/goal/templates/goals/continuation.md:1-56`). 预算用尽时改发收尾提示 (`ext/goal/templates/goals/budget_limit.md:1-16`). 模型只有在同一阻塞连续出现至少 3 个目标回合后才可标记 `blocked` (`ext/goal/src/spec.rs:63-70`). 该功能默认开启 (`features/src/lib.rs:1843`).
- **休眠**. `clock.sleep` 最长 12 小时, 有新输入时提前结束 (`core/src/tools/handlers/sleep.rs:28`, `:51`).
- **后台终端**. unified exec 的进程跨回合存活. 中断不会终止它们 (`protocol/src/protocol.rs:589-591`). 清理用 `Op::CleanBackgroundTerminals` (`:601-603`). app-server 提供 `thread/backgroundTerminals/{list,clean,terminate}` (`app-server-protocol/src/protocol/common.rs:750-762`). 仓库里留有专门轮询长命令的 `awaiter` 角色配置 (`core/assets/agent/builtins/awaiter.toml`), 但它的注册代码已被注释掉 (`core/src/agent/role.rs:386-403`).

## 9. 子代理与多代理

- **版本与成熟度**. `MultiAgentVersion` 为 `Disabled`, `V1`, `V2` (`protocol/src/protocol.rs:3141-3145`). feature `multi_agent` 稳定且默认开启 (`features/src/lib.rs:1435`). `multi_agent_v2` 标为稳定但默认关闭 (`:1441`), 自带模型目录里多数模型声明使用 v2. `agent_message_board` 标为开发中 (`:1465`).
- **v1 工具**. `spawn_agent(message 或 items, agent_type, fork_context, model, reasoning_effort)`, `send_input`, `resume_agent`, `wait_agent`, `close_agent` (`core/src/tools/handlers/multi_agents_spec.rs:14`, `:610-646`).
- **v2 工具**. 代理以路径命名, 例如 `/root/task1/task_3`. `send_message` 只投递不触发回合. `followup_task` 会触发回合. `wait_agent` 等待邮箱更新. 另有 `interrupt_agent` 与 `list_agents` (`core/src/tools/handlers/multi_agents_spec.rs:222-223`, `:255-256`, `:305-306`, `:370-371`, `:801-808`).
- **限额**. 线程数默认 6, 嵌套深度默认 1 (`core/src/config/mod.rs:255`, `:265`). v2 每会话并发 4 个, 等待超时 10 s 到 1 h, 默认 30 s (`:256-259`).
- **上下文隔离**. 每个子代理是独立线程, 有自己的历史与 rollout. 起始历史要么只有任务提示, 要么是父线程历史的完整拷贝 (`core/src/agent/control/spawn.rs:695`, `:1331`, `core/src/tools/handlers/multi_agents_spec.rs:627-631`, `:665-668`). 结果以邮件形式回到父线程: `InterAgentCommunication { author, recipient, content, trigger_turn }` (`protocol/src/protocol.rs:846-862`). 模型看到的是 `<subagent_notification>` 块或一条带 `FINAL_ANSWER` 标记的消息 (`core/src/context/subagent_notification.rs:35-45`, `core/src/context/inter_agent_completion_message.rs:41-47`). 父回合已输出最终回答后, 迟到的邮件留到下一回合 (`core/src/state/turn.rs:35-54`).
- **角色**. `agent_type` 对应一份角色配置, 可覆盖指令, 模型, 推理强度. 角色只能收窄能力, 不能扩大 (`core/src/agent/role.rs:1-4`, `:37-49`, `:93-105`). 内置角色是 `default`, `explorer`, `worker` (`:346-385`). `explorer` 对应的配置文件是空的 (`core/assets/agent/builtins/explorer.toml`).
- **代理图**. `AgentGraphStore` 持久化父子派生边及其状态 (`agent-graph-store/src/store.rs:14-57`).
- **留言板**. 代理间共享的频道, 话题, 帖子. 开发中 (`ext/agent-message-board/src/lib.rs:1-4`).
- **协作模式**. 指 Plan 与 Default, 与多代理无关 (`protocol/src/config_types.rs:674-686`).
- **内部子代理**. 审查, 自动审批 (guardian), 记忆合并都以子线程实现 (`core/src/tasks/review.rs:128-137`, `protocol/src/protocol.rs:2901-2907`, `:2958-2960`).
- **提示层约束**. v1 的 `spawn_agent` 描述明确要求: 除非用户或指令文件明确要求, 不要派生子代理 (`core/src/tools/handlers/multi_agents_spec.rs:738-739`).

## 10. 扩展机制

**Skills**. 一个目录加一份 `SKILL.md`. frontmatter 字段为 `name`, `description`, `metadata.short-description` (`skills/src/parser.rs:6-20`). 可选的 `agents/openai.yaml` 提供额外元数据 (`ext/skills/src/loader/mod.rs:19-21`). 限制: 名称 64 字符, 描述 1024 字符, 扫描深度 6 (`:22-32`). 根目录包括项目的 `.codex/skills`, 从项目根到 cwd 各级目录下的 `.agents/skills`, `~/.agents/skills`, 已弃用的 `$CODEX_HOME/skills`, 系统目录, 插件自带目录 (`ext/skills/src/host_roots.rs:24-25`, `:74-183`). 进入上下文的方式是 "目录在前, 正文按需": developer 消息里只放名称, 描述, 路径的清单. 清单预算依次取: 配置的 token 数 (封顶 10000), 上下文窗口的 2%, 窗口未知时 8000 字符 (`ext/skills/src/render.rs:19-22`, `:126-152`). 清单后的用法说明要求模型在决定使用某个 skill 后自己完整读取其 `SKILL.md` (`ext/skills/src/catalog_prompt.rs:7-12`). 用户显式选择某个 skill 时 (`UserInput::Skill`), 相关内容在回合开始时注入 (`protocol/src/user_input.rs:48-52`, `core/src/session/turn.rs:311-321`, `:397-404`).

**Hooks**. 配置在各配置层目录的 `hooks.json`, 或 `config.toml` 的 `[hooks]` (`hooks/src/engine/discovery.rs:343`, `:768-769`, `config/src/hook_config.rs:12-61`). 结构是 "事件名 到 匹配组列表", 每组为 `{ matcher, hooks: [...] }`. 配置里可写的处理器类型有 `command`, `mcp_tool`, `prompt`, `agent` (`config/src/hook_config.rs:154-200`). 其中 `prompt` 与 `agent` 在加载时被跳过并给出 "尚不支持" 的警告, 实际能运行的只有前两种 (`hooks/src/engine/discovery.rs:637-656`). `command` 处理器从 stdin 收到 JSON. 退出码 2 表示阻断 (`hooks/src/events/pre_tool_use.rs:261`). 默认超时 600 s. `SessionEnd` 与 `Interrupt` 默认 1 s, 上限 3 s (`hooks/src/engine/discovery.rs:742-765`). 钩子有信任状态 `Managed`, `Untrusted`, `Trusted`, `Modified`. 跳过信任检查需要显式的危险参数 (`protocol/src/protocol.rs:1681-1686`, `utils/cli/src/shared_options.rs:61-64`). 钩子返回的附加上下文超过 2500 token 时写入临时文件, 上下文里只留头尾预览与文件路径 (`hooks/src/output_spill.rs:12`, `:53-90`). 工具前后钩子在注册表的统一派发处调用 (`core/src/tools/registry.rs:702-712`).

事件与载荷 (`protocol/src/protocol.rs:1624-1637`, `hooks/schema/generated/*.schema.json`). 公共输入字段是 `session_id`, `transcript_path`, `cwd`, `hook_event_name`. 多数事件另带 `model`, `turn_id`, `permission_mode`. 有输出 schema 的事件大多还接受 `continue`, `stopReason`, `systemMessage`, `suppressOutput`. 下表只列各事件专有的部分.

| 事件 | 专有输入字段 | 可用的输出 |
|---|---|---|
| `PreToolUse` | `tool_name`, `tool_input`, `tool_use_id` | `decision`, `reason`, `hookSpecificOutput` |
| `PermissionRequest` | `tool_name`, `tool_input` | `hookSpecificOutput` |
| `PostToolUse` | `tool_name`, `tool_input`, `tool_response`, `tool_use_id` | `decision`, `reason`, `hookSpecificOutput` |
| `UserPromptSubmit` | `prompt` | `decision`, `reason`, `hookSpecificOutput` |
| `SessionStart` | `source` | `hookSpecificOutput` |
| `SessionEnd` | `reason` | 无输出 schema |
| `Stop` | `last_assistant_message`, `stop_hook_active` | `decision`, `reason` |
| `SubagentStart` | `agent_id`, `agent_type` | `hookSpecificOutput` |
| `SubagentStop` | `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message`, `stop_hook_active` | `decision`, `reason` |
| `PreCompact`, `PostCompact` | `trigger` | `continue`, `stopReason` |
| `Interrupt` | 仅公共字段 | `systemMessage` |

**MCP 客户端**. 传输只有 stdio 与 streamable HTTP (`config/src/mcp_types.rs:614-646`). 命令行有 `codex mcp list|get|add|remove|login|logout` (`cli/src/mcp_cmd.rs:74-81`). 工具名加 `mcp__<server>__` 前缀并清洗, 总长上限 128 (`codex-mcp/src/mcp/mod.rs:66-86`, `codex-mcp/src/tools.rs:225-226`). 模型支持 `tool_search` 时, MCP 工具一律注册为延迟加载, 由模型检索后再使用 (`core/src/mcp_tool_exposure.rs:84-88`, `core/src/tools/spec_plan.rs:156`). 审批模式按工具配置为 `auto`, `prompt`, `writes`, `approve` (`config/src/mcp_types.rs:28-34`), 并参考工具注解里的只读, 破坏性, 开放世界提示 (`core/src/mcp_tool_call.rs:1556-1560`). 用户可选 "允许", "本会话允许", "允许且不再询问" (`:1457-1461`). 服务器发起的 elicitation 以 `ElicitationRequest` 事件转给前端.

**MCP server 模式**. 本快照中未找到. workspace 成员里没有 `mcp-server` crate (`Cargo.toml:2-158`), `cli/src/main.rs:148-243` 的子命令里也没有. 全库只剩一处注释提到 (`codex-mcp/src/elicitation.rs:434`).

**动态工具**. 前端可以在会话启动时注册自己的工具. 调用时核心发 `DynamicToolCallRequest` 事件, 前端执行后用 `Op::DynamicToolResponse` 回传 (`protocol/src/dynamic_tools.rs:13-63`, `protocol/src/protocol.rs:744-751`).

**Plugins**. 清单文件是 `.codex-plugin/plugin.json`, 也识别 `.claude-plugin/` 与 `.cursor-plugin/` (`exec-server-protocol/src/protocol.rs:51-55`). 插件可以带 skill 根目录, hooks, MCP 服务器 (`ext/skills/src/host_roots.rs:59-63`, `hooks/src/engine/discovery.rs:249-254`, `ext/mcp/src/plugin.rs`). 插件里的命令在加载时被迁移成 skill (`core-plugins/src/command_migration.rs:14-19`). 命令行有 `codex plugin` (`cli/src/main.rs:170-171`).

**扩展 crate**. `ext/extension-api` 定义了回合生命周期, 工具生命周期, world state, 提示, 回合输入等贡献点 (`ext/extension-api/src/contributors.rs`). goal, memories, web-search 等都通过它接入.

**自定义提示与斜杠命令**. 未找到自定义提示文件机制 (grep `ListCustomPrompts` 无匹配). 斜杠命令是 TUI 里的一个枚举 (`tui/src/slash_command.rs:15-86`).

**配置 profile 与分层**. `--profile <name>` 把 `$CODEX_HOME/<name>.config.toml` 叠加到用户配置之上 (`utils/cli/src/shared_options.rs:34-36`). 优先级从高到低: 受管配置, 会话参数 (`-c key=value`), 项目的 `.codex/config.toml`, 用户 profile, 用户 `config.toml`, 企业云配置, 系统配置 (`config/src/loader/README.md:26-35`).

## 11. 防御性设计

- **没有回合内迭代上限**. 在 `codex-rs` 全库 grep `max_turns`, `max_steps`, `MAX_ITERATIONS`, `max_tool_calls`, `iteration_limit` 等, 没有与回合循环相关的匹配. 循环靠压缩, 预算, 用户中断收敛. 源码注释的意思是: 只要压缩能把用量压到上限以下, 就不必担心死循环 (`core/src/session/turn.rs:620`).
- **重试有界**. 流重试 5 次, HTTP 重试 4 次, 可配置但封顶 100. 流空闲 300 s 判定断开 (`model-provider-info/src/lib.rs:66-75`).
- **畸形工具调用**. 参数 JSON 解析失败时, 把错误作为工具输出返回给模型, 让它自己改 (`core/src/tools/handlers/mod.rs:86-93`). 未知工具返回 `unsupported call: <name>` (`core/src/tools/registry.rs:965`). 只有 `Fatal` 才终止回合 (`core/src/stream_events_utils.rs:400-429`). 解析不了的 SSE 输出项被丢弃并记日志 (`codex-api/src/sse/responses.rs:353-359`).
- **历史不变量**. 每次请求前补齐缺失的工具输出, 删除孤儿输出 (`core/src/context_manager/normalize.rs:21-138`, `:155-225`).
- **上下文溢出**. 采样时遇到 `ContextWindowExceeded`, 把 token 用量标记为已满, 下一回合开始时会先压缩 (`core/src/session/turn.rs:1690-1693`). 压缩请求自身溢出时逐条删最旧的项 (`core/src/compact.rs:338-350`).
- **exec 的各种上限**. 让出时间夹值, 1 MiB 采集缓冲, 64 进程上限, 管道排空 2 s, 每次调用最多 10000 个输出增量事件, 超时退出码 124 (`core/src/unified_exec/mod.rs:68-77`, `core/src/exec.rs:70`, `:85`, `:94`). 用户 `!cmd` 默认 1 小时超时 (`core/src/tasks/user_shell.rs:47`).
- **中断**. 等 100 ms 后强制中止任务 (`core/src/tasks/mod.rs:71`, `:958-966`).
- **自动审批熔断**. 同一回合内连续 3 次拒绝, 或最近 50 次审查里有 10 次拒绝, 就请求中断回合 (`ext/guardian-reviewer/src/circuit_breaker.rs:3-7`, `:53-86`).
- **目标防空转**. token 预算, 以及 "同一阻塞连续 3 回合才算阻塞" 的规则 (`ext/goal/src/spec.rs:63-70`).
- **子代理限额**. 线程数与深度上限 (`core/src/config/mod.rs:255`, `:265`).
- **钩子**. 有超时. `Stop` 钩子再次触发时会收到 `stop_hook_active`, 由钩子自己避免无限续跑 (`core/src/session/turn.rs:659-693`).
- **注入内容的大小上限**. 单条用户输入 1 MiB 字符 (`protocol/src/user_input.rs:10`). AGENTS.md 32 KiB. skills 清单与各类指令片段各有字节或 token 上限 (例如 `core/src/context/world_state/tools.rs:25-26`).
- **持久化容错**. rollout 里的坏行被跳过 (`rollout/src/recorder.rs:1119-1145`).
- **沙箱策略无法执行时失败关闭** (`core/README.md:87-92`).
- **rollout budget**. 整棵线程树共享的 token 预算与提醒. 开发中 (`core/src/rollout_budget.rs:19-48`, `features/src/lib.rs` 中 `rollout_budget` 一项).

## 12. 无界面与 SDK

**`codex exec`** (`exec/src/cli.rs:10-86`, `utils/cli/src/shared_options.rs:9-77`):

| 参数 | 作用 |
|---|---|
| `[PROMPT]` 或 `-` | 提示词. 省略或为 `-` 时读 stdin. 两者都有时 stdin 作为 `<stdin>` 块追加 |
| `--json` (别名 `--experimental-json`) | 向 stdout 输出 JSONL 事件 |
| `-o, --output-last-message FILE` | 把最后一条助手消息写入文件 |
| `--output-schema FILE` | 用 JSON Schema 约束最终回答 |
| `-m, --model`, `--oss`, `--local-provider`, `-p, --profile` | 模型与配置 |
| `-s, --sandbox`, `--approve-for-me`, `--dangerously-bypass-approvals-and-sandbox` | 沙箱与审批 |
| `-C, --cd`, `--add-dir`, `--worktree`, `-i, --image` | 工作目录, 额外可写目录, 工作树, 图片 |
| `--skip-git-repo-check`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--strict-config` | 运行环境控制 |
| 子命令 `resume`, `fork`, `review` | 续接, 分叉, 审查 |

- 行为: 无界面模式默认把审批策略设为 `Never`, 审批人解析为 `auto_review` 时除外 (`exec/src/lib.rs:589-591`). 服务器发来的命令审批, 文件变更审批, 用户提问, 动态工具调用一律拒绝. MCP elicitation 自动取消 (`:2077-2206`). 不在 Git 仓库内且没有 `--skip-git-repo-check` 时直接退出 (`:1010-1018`).
- 输出: 普通模式把过程写到 stderr (`exec/src/event_processor_with_human_output.rs:71-221`). `--json` 模式把 `ThreadEvent` 逐行写到 stdout (`exec/src/event_processor_with_jsonl_output.rs:104-106`). 事件类型为 `thread.started`, `turn.started`, `turn.completed` (带 `usage`), `turn.failed`, `item.started`, `item.updated`, `item.completed`, `error` (`exec/src/exec_events.rs:11-37`). item 类型为 `agent_message`, `reasoning`, `command_execution`, `file_change`, `mcp_tool_call`, `collab_tool_call`, `web_search`, `todo_list`, `error` (`:107-133`).
- 退出码: 出现不可重试的错误通知, 或回合状态为 `Failed` 或 `Interrupted`, 退出码为 1 (`exec/src/lib.rs:1340-1357`, `:1399-1403`). 配置或参数错误也是 1. 其余为 0. 未见更细的退出码.

**SDK**.

- TypeScript (`@openai/codex-sdk`): 启动子进程 `codex exec --experimental-json`, 把提示写入 stdin, 逐行解析 stdout (`ROOT/sdk/typescript/src/exec.ts:95`, `:203-216`, `ROOT/sdk/typescript/README.md:5`).
- Python (`openai-codex`): 通过 stdio 连接 `codex app-server --listen stdio://` 的 JSON-RPC 客户端 (`ROOT/sdk/python/src/openai_codex/client.py:215`, `:256`). 审批只有两档: `deny_all` 与 `auto_review` (`ROOT/sdk/python/src/openai_codex/_approval_mode.py:13-37`). `ROOT/sdk/python-runtime` 负责打包各平台的 CLI 二进制 (`ROOT/sdk/python-runtime/README.md:1-9`).

**app-server 协议**. 客户端请求包括 `initialize`, `thread/start`, `thread/resume`, `thread/fork`, `thread/list`, `thread/read`, `turn/start`, `turn/steer`, `turn/interrupt`, `thread/compact/start`, `review/start`, `command/exec`, `fs/*`, `skills/list`, `hooks/list`, `model/list` 等 (`app-server-protocol/src/protocol/common.rs:500-1384`). 服务器向客户端发起的请求包括 `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/tool/requestUserInput`, `item/tool/call` (`:1784-1815`). 通知包括 `turn/started`, `item/started`, `item/agentMessage/delta`, `item/completed`, `turn/completed`, `thread/tokenUsage/updated` (`:1937-2013`).

## 13. 测试

- **集成测试**. `core/tests/all.rs` 把 `core/tests/suite/` 下的 235 个 `.rs` 文件汇成一个测试二进制 (`core/tests/all.rs:3-7`). 公共夹具在 `core/tests/common/`.
- **模拟模型服务器**. 用 wiremock 起本地 HTTP 服务器. 一组小函数把 JSON 事件拼成 SSE 响应体: `sse`, `ev_response_created`, `ev_assistant_message`, `ev_function_call`, `ev_apply_patch_custom_tool_call`, `ev_completed_with_tokens`, `sse_failed` (`core/tests/common/responses.rs:753-1083`). `mount_sse_once` 与 `mount_sse_sequence` 挂载一次或一串响应, 并返回可以取回已收请求的 `ResponseMock` (`:40`, `:1144-1151`, `:1469`).
- **可控的流**. `start_streaming_sse_server` 接受分块且可设闸门的响应, 用来测流中断与重试 (`core/tests/common/streaming_sse.rs`, 示例 `core/tests/suite/stream_no_completed.rs:26-100`). 还有 WebSocket 测试服务器 (`core/tests/common/responses.rs:589`, `:1220`).
- **会话构造器**. `test_codex()` 返回构造器, 支持 `with_config`, `with_model`, `build(&server)`. 再用 `submit_turn` 与 `wait_for_event` 驱动 (`core/tests/common/test_codex.rs:406-601`, `:1117`, `:1522`).
- **请求形状快照**. `context_snapshot` 把每次请求的输入渲染成 "序号: 类型/角色: 内容" 的行, 把常见上下文块替换成 `<ENVIRONMENT_CONTEXT>` 这样的标签, 并按上下文窗口分组. 用 insta 存成 `.snap` (`core/tests/common/context_snapshot.rs:1-48`). `core/tests/suite/snapshots/` 下有 84 个快照, 例如 `all__suite__compact__mid_turn_compaction_shapes.snap`.
- **单元测试**. 与源码同目录的 `*_tests.rs`. world state 每一节的 diff 渲染都有快照 (`core/src/context/world_state/snapshots/`).
- **确定性辅助**. 测试专用工具 `test_sync_tool` 用于并发同步 (`core/src/tools/handlers/test_sync_spec.rs:59-60`). unified exec 可切换为确定性的进程 id (`core/src/unified_exec/mod.rs:59-61`).
- **可参考的测试文件**. `core/tests/suite/tool_parallelism.rs`, `truncation.rs`, `approvals.rs`, `compact.rs`, `resume.rs`, `pending_input.rs`, `stream_error_allows_next_turn.rs`; `exec/tests/suite/server_error_exit.rs`, `mcp_required_exit.rs`.

## 14. 值得小型 Python harness 借鉴的设计

1. **提交与事件双队列**. 核心只收 `Op`, 只发带提交 id 的 `Event`. 界面, 无界面模式, 测试共用同一个核心, 前端永远不碰循环内部 (`core/src/session/mod.rs:602-603`, `protocol/src/protocol.rs:1382-1387`).
2. **只对完成的输出项动作**. 忽略函数参数增量, 等 `output_item.done` 再解析. 不需要处理半截 JSON (`codex-api/src/sse/responses.rs:353-360`, `:481-489`).
3. **调用先落历史, 执行与流并发, 结果按序回填**. 回合被取消时历史仍然自洽, 多个工具也不必等流结束才开始 (`core/src/stream_events_utils.rs:346-357`, `core/src/session/turn.rs:2462-2490`).
4. **用一把读写锁做并行门**. 工具自己声明是否可并行. 只读工具并发, 有副作用的工具独占. 实现只有几行 (`core/src/tools/parallel.rs:213-217`, `tools/src/tool_executor.rs:122-124`).
5. **两类工具错误**. `RespondToModel` 变成工具输出让模型自我修正. `Fatal` 才终止回合. 畸形参数与未知工具都走第一类 (`core/src/stream_events_utils.rs:400-429`, `core/src/tools/handlers/mod.rs:86-93`).
6. **请求前统一归一化历史**. 补 `aborted` 输出, 删孤儿输出. 中断, 崩溃, 恢复之后都不会因为调用与输出不配对而被 API 拒绝 (`core/src/context_manager/normalize.rs:21-138`, `:155-225`).
7. **两层截断加显式标记**. 采集层限内存 (1 MiB, 头尾各半). 展示层限 token (默认 10000). 标记里写明省略量与原始大小, 模型知道被截断并能决定是否重读 (`core/src/unified_exec/head_tail_buffer.rs:5-19`, `core/src/tools/context.rs:495-522`).
8. **让出式 exec**. 等待固定时间后返回会话 id, 进程继续运行, 之后轮询. 长命令不会卡住回合, 也不需要模型猜超时 (`core/src/unified_exec/mod.rs:68-74`, `core/src/tools/handlers/shell_spec.rs:117-158`).
9. **补丁式编辑加分级模糊定位**. 精确匹配失败后依次放宽空白与标点. 执行前先校验并算出确切变更, 再去审批 (`apply-patch/src/seek_sequence.rs:35-108`, `apply-patch/src/invocation.rs:141-166`).
10. **一个编排器管审批, 沙箱, 重试**. 工具只实现几个小接口: 审批键, 沙箱偏好, 运行. 策略集中在一处, 新工具不会漏掉审批 (`core/src/tools/orchestrator.rs:1-8`, `core/src/tools/sandboxing.rs:323-399`).
11. **前缀规则加追加式放行**. 规则是可读的文本文件. 多条命中取最严格. 用户批准时把前缀追加进文件. 同时禁止过宽的前缀 (`execpolicy/README.md:5-9`, `core/src/exec_policy.rs:57-133`, `:464-512`).
12. **上下文更新只追加不改写**. 把环境, 权限, 指令文件拆成节, 每步与基线比较, 只追加变化的节. 历史保持前缀稳定, 提示缓存不被破坏 (`core/src/context/world_state/mod.rs:1-21`, 快照 `core/tests/suite/snapshots/all__suite__model_visible_layout__model_visible_layout_cwd_change_refreshes_agents.snap`).
13. **压缩等于交接摘要, 加近期用户原话, 加检查点**. 摘要按 "写给接手者" 来要求. 用户消息原样保留一段. 压缩记录自带替换历史, 恢复时不必重放全部 (`core/src/compact.rs:383-386`, `:686-764`, `history/src/lib.rs:287-307`, `core/src/session/rollout_reconstruction.rs:64-91`).
14. **JSONL 作为唯一事实来源**. 追加写, 容忍坏行, 在回合终止事件前设刷盘屏障. 索引类数据库只是可重建的镜像 (`rollout/src/recorder.rs:1119-1145`, `core/src/tasks/mod.rs:384-399`, `state/src/lib.rs:1-5`).
15. **模拟 SSE 加请求形状快照**. 用几十行辅助函数就能离线, 确定性地测整个循环. 快照直接断言 "模型在第 n 次请求里看到了什么", 这是上下文管理最需要的回归测试 (`core/tests/common/responses.rs:753-1083`, `core/tests/common/context_snapshot.rs:1-48`).

另外两个低成本的做法也值得照做: 把 "错误是否可重试, 等多久" 写成一张表放在错误类型上 (`protocol/src/error.rs:397-446`); 中断时往历史里写一条模型可见的标记, 让下一回合知道上一回合没做完 (`core/src/tasks/mod.rs:972-991`).

**不建议照搬的部分**

- **原生沙箱**. 三个系统共五套后端: Seatbelt, bubblewrap 加 seccomp, Windows 受限令牌, Windows 提权账户加 WFP, MXC. 这是整个仓库里最重的部分, 单人项目无法维护. 小项目应保留策略层 (模式, 可写根, 受保护路径, 审批), 执行隔离交给容器或直接不做. 特别注意 Windows 上 Codex 未配置时沙箱级别也是 `Disabled`, unelevated 后端的断网只是环境变量层面的尽力而为 (`windows-sandbox-rs/src/env.rs:126-177`).
- **多环境与远程执行器**. `exec-server`, `environment_id`, `PathUri` 贯穿所有工具. 单机 harness 用不到.
- **Responses 的 WebSocket 传输, 增量请求, 预热**. 属于针对自家后端的延迟优化 (`core/src/client.rs:11-26`).
- **远端压缩 V2 与加密推理项的记账**. 依赖 OpenAI 服务端能力. 换别家模型时只能走本地摘要路径.
- **code mode**. 需要嵌入 V8 运行时和一整套嵌套工具调用协议.
- **guardian 自动审批子代理**. 带独立的上下文预算, 证据裁剪, 熔断, 多套提示. 小项目用规则加人工审批足够.
- **多代理 v2**. 路径寻址, 邮箱, 常驻, 代理图, 留言板. 若需要子代理, v1 的 "派生, 等待, 取最终消息" 加线程数与深度限额就够.
- **app-server 全套**. 约 150 个客户端请求方法 (`app-server-protocol/src/protocol/common.rs:500-1384`), 守护进程, 远程控制. 小项目保留进程内事件队列加一个 JSONL 输出即可.
- **world state 的完整分节体系**. 思路可取, 规模可以缩到三四个节.
- **约 170 个 feature 开关 (`features/src/lib.rs`), 插件市场, connectors, 实时语音, 遥测与分析, SQLite 镜像, rollout 压缩与分页历史, 两阶段记忆流水线**. 都是产品化与规模化的产物.
