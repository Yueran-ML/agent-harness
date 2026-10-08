# Claude Code harness 设计摘要

> 调研日期：2026-10-09。来源仅限公开资料：Claude Code 文档、Agent SDK 文档、Anthropic 工程博客。
> 方括号数字对应文末 URL 列表。文档未说明之处直接标注"文档未说明"。
> 部分页面很长，个别数字来自抓取工具的抽取结果，实现前应对照原页核对 [1][2]。
> 本文由调研子代理整理，尚未逐条人工复核。

## 1. Agent 循环

- 文档把循环描述为收集上下文、采取行动、验证结果三个交织的阶段，每一步由上一步的工具结果决定 [3]。模型之外提供工具并管理上下文的那一层，被文档称为 agentic harness [3]。
- 回合结束：模型输出不再含工具调用即结束 [23]。SDK 的 ResultMessage 用 subtype 说明原因，如 success、error_max_turns、error_max_budget_usd [23]。Stop 钩子在 Claude 结束回复时触发，可用 block 或 exit 2 让对话继续 [13]。
- 中断与转向：Esc 取消当前工具调用，Ctrl+C 中断运行中的操作 [3][22]。运行中输入的消息排队，在当前工具调用结束后、同一回合内读取；Ctrl+Enter 可立即发出队列消息 [22]。
- 交互模式的默认回合上限：文档未说明。无界面模式提供 --max-turns 与 --max-budget-usd [19]。

## 2. 内置工具

下表来自 [4]。

| 工具 | 作用 | 需权限 |
|---|---|---|
| Read | 读取文本、图片、PDF（分页）与 notebook | 否 |
| Write | 新建或整体覆盖文件，不做追加 | 是 |
| Edit | 精确字符串替换 | 是 |
| NotebookEdit | 修改 Jupyter cell | 是 |
| Glob | 按模式找文件，最多返回 100 条 | 否 |
| Grep | 基于 ripgrep 的内容搜索 | 否 |
| Bash | 执行 shell 命令，每条命令独立进程 | 是（只读命令例外） |
| PowerShell | 原生 PowerShell，Windows 上按条件启用 | 是 |
| Monitor | 后台运行命令，并把输出逐行回馈给 Claude | 是 |
| WebFetch | 抓取 URL，并按提示词提取内容 | 是 |
| WebSearch | 返回搜索结果的标题与 URL，不抓取页面 | 是 |
| Agent | 启动拥有独立上下文的子代理 | 否 |
| Skill | 在主会话中执行技能 | 是 |
| EnterPlanMode / ExitPlanMode | 进入计划模式 / 提交计划审批 | 否 / 是 |
| TaskCreate、TaskGet、TaskList、TaskUpdate | 任务清单（旧版 TodoWrite 默认关闭） | 否 |
| AskUserQuestion | 向用户提出选择题 | 否 |
| LSP、ToolSearch | 代码智能；按需加载延迟工具 | 否 |

- Edit 要求 old_string 与文件逐字一致且唯一，否则需扩大上下文或设置 replace_all [4]。
- 先读后改：Opus 4.6、Haiku 4.5 及更早模型必须先读文件；较新模型在读取无需权限时可直接改未读文件，Write 覆盖已有文件同理 [4]。
- 输出与超时：Bash 成功结果内联约 30,000 字符，超出则落盘并返回路径与 2,000 字符预览；默认超时 2 分钟，上限 10 分钟；无人值守运行中，后台命令限时 30 分钟 [4]。Read 默认上限 25,000 token，超出时提示用 offset 与 limit 分页 [4][12]。
- 平台差异：macOS、Linux 与 WSL 默认不带 Glob 与 Grep，改用 Bash 中的 find 与 grep；Windows 默认带有 [4]。

## 3. 权限与安全

| 模式（设置值） | 无需询问即可执行的操作 |
|---|---|
| default（CLI 显示为 Manual） | 仅读取 |
| acceptEdits | 读取、编辑，以及 mkdir、touch、mv、cp 等常见文件命令 |
| plan | 读取；auto 可用时还包括分类器批准的命令 |
| auto | 后台分类器审查动作，拦截有风险的操作 |
| dontAsk | 只执行预先批准的工具，其余直接拒绝 |
| bypassPermissions | 跳过提示，deny 规则仍生效，仅用于隔离环境 |

上表来自 [6]，auto 与 dontAsk 的说明参见 [3][5]。

- 规则写作 Tool 或 Tool(specifier)，如 Bash(npm run *)、Read(./.env)、WebFetch(domain:example.com)、mcp__puppeteer__* [5]。
- 评估顺序为 deny、ask、allow，首个匹配生效，具体程度不改变顺序。PreToolUse 钩子返回 allow 也不能绕过 deny 与 ask [5]。
- Bash 规则按命令文本匹配。Bash(curl *) 挡不住 /usr/bin/curl，文档因此说明它不是安全边界 [5]。
- 设置优先级从高到低为 managed settings、--settings、.claude/settings.local.json、.claude/settings.json、~/.claude/settings.json，permissions.allow 等数组跨文件合并 [21]。权限由 Claude Code 强制执行，提示词与 CLAUDE.md 不能放宽它 [5]。
- 工作目录默认为启动目录，可用 --add-dir、/add-dir 或 additionalDirectories 扩展 [5]。
- 沙箱默认关闭，只覆盖 Bash、PowerShell、Monitor 及其子进程；Read、Edit、Write、WebFetch、MCP 服务器与钩子不在沙箱内 [7]。macOS 使用 Seatbelt，Linux 与 WSL2 使用 bubblewrap，原生 Windows 不沙箱化命令 [7]。默认可写范围为工作目录、添加的目录与临时目录，默认可读范围几乎覆盖整机 [7]。

## 4. 上下文管理

- 启动时加载系统提示、CLAUDE.md、auto memory（MEMORY.md 前 200 行或 25KB）、MCP 工具名与 skill 描述 [3][9]。
- CLAUDE.md 按 managed、用户、项目、本地的顺序拼接，子目录中的文件按需加载。@path 导入最多递归 4 跳，.claude/rules/ 可用 paths 限定作用范围 [8]。文档强调 CLAUDE.md 是上下文而非强制配置，要确定性阻止动作应使用 PreToolUse 钩子 [8]。
- 自动压缩：接近上限时先清理较早的工具输出，再按需摘要 [3]。未设窗口时在模型上下文上限触发；未启用扩展上下文的 Sonnet 4.6 与 Opus 4.6 在 200K 边界压缩，原生 1M 模型约在 967K token 时压缩 [10]。摘要所用的提示词，文档未说明。除清理与摘要外的其他上下文编辑机制，文档未说明。
- 压缩后，项目根 CLAUDE.md、无路径限定的规则、auto memory 与计划从磁盘重新注入；至多 5 个近期文件被重读；已调用 skill 的正文每个上限 5,000 token。早期对话中的具体指令可能丢失 [9]。/compact [focus] 可指定摘要重点，/context 显示各项占用 [3][9]。
- 检查点：每个回合的提示前快照文件，保留最近 100 个。输入框为空时按两次 Esc 或使用 /rewind，可恢复代码、对话或两者，也可从某点起压缩 [11]。检查点只跟踪文件工具的编辑，不跟踪 Bash 命令造成的改动与外部修改，也不能替代 git [11]。

## 5. 规划

- 计划模式下 Claude 只读探索并写出计划，获批前不编辑源码。入口为 Shift+Tab、/plan 或 --permission-mode plan [6][16]。自动模式可用时，计划期间的 shell 命令由分类器审查 [6]。
- 计划通过 ExitPlanMode 提交，选项为 "Yes, and use auto mode"、"Yes, manually approve edits" 与 "No, keep planning"。Ctrl+G 可在编辑器中修改计划，批准后切换到所选模式 [4][6]。
- 任务清单默认提供 TaskCreate、TaskGet、TaskList、TaskUpdate，仅在 Claude 3.x、Opus 4 至 4.7、Sonnet 4 至 4.6 与 Haiku 4.5 上默认开启。其他模型需设置 CLAUDE_CODE_ENABLE_TODO_TOOLS=1，旧版 TodoWrite 默认关闭 [4]。Ctrl+T 切换清单显示，CLAUDE_CODE_TASK_LIST_ID 可跨会话共享 [22]。

## 6. 子代理

- 定义为 Markdown 加 YAML frontmatter，正文即系统提示，仅 name 与 description 必填。其他字段包括 tools、disallowedTools、model、permissionMode、maxTurns、skills、mcpServers、hooks、memory、background、effort、isolation、initialPrompt 等 [14]。
- 优先级依次为 managed、--agents 参数、.claude/agents/、~/.claude/agents/、插件 agents 目录 [14]。
- 内置类型：Explore 只读，禁用 Write 与 Edit；Plan 在计划模式中收集上下文，只读；general-purpose 可用全部子代理工具 [14]。
- 上下文隔离：子代理拥有独立的上下文窗口、系统提示与权限，看不到主对话历史。父代理看不到子代理的中间工具调用，只收到最终结果 [4][14]。
- 并行与嵌套：后台子代理并发运行，同一会话中同时运行的子代理默认上限为 20；默认允许嵌套三层；fork（分叉）继承完整对话，但不能再 fork [14]。达到 maxTurns 时返回部分结果，可恢复 [4]。
- 子代理的工具调用仍受权限规则约束，后台子代理的权限提示显示在主会话中，按 Esc 只拒绝该次调用 [4]。

## 7. 扩展机制

Hooks 的 33 个事件如下，表格来自 [13]。

| 事件 | 触发时机与能力 |
|---|---|
| SessionStart | 会话开始或恢复时；可附加上下文，不能阻止 |
| Setup | --init-only 或 -p 下的 --init、--maintenance；仅副作用 |
| UserPromptSubmit | 提示提交后；可阻止或附加上下文，不能改写提示 |
| UserPromptExpansion | 命令展开为提示时；可阻止展开 |
| PreToolUse | 工具执行前；返回 allow、deny、ask 或 defer，updatedInput 改参数，exit 2 阻止 |
| PermissionRequest | 需要权限决定时；返回 allow 或 deny，可改 updatedInput |
| PermissionDenied | 自动模式拒绝后；retry: true 允许模型重试 |
| PostToolUse | 工具成功后；可阻止，updatedToolOutput 替换结果 |
| PostToolUseFailure | 工具失败后；附加上下文或反馈 |
| PostToolBatch | 并行工具批次结束后、下次模型调用前；可停止循环 |
| Notification | 发送通知时；仅副作用 |
| MessageDisplay | 助手文本显示时；displayContent 只改屏幕显示 |
| SubagentStart | 子代理启动时；附加上下文 |
| SubagentStop | 子代理结束时；可阻止其停止 |
| TaskCreated | 创建任务时；可取消 |
| TaskCompleted | 任务标记完成时；可阻止完成 |
| Stop | Claude 结束回复时；可让对话继续 |
| StopFailure | 回合因 API 错误结束时；输出被忽略 |
| TeammateIdle | 队友即将空闲时；可让其继续工作 |
| InstructionsLoaded | CLAUDE.md 或 rules 加载时；仅观测 |
| ConfigChange | 配置文件变化时；可阻止变更（policy_settings 除外） |
| CwdChanged | 工作目录变化时；仅副作用，如 direnv |
| DirectoryAdded | 通过 /add-dir 添加目录时；仅副作用 |
| FileChanged | 被监视文件变化时；仅副作用 |
| WorktreeCreate | 创建 worktree 时；可替换创建方式，失败则中止 |
| WorktreeRemove | 删除 worktree 时；非零退出会使删除失败 |
| PreCompact | 压缩前；可阻止压缩 |
| PostCompact | 压缩后；仅副作用 |
| PreModelSwitch | 模型切换前；可阻止切换 |
| PostModelSwitch | 模型切换后；附加上下文 |
| Elicitation | MCP 服务器请求用户输入时；可接受、拒绝或取消 |
| ElicitationResult | 用户回应后、回传服务器前；可覆盖字段 |
| SessionEnd | 会话结束时；仅副作用 |

- 处理器类型为 command、http、mcp_tool、prompt，以及实验性的 agent [13]。command 钩子以退出码 2 表示阻止，且输出的 JSON allow 无法覆盖该阻止；其他非零退出码对多数事件只是非阻止性错误 [13]。
- Skills 是含 SKILL.md 的目录，frontmatter 推荐包含 description。位置依次为 managed、~/.claude/skills/、.claude/skills/、嵌套目录与插件，同名时个人级优先于项目级 [15]。描述常驻上下文（列表中截断于 1,536 字符），正文在调用后加载，附属文件按引用读取，脚本被执行而不被加载 [15]。
- 斜杠命令分为内置命令、捆绑 skill 与自定义命令，同名时 skill 优先。回复过程中输入的命令会排队到回合结束后执行 [15][16]。
- 插件可打包 skills、commands、agents、hooks、MCP 与 LSP 等组件，清单位于 .claude-plugin/plugin.json [17]。
- MCP 传输为 HTTP（远程推荐）、SSE（已弃用）与 stdio，WebSocket 只能通过 JSON 配置添加 [18]。作用域为 local（默认）、project（.mcp.json，首次使用需批准）与 user。同名时整条配置取用，字段不合并 [18]。工具命名为 `mcp__<server>__<tool>` [5][18]。

## 8. 会话持久化与无界面模式

- 转录为 `~/.claude/projects/<project>/<session-id>.jsonl`。文档说明条目格式属于内部实现，版本间可能变化，脚本应使用 /export 或输出接口 [20]。
- 续接方式包括 --continue、--resume（接受 ID、名称或转录路径）与 /resume。--fork-session 或 /branch 生成新 ID，原会话不变 [16][20]。会话默认约 30 天后清理，可用 cleanupPeriodDays 调整 [20]。
- 无界面模式使用 claude -p，--output-format 可选 text、json 与 stream-json。stream-json 需配合 --verbose 与 --include-partial-messages 输出逐步事件，末行为 result [19]。json 输出包含 session_id 与 total_cost_usd；配合 --json-schema 时结构化结果位于 structured_output [19]。
- --bare 跳过 hooks、skills、插件、MCP、auto memory 与 CLAUDE.md，文档推荐用于脚本与 SDK 调用。--allowedTools、--permission-mode 与 --permission-prompts none 控制无人值守时的权限 [19]。
- 退出码成功为 0、失败为非零。SIGTERM 退出码为 143，未完成的回合不产生结果；发送 SIGINT 或调用 SDK 的 interrupt() 可干净地结束回合 [19]。
- Agent SDK 运行同一循环，并随包附带原生 Claude Code 二进制 [23]。其消息包括 SystemMessage、AssistantMessage、UserMessage、StreamEvent 与 ResultMessage [23]。权限顺序为 hooks、deny、ask、权限模式、allow、canUseTool 回调 [25]。其他语言可通过 -p 与 --output-format json 以子进程方式调用 CLI [24]。

## 9. Anthropic 关于 harness 设计的公开观点

- Building effective agents [26]：文章区分由预定义代码路径编排的 workflow 与由 LLM 自主决定过程的 agent，主张先用最简单的方案，只在明显改善结果时增加复杂度。它还认为 agent-computer interface 的设计值得投入与人机界面同等的精力。
- Effective context engineering for AI agents [27]：文章把上下文视为有限资源，目标是最小的高信号 token 集合。它推荐恰当高度的系统提示、职责不重叠的工具与按需检索，并用压缩、结构化笔记和子代理管理长任务。
- Writing effective tools for agents [28]：文章认为工具是确定性软件与非确定性代理之间的契约，应合并多步操作而非包装每个 API 端点，并返回高信号字段与可操作的错误信息。它建议用真实任务评测工具，并把工具描述当作提示词打磨。
- Effective harnesses for long-running agents [29]：文章用初始化代理搭建环境、进度文件与 git 历史，再由编码代理每次只推进一个特性。特性清单用 JSON 记录，因为模型较少误改 JSON；文章还建议用浏览器自动化做端到端验证。
- Building agents with the Claude Agent SDK [30]：文章称 SDK 的设计原则是 "give your agents a computer"，循环包括收集上下文、采取行动与验证工作。它推荐先用 agentic search，把文件系统当作外部记忆，并以基于规则的反馈验证输出。
- Equipping agents for the real world with Agent Skills [31]：文章把技能定义为包含 SKILL.md 及可选脚本与资源的文件夹，核心是渐进披露。它也提醒，来源不明的技能可能带来数据泄露或越权风险。
- How Anthropic built its multi-agent research system [32]：lead agent 拆解问题并并行启动子代理，子代理在独立上下文中返回压缩结果。文章报告其多代理系统在内部研究评测上比单代理 Opus 4 高 90.2%，但 token 消耗约为聊天的 15 倍，因此只适合高价值且可并行的任务。
- Claude Code sandboxing [33]：文章指出频繁审批会造成审批疲劳，因此用文件系统与网络两层隔离减少提示。网络流量经沙箱外的代理按域名放行，文中称内部使用中沙箱减少了 84% 的权限提示。

## 10. 值得小型 harness 借鉴的设计

1. 把循环显式分为收集上下文、行动、验证三阶段，模型不再发起工具调用即结束回合，并用结果 subtype 记录原因。理由是调用方能区分正常完成、超出回合数与超出预算。
2. 运行中的输入先排队，在当前工具调用结束后注入同一回合，Esc 只取消当前动作。理由是用户可以纠偏，而不必丢弃已有上下文。
3. 权限在执行层按 deny、ask、allow 的固定顺序求值，钩子的 allow 不能绕过 deny。理由是模型可能被提示注入误导，规则必须独立于模型生效。
4. Bash 规则只作便利层，真正的隔离交给 OS 沙箱或容器。理由是前缀规则挡不住等价的程序路径，例如 /usr/bin/curl。
5. 读取、搜索与命令输出都设上限，超长结果落盘并只回传路径与预览。理由是既保护上下文窗口，又不丢失信息。
6. Edit 采用唯一的精确替换，并强制先读后改。理由是匹配失败会被看见，不会静默改错位置。
7. 压缩分两步，先清理旧工具输出再摘要，压缩后按类别重新注入指令、计划与近期文件。理由是摘要必然丢失细节，可重新加载的状态需要单独保障。
8. 检查点只承诺覆盖自己跟踪的文件编辑，并在文档中写明 Bash 副作用不在范围内。理由是边界清楚比承诺过大更可信。
9. 子代理使用独立上下文，只回传最终结果，并设置工具白名单与 maxTurns。理由是控制成本与副作用，超限时还能返回可恢复的部分结果。
10. 技能采用渐进披露：描述常驻，正文按需加载，脚本执行而不进入上下文。理由是长参考资料的常驻成本可以接近零。
11. 会话以追加式 JSONL 保存，支持续接与分支，但不对外承诺格式稳定。理由是兼顾恢复能力与内部演进空间。
12. 无界面模式提供 text、json、stream-json 三种输出，并保留结果的 subtype。理由是脚本能区分成功、超回合与超预算。

## 参考 URL 列表

- [1] https://code.claude.com/docs/en/claude_code_docs_map.md
- [2] https://code.claude.com/docs/llms.txt
- [3] https://code.claude.com/docs/en/how-claude-code-works.md
- [4] https://code.claude.com/docs/en/tools-reference.md
- [5] https://code.claude.com/docs/en/permissions.md
- [6] https://code.claude.com/docs/en/permission-modes.md
- [7] https://code.claude.com/docs/en/sandboxing.md
- [8] https://code.claude.com/docs/en/memory.md
- [9] https://code.claude.com/docs/en/context-window.md
- [10] https://code.claude.com/docs/en/model-config.md
- [11] https://code.claude.com/docs/en/checkpointing.md
- [12] https://code.claude.com/docs/en/common-workflows.md
- [13] https://code.claude.com/docs/en/hooks.md
- [14] https://code.claude.com/docs/en/sub-agents.md
- [15] https://code.claude.com/docs/en/skills.md
- [16] https://code.claude.com/docs/en/commands.md
- [17] https://code.claude.com/docs/en/plugins/components.md
- [18] https://code.claude.com/docs/en/mcp.md
- [19] https://code.claude.com/docs/en/headless.md
- [20] https://code.claude.com/docs/en/sessions.md
- [21] https://code.claude.com/docs/en/settings.md
- [22] https://code.claude.com/docs/en/interactive-mode.md
- [23] https://code.claude.com/docs/en/agent-sdk/agent-loop.md
- [24] https://code.claude.com/docs/en/agent-sdk/overview.md
- [25] https://code.claude.com/docs/en/agent-sdk/permissions.md
- [26] https://www.anthropic.com/engineering/building-effective-agents
- [27] https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- [28] https://www.anthropic.com/engineering/writing-tools-for-agents
- [29] https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- [30] https://claude.com/blog/building-agents-with-the-claude-agent-sdk
- [31] https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills
- [32] https://www.anthropic.com/engineering/multi-agent-research-system
- [33] https://www.anthropic.com/engineering/claude-code-sandboxing
