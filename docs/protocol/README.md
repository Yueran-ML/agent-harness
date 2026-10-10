# Columbina 应用协议

本文规定桌面应用的界面与 Columbina 之间交换的消息。协议在第 10 阶段实现，提前写出来，是为了让界面可以先对着假的服务进程开发。

**状态：** 草案，协议版本 1。事件一节（第 7、8 节）描述的是已经实现并经过测试的数据，形状以 [`events.schema.json`](events.schema.json) 为准，仓库里有测试保证它与代码一致。请求和通知（第 3 到 6 节）在第 10 阶段实现时可能调整细节。

## 1. 三个部分

桌面应用由三个部分组成。

| 部分 | 用什么写 | 做什么 |
|---|---|---|
| 界面 | React、TypeScript，跑在壳的窗口里 | 把会话显示出来，把人的操作变成请求 |
| 壳 | Electron | 开窗口、选文件夹、启动和结束服务进程、在界面和服务进程之间原样转发消息 |
| 服务进程 | Python，命令暂定为 `columbina serve` | 持有对话的全部状态：调模型、执行工具、写会话日志 |

本协议规定的是界面与服务进程之间的消息。壳只转发，不解释。壳的职责见第 11 节。

界面不保存对话，不调模型，不执行工具。服务进程以会话日志为唯一依据。日志是一串带编号的事件，只增不改。界面拿到全部事件，就能还原出任何时刻的画面。

一个服务进程在一个工作目录里运行，同一时刻只打开一个会话。要同时进行几段对话，壳就启动几个服务进程。这样一段对话出了问题不会牵连别的对话，而且一个会话的日志本来就只允许一个进程写。

## 2. 传输

消息走服务进程的标准输入和标准输出，不开网络端口。

- 每条消息是一行 UTF-8 的 JSON，以 `\n` 结束，JSON 内部不含换行。读的一方也要接受 `\r\n`。
- 一行可以有几百 KB，例如一条带着整个工具结果的事件。读的一方不能假设行很短。
- 标准输出上只有协议消息。日志写到标准错误。
- 消息遵循 JSON-RPC 2.0，不使用批量。请求的 `id` 由发出请求的一方选，双方各自编号。
- 请求可以并行。服务进程在每个请求办完时回答，不保证按收到的顺序回答。

**生命周期。** 壳启动服务进程，工作目录就是用户选的项目文件夹。界面先发 `initialize`，之后才能发别的请求。结束时界面发 `shutdown`，服务进程回答之后退出，退出码为 0。标准输入被关闭时，服务进程按收到 `shutdown` 处理：停下正在运行的回合，把收尾写进日志，然后退出。壳崩溃时因此不会留下没人管的服务进程和它启动的命令。

**密钥。** 模型服务的密钥由服务进程从自己的环境变量里读，例如 `DEEPSEEK_API_KEY`。它不出现在任何协议消息里，界面拿不到它。

## 3. 界面发出的请求

| 方法 | 参数 | 结果 |
|---|---|---|
| `initialize` | `{protocol, client}` | `Hello` |
| `session/list` | 无 | `{sessions}` |
| `session/open` | `{session_id?}` | `{session_id, resumed, created_at, events}` |
| `session/events` | `{from, limit?}` | `{events, next}` |
| `session/prompt` | `{text}` | `{}` |
| `session/steer` | `{text}` | `{}` |
| `session/cancel` | 无 | `{}` |
| `session/command` | `{line}` | `{text}` |
| `shutdown` | 无 | `{}` |

### `initialize`

握手。`protocol` 是界面支持的最高协议版本，现在是 1。`client` 是 `{name, version}`，只用于日志。结果里的 `protocol` 是服务进程将要使用的版本，不大于界面给的。

```json
{
  "protocol": 1,
  "server": {"name": "columbina", "version": "0.1.0.dev0"},
  "cwd": "F:\\work\\project",
  "commands": [
    {"name": "compact", "summary": "Sum up the older part of the conversation to make room."},
    {"name": "help", "summary": "List the commands."},
    {"name": "permissions", "summary": "Show what the session may do, or change it to a preset."}
  ]
}
```

`commands` 是可以用 `session/command` 执行的命令，名字不带斜杠。界面用它做命令的补全，不要把命令名写死在代码里。

### `session/list`

列出这个工作目录下存着的会话，最近写过的在前。读不出来的会话不在列表里。

```json
{"sessions": [
  {"id": "20261010T093015-3fa9c2d1", "modified_ms": 1791596000000, "first_input": "check.py fails. Find the mistake in calc.py and fix it, and make sure that `py..."}
]}
```

`first_input` 是用户说的第一句话的第一行，过长时截成 79 个字符加省略号。用户还没有说过话时为 `null`。

### `session/open`

打开一个会话，服务进程从此为它创建 Agent。

- 不给 `session_id`：新建一个会话，id 由服务进程生成。
- `session_id` 是一个存着的会话：接着它继续。结果里 `resumed` 为真。
- `session_id` 没有对应的存着的会话：用这个 id 新建。

会话 id 最多 100 个字符，由字母、数字、点、短横线和下划线组成，首尾必须是字母或数字。

结果里的 `events` 是这一刻日志的长度 N。编号小于 N 的事件用 `session/events` 读。编号不小于 N 的事件一定会作为 `session.event` 通知送到，不重不漏。`created_at` 是会话创建的时刻，自 1970 年起的毫秒数。

已经有会话打开时再次调用，服务进程先结束当前的会话（正在运行的回合被停下），再打开新的。

### `session/events`

分页读已打开的会话的日志。`from` 是第一条要读的事件的编号，`limit` 是最多读几条，缺省和上限都是 500。服务进程可以给得比 `limit` 少。结果里的 `next` 是接着读的编号，读到日志末尾时为 `null`。`events` 里每一项的形状见第 7.1 节。

### `session/prompt`

交给 Agent 一件任务。Agent 空闲时立刻开始一个回合。有回合在运行时，任务排在后面，等前面的回合结束后自己开一个回合。文字首尾的空白会被去掉，去掉之后为空的请求被拒绝。

以斜杠开头的文字在这里不算命令，它会原样交给模型。要执行命令用 `session/command`。

### `session/steer`

中途转向。正在运行的回合在下一步开始时读到这段文字。没有回合在运行时，它自己开一个回合。

终端界面的做法可以照搬：Agent 空闲时，用户提交的一行用 `session/prompt` 发出；回合进行中，用 `session/steer` 发出。

### `session/cancel`

停下正在运行的回合，或者正在两个回合之间进行的工作（例如压缩）。等待中的输入一并丢弃。没有东西在运行时什么也不做。

这个请求立刻返回，它只是提出要求。回合真正停下时，日志里出现 `turn/end`，`reason` 为 `aborted`，随后 `session.status` 变为 `idle`。

### `session/command`

执行一条斜杠命令。`line` 是用户输入的整行，带着斜杠，例如 `/permissions read-only`。结果里的 `text` 是给人看的回答，可以有多行，也可以为空。模型不知道命令的存在。

没有这个命令、命令执行失败，或者命令做到一半被停下，都用 `text` 里的一句话说明，不算协议错误。Agent 不在空闲状态时，命令被拒绝，错误码 1003。

有的命令要花时间，例如 `/compact` 要请模型写摘要。这段时间里 `session.status` 是 `running`，`session/cancel` 可以把它停下。

### `shutdown`

结束服务进程。正在运行的回合被停下，收尾写进日志，会话关闭，然后进程退出。

## 4. 服务进程发出的通知

通知没有 `id`，不需要回答。三种通知的参数里都有 `session_id`。现在一个连接只有一个会话，以后有了子代理，一个连接会带着几个会话的事件。

### `session.event`

日志里新增了一条事件。参数是 `{session_id, event}`。同一个会话的事件按 `seq` 递增，不跳号。

### `session.stream`

模型正在写的回复的一个片段。参数是 `{session_id, turn, step, chunk}`。`chunk` 的形状见第 8 节。

### `session.status`

状态栏需要而日志里读不到的东西。打开会话之后发一次，之后任何一项变化时再发。

```json
{
  "session_id": "20261010T093015-3fa9c2d1",
  "status": "running",
  "detail": null,
  "model": {"provider": "deepseek", "model": "deepseek-flash", "context_window": 1048576},
  "sandbox": {"mode": "workspace-write", "strength": "partial"},
  "approval": "ask"
}
```

| 字段 | 取值 | 含义 |
|---|---|---|
| `status` | `idle` | Agent 空闲：没有回合在运行，也没有回合即将开始 |
| | `running` | 有回合在运行或即将开始，或者两个回合之间的工作正在进行 |
| | `stopped` | Agent 已经停止，不再接受输入，`detail` 说明原因。会话日志写不下去时会这样 |
| `model` | | 接下来的请求使用的模型。`context_window` 没有配置时为 `null` |
| `sandbox.mode` | `read-only`、`workspace-write`、`danger-full-access` | 会话可以改动什么 |
| `sandbox.strength` | `none`、`partial`、`full` | 这台机器能把命令关住到什么程度 |
| `approval` | `ask`、`never` | 需要人点头的调用是来问，还是一律拒绝 |

没有沙箱或审批插件时，`sandbox` 或 `approval` 为 `null`。

## 5. 审批

工具调用需要人点头时，服务进程向界面发一条请求并等待回答。这是协议里唯一由服务进程发出的请求。

方法 `approval/ask`，参数：

```json
{
  "session_id": "20261010T093015-3fa9c2d1",
  "call_id": "call_00_abc",
  "tool": "powershell",
  "summary": "powershell: git push origin main",
  "reason": "No rule allows this command, and nothing holds it here.",
  "target": "git push origin main",
  "standing": "this command, word for word"
}
```

| 字段 | 含义 |
|---|---|
| `summary` | 给人看的一行，过长的内容在里面被截断 |
| `reason` | 为什么要问 |
| `target` | 调用作用的对象的全文，工具没有说明时为 `null` |
| `standing` | 一个更长的许可会覆盖什么，是接在"以后不再询问"后面的话。这个调用每次都必须问时为 `null` |

结果：

```json
{"outcome": "allowed-once", "remember": false}
```

`outcome` 取 `allowed-once` 或 `rejected`。`remember` 为真表示同样的调用在服务进程这次运行期间不再询问。它只在 `outcome` 是 `allowed-once` 并且请求里的 `standing` 不为 `null` 时起作用，可以省略。这个许可只存在服务进程的内存里，进程结束就没有了。

界面必须遵守四条规则。

1. **显示 `target` 的全文。** 只看到命令的开头，没有人能负责地点头。`summary` 只能当标题用。
2. **只有明确的 `allowed-once` 才放行。** 其他任何回答，包括错误的回答和出错，服务进程都当作不同意。
3. **不替人做决定。** 审批没有超时，人不回答，调用就一直等着。
4. **提问会被撤回。** 回合被停下时，等待中的提问作废。界面看到带着同一个 `call_id` 的 `approval/decided` 事件，就关掉对话框。这之后送到的回答会被忽略。

一个会话同一时刻只有一个提问在等回答。界面仍然要按 `call_id` 对应，不要假设只有一个。

日志里每次提问留下两条事件，`approval/asked` 写在提问发出之前，`approval/decided` 写在有了结果之后。审批策略为 `never` 时，这两条事件照样出现，结果是 `rejected`，界面不会收到 `approval/ask`。

## 6. 错误

请求失败时返回 JSON-RPC 的 `error`。`message` 是给人看的一句话。

| `code` | 含义 |
|---|---|
| -32700 | 这一行不是合法的 JSON |
| -32600 | 不是合法的请求 |
| -32601 | 没有这个方法 |
| -32602 | 参数不对 |
| -32603 | 服务进程内部出错 |
| 1000 | 还没有 `initialize`，或者 `initialize` 了两次 |
| 1001 | 没有打开的会话 |
| 1002 | 会话打不开：读不出来，或者正被别的进程打开着 |
| 1003 | Agent 不在空闲状态，这件事要等它空闲 |
| 1004 | Agent 已经停止 |

## 7. 事件

### 7.1 信封

事件的形状与会话文件里的一行相同。

```json
{"type": "tool/result", "seq": 25, "time": 1791534431210, "data": {"message": {"role": "tool", "call_id": "call_00_abc", "content": [{"type": "text", "text": "ok\n[exit code 0]"}], "is_error": false}, "error": null, "meta": null}, "surface_op": "append"}
```

| 字段 | 含义 |
|---|---|
| `type` | 事件类型 |
| `seq` | 在日志里的位置，从 0 起，不跳号 |
| `time` | 写入的时刻，自 1970 年起的毫秒数 |
| `data` | 载荷，形状由 `type` 决定 |
| `ignorable` | 只在为真时出现。不认识这种类型的读者可以跳过它 |
| `surface_op` | 只在事件带着一条模型能看到的消息时出现。`"append"` 表示这条消息接在对话后面。`{"op": "replace", "start_seq": 14, "end_seq": 15}` 表示它顶替了一段旧消息，见第 7.5 节 |
| `source_event_seqs` | 只在顶替时出现：被顶替的事件的编号，按它们在模型看到的对话里的先后排列。`start_seq` 和 `end_seq` 是这个列表的第一项和最后一项 |

### 7.2 类型一览

下表是现有的全部类型。"消息"一栏为"是"的类型带着一条模型能看到的消息，在 `data.message` 里。

| 类型 | 载荷的字段 | 消息 | 含义 |
|---|---|---|---|
| `turn/start` | `turn` | | 一个回合开始 |
| `turn/end` | `turn`、`reason`、`detail` | | 一个回合结束 |
| `step/start` | `turn`、`step` | | 一步开始：一次对模型的请求和它引出的工具调用 |
| `step/end` | `turn`、`step` | | 一步结束 |
| `system/message` | `message` | 是 | 系统提示词 |
| `user/message` | `message`、`source` | 是 | 用户角色的输入，来源见第 7.4 节 |
| `assistant/message` | `message`、`usage`、`finish_reason` | 是 | 模型的一次回复 |
| `assistant/attempt` | `message`、`usage`、`failure` | | 一次失败的请求，`message` 是失败前已经到达的部分 |
| `request/header` | `config`、`tools` | | 之后的请求使用的模型配置和工具声明 |
| `request/aside` | `purpose`、`usage`、`failure` | | 对话之外的一次请求，例如请模型写摘要。只用于记账 |
| `tool/call` | `call_id`、`name`、`arguments` | | 一次工具调用即将执行 |
| `tool/result` | `message`、`error`、`meta` | 是 | 一次工具调用的结果 |
| `agent/inbox/spliced` | `queue`、`removed`、`added` | | 等待中的输入有了变化，见第 9 节 |
| `llm/retry` | `turn`、`step`、`attempt`、`code`、`delay_ms` | | 一次请求失败，等 `delay_ms` 之后重试 |
| `approval/policy` | `policy` | | 审批策略被设定 |
| `approval/asked` | `call_id`、`tool`、`summary` | | 一次提问 |
| `approval/decided` | `call_id`、`outcome` | | 一次提问有了结果 |
| `sandbox/mode` | `mode` | | 沙箱模式被设定 |
| `todo/write` | `call_id`、`items` | | 模型写下了自己的待办清单，整份顶替上一份 |
| `hook/invoked` | `event`、`command` | | 用户的一个钩子即将运行 |
| `hook/result` | `event`、`command`、`status`、`exit_code`、`duration_ms`、`reason` | | 这个钩子运行完了 |
| `compaction/start` | `why`、`tokens` | | 开始压缩对话 |
| `compaction/summary` | `summary`、`replaces`、`region_tokens`、`summary_tokens`、`usage` | | 摘要写好了，带着它的消息紧随其后 |
| `compaction/end` | `why`、`outcome` | | 压缩结束 |
| `session/end-seed` | `inherited` | | 接过来的历史与这次运行的分界 |

各字段的取值范围：

- `turn/end.reason`：`completed`、`aborted`、`blocked`、`error`、`max-tokens`、`interrupted`、`forked`。`detail` 是补充说明，例如 `aborted` 时是谁停的，`error` 时是错误信息。
- `assistant/message.finish_reason`：`stop`、`tool-calls`、`max-tokens`，或者 `null`。`null` 表示回复被打断，`message` 里是已经到达的部分。
- `usage`：`{input_tokens, output_tokens, cache_read_tokens, cache_write_tokens}`。四个数互不重叠，`input_tokens` 只算没有从缓存里读的输入。
- `failure`：`{message, code, status, retry_after_ms, request_id}`。`code` 是稳定的代码，例如 `RATE_LIMIT`。
- `tool/result.error`：`{code, reason}` 或 `null`。`meta` 是模型看不到的附带信息，目前没有工具写它。
- `approval/decided.outcome`：`allowed-once`、`rejected`、`cancelled`、`unavailable`。后两个分别表示提问被撤回和没有人可以问。
- `todo/write.items`：每项是 `{content, status}`，`status` 取 `pending`、`in_progress`、`completed`，同一时刻最多一项是 `in_progress`。`call_id` 是写下清单的那次调用。
- `hook/*.event`：钩子运行的时刻，取 `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`Stop`。`command` 是钩子的命令。
- `hook/result.status`：`completed`、`blocked`、`failed`、`timed-out`。`blocked` 表示钩子拦下了它被问到的那件事，`reason` 是它给的理由，其余情况下 `reason` 为空或者是它要求先问人的理由。`exit_code` 在命令没有运行到结束时为 `null`。`duration_ms` 是这次运行的用时。钩子运行失败或者超时都不拦截任何事。
- `compaction/*.why`：`pressure`（对话接近模型的上限）、`overflow`（模型因为太长拒绝了请求）、`manual`（用户要求）。
- `compaction/end.outcome`：`{trimmed, summarised, before, after, error}`。`trimmed` 是被裁短的工具结果的条数，`summarised` 是被摘要顶替的消息的条数，`before` 和 `after` 是压缩前后对话的 token 估计数，`error` 不为 `null` 时说明没有做完的原因。

### 7.3 消息与内容块

```json
{"role": "assistant", "content": [
  {"type": "reasoning", "text": "The check divides by one too few.", "replay_state": null},
  {"type": "text", "text": "I will read the file."},
  {"type": "tool-call", "call_id": "call_00_abc", "name": "read", "arguments": "{\"path\": \"calc.py\"}"}
]}
```

`role` 取 `system`、`user`、`assistant`、`tool`。前两种和 `tool` 的内容块只有 `text`。`tool` 消息另有 `call_id` 和 `is_error`。

`arguments` 是模型原样写出的字符串，不保证是合法的 JSON。`replay_state` 属于模型的适配器，界面不要读它。

### 7.4 用户消息的来源

`user/message.source` 说明这条用户角色的消息从哪里来。只有前两种是人说的话。

| `source` | 含义 | 建议的显示 |
|---|---|---|
| `user` | 人交给 Agent 的任务 | 正常显示 |
| `steer` | 人在回合中途说的话 | 正常显示，带转向的标记 |
| `context/<名字>` | 程序告诉模型的上下文，例如项目里的说明文件和运行环境 | 不显示，或者折叠起来 |
| `compaction` | 较早对话的摘要 | 显示为一条分隔线，可以展开 |
| `skill` | 人用 `/skill` 交给 Agent 的一个技能的正文 | 折叠起来，标题写技能的名字 |
| `hook/<时刻>` | 用户的钩子放进对话的文字：给一条输入补充的内容，顶替一条被拦下的输入的说明，或者让模型继续的理由 | 折叠起来，注明来自钩子 |
| 其他 | 插件代用户给的输入 | 折叠起来 |

### 7.5 顶替

`surface_op` 为 `replace` 的事件顶替了 `source_event_seqs` 列出的事件。被顶替的事件仍在日志里，只是模型不再看到它们的消息。新的消息站在被顶替的那一段原来的位置上。有三种情况。

- **摘要。** 一条 `source` 为 `compaction` 的 `user/message` 顶替一段相邻的旧消息。
- **裁短。** 一条 `tool/result` 顶替同一个调用的较长的结果，只留下开头和结尾。
- **更新。** 一条 `system/message` 顶替上一条系统提示词，或者一条 `context/<名字>` 的消息顶替同一来源的上一条。

`source_event_seqs` 的顺序是对话里的先后，不是编号的大小。一份摘要写下得晚，编号比它后面保留下来的旧消息大。下一次摘要把它们一起顶替时，这份摘要排在列表的最前面，`start_seq` 因此可以比 `end_seq` 大。样例会话的最后一次压缩就是这样。界面不要假设这个列表是递增的，也不要把 `start_seq` 和 `end_seq` 当作编号的范围去取事件。

界面按下面的规则处理。

1. 顶替别的事件的 `tool/result` 不是一次调用又结束了一遍，不要再显示一次。原来的结果照旧显示，可以注明模型现在只看到它的两端。
2. 摘要显示为一条分隔线。线以上的内容留在页面上，可以注明模型已经看不到它们。
3. 更新不显示。

### 7.6 压缩时的事件顺序

一次压缩在日志里以 `compaction/start` 开始，以 `compaction/end` 结束。两者之间可能有裁短的 `tool/result`，也可能有一次摘要。一次摘要留下三种事件：每次请求模型留下一条 `request/aside`，摘要写成之后是一条 `compaction/summary`，紧接着是带着摘要的 `user/message`。界面不要依赖裁短和摘要的先后。

界面在 `compaction/start` 和 `compaction/end` 之间显示"正在压缩对话"。`compaction/end.outcome.error` 不为 `null` 时把它显示出来。程序在压缩中途退出时日志里没有 `compaction/end`，下次打开时写下的 `session/end-seed` 表示它不会再结束了。

## 8. 流式片段

`session.stream` 里的 `chunk` 有七种，用 `type` 区分。`index` 是内容块在这次回复里的序号。

| `type` 的取值 | 其他字段 | 含义 |
|---|---|---|
| `block-start` | `index`、`kind`、`call_id`、`name` | 一个内容块开始。`kind` 取 `text`、`reasoning`、`tool-call`，后一种带着 `call_id` 和 `name` |
| `text-delta` | `index`、`text` | 文字块新增的文字 |
| `reasoning-delta` | `index`、`text` | 推理块新增的文字 |
| `tool-call-delta` | `index`、`arguments` | 工具调用的参数新增的一段 |
| `block-end` | `index`、`replay_state` | 一个内容块结束 |
| `usage` | `usage` | 到目前为止的用量，后到的取代先到的 |
| `finish` | `reason`、`failure` | 回复结束。`reason` 取 `stop`、`tool-calls`、`max-tokens`、`error`，为 `error` 时带着 `failure` |

片段只是预览。回复写完之后，日志里出现一条 `assistant/message`，界面用它替换预览。请求失败时出现的是 `assistant/attempt`，界面撤掉预览。随后如果有 `llm/retry`，说明服务进程会在 `delay_ms` 之后再试一次，新的片段从头开始。

## 9. 界面从事件算出来的状态

下面这些状态服务进程不另外通知，界面按事件的顺序自己算。断线之后从头再算一遍，结果必须相同。

**等待中的输入。** 有两个队列，`next-turn` 里是各开一个回合的任务，`next-step` 里是下一步要读的输入。每条 `agent/inbox/spliced` 先从它的 `queue` 的队首去掉 `removed` 项，再把 `added` 接到队尾。`added` 的每一项是 `{message, source}`。界面可以把还在队列里的输入显示为"等待中"。

**工具调用。** `tool/call` 与 `tool/result` 按 `call_id` 配对，结果的 `call_id` 在 `data.message.call_id` 里。一次回复可以带几个调用，结果到达的顺序不一定与调用的顺序相同。调用的用时是结果的 `time` 减去调用的 `time`。中间有过审批时，从 `approval/decided` 的 `time` 算起，等人回答的时间不算在内。

**回合。** `turn/start` 之后到同一个 `turn` 的 `turn/end` 之前，回合在进行。`reason` 不是 `completed` 时，界面用一行说明回合是怎么结束的。

**用量。** 上一次请求发给模型的 token 数，是最近一条 `assistant/message` 或 `assistant/attempt` 的 `input_tokens`、`cache_read_tokens`、`cache_write_tokens` 之和。它与 `session.status.model.context_window` 之比是上下文的占用率。缓存命中率是 `cache_read_tokens` 的总和除以上述三项的总和，求和的范围是全部 `assistant/message`、`assistant/attempt` 和 `request/aside`。

**待办清单。** 最近一条 `todo/write` 的 `items` 是当前的清单。没有这种事件，或者最近一条的 `items` 为空，就是没有清单。清单是模型给自己记的事，界面可以把它显示成一块面板。写下清单的工具叫 `todo_write`，它的调用不必再把参数显示一遍。

**工具的声明。** 最近一条 `request/header` 的 `tools` 是模型现在能用的工具，每项是 `{name, description, parameters}`。

内置工具的参数名可以用来把一次调用显示得更清楚。终端界面的做法是：依次取 `command`、`path`、`pattern` 中第一个存在的参数当作调用作用的对象；`edit` 的 `old` 和 `new` 显示成删去和加上的行；`write` 的 `content` 只显示行数；命令的输出显示末尾几行，因为报错和退出码在末尾。

## 10. 界面必须遵守的规则

1. **以事件为准。** 界面的状态是事件的函数。重新打开一个会话，画面应当与当时一样。
2. **跳过不认识的东西。** 不认识的事件类型、字段、通知和片段类型都跳过，不报错。
3. **模型和工具写的文字不可信。** 一律当作纯文本处理，不当作 HTML 插入页面。渲染 Markdown 时使用不执行脚本、不加载远程资源的渲染器。链接不自动打开，打开之前显示完整的地址。
4. **忙碌状态以 `session.status` 为准。** `running` 时界面提供停止的办法，新的输入作为转向发出。
5. **沙箱模式和审批策略始终看得见。** 模式为 `danger-full-access` 或者策略为 `never` 时要醒目。
6. **审批的四条规则**，见第 5 节。

## 11. 壳的职责

壳用 Electron 写。它对服务进程的了解只有三件事：怎么启动，怎么转发一行文字，怎么结束。

- 主进程为每一段对话启动一个服务进程，工作目录是用户选的文件夹，环境变量原样传下去。它按行读标准输出，每一行原样交给那段对话的窗口。窗口交来的每一行原样写进标准输入。标准错误写进应用的日志文件。
- 关闭一段对话时先让界面发 `shutdown`，等几秒，进程还在就结束它。应用退出时对每个服务进程都这样做。
- 界面跑在隔离的环境里：`contextIsolation` 开，`nodeIntegration` 关，`sandbox` 开。预加载脚本只暴露这几个函数：选文件夹、启动服务进程、发送一行、接收一行、结束服务进程、在系统浏览器里打开一个 http 或 https 地址。
- 页面的内容安全策略只允许加载应用自己的文件。页面不能跳转到别的地址，也不能打开新窗口。

这些限制的理由是界面会显示模型和工具写的文字。万一其中有内容被当成代码执行，它能做的事情也只有上面这几件，而审批仍然要过服务进程那一关。

## 12. 一个回合的消息顺序

下面是新会话第一个回合里双方交换的消息，省略了 JSON-RPC 的外壳。事件的顺序取自 [`deepseek-fix`](../../tests/fixtures/recorded/deepseek-fix/session.v1.jsonl) 这份真实的记录，审批的一段是加进去的。

```
界面 → session/prompt {text}
     ← session.event  agent/inbox/spliced   (queue: next-turn, added: 1)
     ← session.status running
     ← session.event  turn/start
     ← session.event  sandbox/mode, approval/policy     (只在第一个回合)
     ← session.event  system/message                    (只在第一个回合)
     ← session.event  user/message   (source: context/instructions)
     ← session.event  user/message   (source: context/runtime)
     ← session.event  agent/inbox/spliced   (queue: next-turn, removed: 1)
     ← session.event  user/message   (source: user)
     ← session.event  step/start
     ← session.event  request/header
     ← session.stream block-start, reasoning-delta ..., block-end, block-start, tool-call-delta ..., block-end, usage, finish
     ← session.event  assistant/message   (finish_reason: tool-calls)
     ← session.event  tool/call
     ← session.event  approval/asked
     ← approval/ask   {call_id, tool, target, ...}      (服务进程的请求)
界面 → {outcome: "allowed-once"}                        (界面的回答)
     ← session.event  approval/decided
     ← session.event  tool/result
     ← session.event  step/end
     ← session.event  step/start
     ← session.stream ...
     ← session.event  assistant/message   (finish_reason: stop)
     ← session.event  step/end
     ← session.event  turn/end   (reason: completed)
     ← session.status idle
```

## 13. 给界面开发用的材料

| 材料 | 位置 | 说明 |
|---|---|---|
| 事件和片段的 JSON Schema | [`events.schema.json`](events.schema.json) | 由代码导出，测试保证它不过时。可以用它生成 TypeScript 类型 |
| 包含全部事件类型的样例会话 | [`samples/tour.jsonl`](samples/tour.jsonl) | 由真实的 Agent 循环跑出来，每种事件至少出现一次。测试保证它不过时 |
| 真实运行的记录 | [`tests/fixtures/recorded/`](../../tests/fixtures/recorded/) | 两段对 DeepSeek 的真实运行留下的会话日志 |

会话文件的第一行是会话头，形状是 `{type: "session", version, id, created_at, cwd, parent_session, delegation_depth}`。`delegation_depth` 是 0 的会话是人参与的对话，大于 0 的是子代理的会话，这时 `parent_session` 是把任务交给它的那个会话。其余每一行就是第 7.1 节的一条事件。

**样例会话里发生了什么。** 模型的回复是事先写好的，命令是假的，其余都是真的，所以事件和它们的顺序与实际运行相同。模型的窗口设成了 3200 个 token，为的是让压缩在几个回合之内发生。

| 回合 | 内容 | 可以看到 |
|---|---|---|
| 1 | 找出并改正一个文件里的错误 | 推理块、一次回复带两个调用、回合中途的转向、一次失败后重试的请求、三次写下的待办清单、改完文件之后运行的一个钩子、一次被同意的审批 |
| 2 | 提交改动被拒绝，回复被用户停下 | 上下文更新、被拒绝的审批和带错误的结果、被打断的回复、`aborted` 的回合 |
| 3 | 程序退出后重新打开会话，两个命令输出很长 | `session/end-seed`、说明上一个回合被停下的上下文、一次只裁短结果的压缩 |
| 4 | 用户要求压缩，然后再问一句 | 一次带摘要的压缩、摘要之后的对话、作为上下文重新告诉模型的待办清单 |
| 之后 | 用户再要求压缩一次 | 第二份摘要顶替了第一份摘要和它后面的旧消息，`source_event_seqs` 不是递增的 |

Schema 描述的是这个版本写出的数据，每个对象都不带多余的字段。界面不要用它来拒绝数据，新版本会增加字段，见第 14 节。

**假的服务进程。** 服务进程实现之前，界面对着一个回放日志的假进程开发。它读一份会话文件，按下面的规则回答：

- `initialize`、`session/list`、`session/open` 给出固定的结果。`session/open` 的 `events` 为 0。
- 收到 `session/prompt` 之后，按顺序把日志里的事件逐条作为 `session.event` 发出，中间留一点间隔。第一条之前发 `session.status running`，最后一条之后发 `session.status idle`。
- 每条 `assistant/message` 之前，按它的内容块合成片段：每块一个 `block-start`，把文字切成小段作为增量，再一个 `block-end`，最后是 `usage` 和 `finish`。
- 每条 `approval/asked` 之后发一个 `approval/ask` 请求，收到回答再继续。

界面里与服务进程通信的代码只依赖一个接口：发送一条消息，接收一条消息，关闭。假进程和 Electron 的转发是这个接口的两个实现。

## 14. 之后会增加的内容

下面这些还没有定义。界面的结构要留出位置，现在不必实现。

- 会话分叉：命令 `/fork` 让对话转到当前会话的一份副本上，副本的会话头里 `parent_session` 是它的来源，日志里接着被复制的事件有一条 `inherited` 为真的 `session/end-seed`。服务进程怎样把"对话换到了另一个会话"告诉界面，还没有定义。
- 子代理、MCP 工具：第 6 阶段。子代理有自己的会话，它的事件带着自己的 `session_id`。
- 图片内容块：尚无计划。

协议的兼容规则只有一条：版本号不变时只增加，不修改已有字段的含义。新的事件类型、新的字段、新的方法、新的取值都算增加，所以界面要遵守第 10 节的第 2 条。
