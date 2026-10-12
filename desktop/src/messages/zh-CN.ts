/**
 * What the interface says, in Chinese as it is written on the mainland.
 *
 * The keys are those of the English catalogue, and the compiler holds this
 * one to them. The words are not a translation, sentence for sentence: each
 * says the same thing the way a Chinese product would say it.
 *
 * How the words are written, beside what the English catalogue says of it:
 *
 * - Punctuation is of full width.
 * - A blank stands between Chinese and a number or a Latin word, and never
 *   between two Chinese characters.
 * - The user is 你.
 */
import { splitDuration } from './duration';
import type { Messages } from './en';
import { beforeWord } from './spacing';

function duration(seconds: number): string {
  const time = splitDuration(seconds);
  if (time.hours > 0) return time.minutes > 0 ? `${time.hours} 小时 ${time.minutes} 分` : `${time.hours} 小时`;
  if (time.minutes > 0) return time.seconds > 0 ? `${time.minutes} 分 ${time.seconds} 秒` : `${time.minutes} 分钟`;
  return `${time.seconds} 秒`;
}

// A turn that reached a limit stops between two steps, whichever limit it was.
const afterLimit = '它停在了下一步之前。需要的话，可以让它接着做。';

export const zhCN: Messages = {
  duration,

  tool: {
    running: { what: '正在执行' },
    waitingForApproval: { what: '等你同意' },
    done: { what: '完成' },
    allowedByYou: '你同意了',
    exited: (code: number) => ({ what: `退出码 ${code}` }),
    timedOut: {
      call: { what: '超时', why: '用时超过了上限，已经被停下。' },
      command: (seconds: number) => ({
        what: '超时',
        why: `命令运行了 ${duration(seconds)}还没有结束，已经被停下。`,
      }),
      search: { what: '超时', why: '搜索用时太长，已经被停下。' },
    },
    declined: { what: '你没有同意' },
    refused: {
      rule: { what: '被规则拒绝', why: '有一条权限规则不允许这样做。' },
      hook: { what: '被钩子拦下', why: '一个钩子没有放行这次调用。' },
      policy: { what: '没有得到同意', why: '这一步需要你同意，而这个任务设置为谁也不问。' },
      nobody: { what: '没有得到同意', why: '这一步需要有人同意，而当时没有人可以问。' },
      approval: { what: '没有得到同意', why: '这一步需要有人同意，但没有人同意。' },
      readOnly: { what: '没有写入', why: '这个任务只能读文件，所以什么都没有写。' },
      protected: { what: '没有写入', why: '它在一个不让改动的文件夹里，比如 .git。' },
      outside: { what: '在项目之外', why: '按这个任务的权限，只能读写项目文件夹里的文件。' },
      notWritable: { what: '没有写入', why: '这个任务没有写文件的权限。' },
      rulesUnreadable: { what: '没有执行', why: '这个项目的权限规则读不出来。修好之前，什么都不会执行。' },
      rulesNotRead: { what: '没有执行', why: '权限规则当时还没有读完。' },
    },
    stopped: {
      turnStopped: { what: '已停下', why: '它执行到一半被停下，可能已经生效了一部分。' },
      programEnded: { what: '中途中断', why: '程序退出时它正在执行。有没有生效，现在不清楚。' },
    },
    notRun: {
      turnStopped: { what: '没有执行', why: '回合被停下时，这一步还没有开始。' },
      programEnded: { what: '没有执行', why: '程序退出时，这一步还没有开始。' },
      questionWithdrawn: { what: '没有执行', why: '问过你要不要同意，但还没等到回答，询问就被撤回了。' },
    },
    unknownTool: { what: '没有这个工具', why: '模型想用的工具不存在。' },
    wrongArguments: { what: '参数不对', why: '模型给这个工具的参数不合它的要求。' },
    failed: {
      error: { what: '失败' },
      checkFailed: {
        what: '没能检查',
        why: '有一项检查出了问题。检查不了的调用不会执行，检查不了的结果也不会交出去。',
      },
      sandboxUnavailable: { what: '没有执行', why: '隔离命令用的沙箱不可用，所以不运行任何命令。' },
      commandTooLong: { what: '命令太长', why: '命令超出了命令行能接受的长度。' },
      projectMoved: {
        what: '项目文件夹变了',
        why: '项目文件夹现在指向的地方，和这个任务开始时不一样了。请重新开始这个任务。',
      },
      notFound: { what: '找不到', why: '没有这个文件或文件夹。' },
      notAFile: { what: '不是文件', why: '这个路径指向的是文件夹，或者别的不是文件的东西。' },
      notADirectory: { what: '不是文件夹', why: '这里需要一个文件夹，路径指向的却是文件。' },
      notText: { what: '不是文本文件' },
      tooLarge: { what: '文件太大', why: '文件超过了一次编辑能处理的大小。' },
      notRead: { what: '还没读过', why: '文件已经存在，却没有先读过，所以没有覆盖它。' },
      changedSinceRead: { what: '读过之后变了', why: '文件在读过之后又有改动，所以没有覆盖它。' },
      noMatch: { what: '没找到原文', why: '文件里没有要替换的那段文字。' },
      ambiguous: { what: '原文不止一处', why: '要替换的文字在文件里出现了不止一次，无法确定改哪一处。' },
      noChange: { what: '没有可改的', why: '新内容和旧内容一样。' },
      invalidPath: { what: '路径不可用', why: '这种写法的路径在这里不能用。' },
      invalidPattern: { what: '搜索条件不可用', why: '搜索条件照现在的写法没法使用。' },
      moved: { what: '位置变了', why: '文件已经不在刚才的位置，所以什么都没有做。' },
      searchFailed: { what: '搜索失败' },
    },
    withheld: { what: '结果被扣下', why: '它已经执行，但一个钩子没有把结果交给模型。' },
    noResult: { what: '没有结果', why: '回合结束时，还没有记下这一步的结果。' },
  },

  model: {
    noKey: { what: '还没有配置模型服务的 API 密钥。' },
    noKeyFor: (service: string) => ({ what: `还没有配置 ${service} 的 API 密钥。` }),
    keyRefused: {
      what: '模型服务没有接受这个 API 密钥。',
      why: '密钥可能填错了或者已经失效，也可能没有使用这个模型的权限。',
    },
    keyProblem: {
      what: '现在的 API 密钥用不了模型服务。',
      why: '密钥还没有配置，或者没有被服务接受。',
    },
    quota: { what: '模型服务的账户余额不足。' },
    rateLimit: { what: '模型服务正忙。', why: '它眼下收到的请求太多。' },
    contextTooLong: { what: '对话太长，超出了模型能处理的长度。' },
    contentFilter: { what: '模型服务扣下了这条回复。', why: '回复没有通过它的内容审核。' },
    invalidRequest: {
      what: '模型服务没有接受这次请求。',
      why: '模型的名字或者服务的地址可能填错了。',
    },
    server: { what: '模型服务出错了。', why: '问题出在服务那一边。' },
    timeout: { what: '模型服务迟迟没有回应。' },
    unreachable: { what: '连不上模型服务。', why: '可能是网络不通，也可能是服务的地址不对。' },
    cutOff: { what: '模型写到一半，连接断了。' },
    malformed: { what: '模型服务发回的内容读不出来。' },
    empty: { what: '模型发回了一条空的回复。' },
    internal: { what: 'Columbina 在接收回复时出了问题。' },
    unknown: { what: '向模型提问时出了问题。' },
  },

  turn: {
    took: (seconds: number, steps: number) => `用了 ${duration(seconds)}，${steps} 步`,
    stopped: {
      you: { what: '你停下了这个回合。' },
      parent: { what: '交出这件任务的 Agent 把它停下了。' },
      hook: { what: '一个钩子停下了这个回合。' },
      budget: { what: '这个回合到了给它设的上限，被停下了。' },
      closing: { what: '任务被关闭，这个回合也随之停下。' },
      unknown: { what: '这个回合被停下了。' },
    },
    limit: {
      steps: (limit: number) => ({ what: `这个回合用完了 ${limit} 步的上限。`, why: afterLimit }),
      tokens: { what: '这个回合用完了 token 的上限。', why: afterLimit },
      time: { what: '这个回合用完了时间上限。', why: afterLimit },
    },
    inputStopped: { what: '一个钩子拦下了你说的这句话。', why: '它没有被发给模型。' },
    blocked: { what: '这个回合没有被允许继续。' },
    crashed: { what: 'Columbina 内部出了问题，这个回合没能继续。' },
    outputLimit: { what: '有一条回复没有写完。', why: '模型一次能写的长度到了上限。' },
    interrupted: { what: '程序退出时，这个回合被中断了。' },
    forked: { what: '这个回合在任务被复制的地方截断了。' },
    unknown: { what: '这个回合结束了。' },
  },

  wait: {
    retry: (seconds: number, retry: number) => (seconds > 0
      ? `${duration(seconds)}后第 ${retry} 次重试。`
      : `正在第 ${retry} 次重试。`),
    retried: (seconds: number, retry: number) => (seconds > 0
      ? `等了 ${duration(seconds)}，第 ${retry} 次重试。`
      : `随即第 ${retry} 次重试。`),
    gaveUp: '还没来得及重试就停下了。',
  },

  notice: {
    compactionFailed: { what: '较早的对话没能整理成摘要。' },
    compactionInterrupted: { what: '程序退出时，整理较早对话的工作被中断了。' },
  },

  permissions: {
    readOnlyAsks: '可以读这个项目里的文件，运行命令之前会问。',
    readOnlyStrict: '可以读这个项目里的文件，只运行规则允许的命令。',
    writeAsks: '可以修改这个项目里的文件，运行命令之前会问。',
    writeStrict: '可以修改这个项目里的文件，只运行规则允许的命令。',
    fullAccess: '可以读写这台电脑上的任何文件，运行命令也不再询问。',
    unknown: '这个任务能做什么，现在不清楚。',
  },

  confinement: {
    none: '这台电脑关不住命令。命令一旦运行，你能做的事它都能做。',
    partial: '这台电脑只能部分关住命令。命令仍然可能越出这些权限。',
    full: '命令被关在这些权限之内。',
  },

  actions: {
    retry: '重试',
    openModelSettings: '打开模型服务设置',
    openPermissions: '打开权限设置',
    compact: '整理较早的对话',
  },

  prompt: {
    goOn: '从刚才中断的地方接着做。',
  },

  label: {
    newTask: '新任务',
    searchTasks: '搜索任务',
    tasks: '任务',
    openProject: '打开项目',
    settings: '设置',
    hideTasks: '收起任务列表',
    showTasks: '展开任务列表',
    changes: '改动',
    details: '详情',
    close: '关闭',
    cancel: '取消',
    done: '完成',
    apply: '应用',
    copy: '复制',
    copied: '已复制',
    send: '发送',
    stop: '停止',
    stopping: '正在停止',
    commands: '命令',
    restart: '重新启动',
    latest: '回到最新',
    replay: '回放',
    showLog: '查看日志',
    lastWords: '它最后写下的内容',
    recentProjects: '最近的项目',
    notices: '通知',
    tellMe: 'Columbina 等我或者做完时通知我',

    key: {
      title: '模型服务',
      service: '服务',
      ownService: '自己添加的服务',
      name: '名字',
      address: '地址',
      model: '模型',
      contextWindow: '上下文窗口',
      tokens: '个 token',
      apiKey: 'API 密钥',
      store: '保存',
      storing: '正在保存',
      setUp: '设置服务或密钥',
      inUse: '使用中',
      from: {
        environment: '密钥来自环境变量',
        stored: '已保存密钥',
        storedElsewhere: '密钥属于另一个地址',
        none: '没有密钥',
        notNeeded: '不需要密钥',
      },
    },

    notice: {
      consent: 'Columbina 需要你同意',
      finished: 'Columbina 做完了',
      stopped: 'Columbina 停下了',
    },
    mute: '关闭声音',
    unmute: '打开声音',
    soundOn: '声音开',
    soundOff: '声音关',

    opening: {
      name: '开场',
      loading: '正在载入开场',
      play: '播放开场',
      again: '再放一遍',
      skip: '跳过',
      anyKey: '按任意键或点击即可跳过',
      begin: '按任意键进入',
    },

    music: {
      none: '还没有选音乐',
      choose: '选择文件',
      change: '换一个文件',
      play: '播放',
      useKept: '使用应用文件夹里的音乐',
      remove: '移除',
      stands: {
        loading: '正在载入',
        none: '还没有选音乐',
        unavailable: '无法播放',
        blocked: '等你按下播放',
        muted: '已静音',
        playing: '播放中',
      },
    },
    otherProject: '打开别的项目',
    skipToInput: '跳到输入框',
    confirmLeave: '停下正在进行的回合？',
    stay: '留在这里',
    stopAndLeave: '停下并离开',

    ago: {
      now: '刚刚',
      minutes: (count: number) => `${count} 分钟前`,
      hours: (count: number) => `${count} 小时前`,
      days: (count: number) => `${count} 天前`,
    },

    status: {
      ready: '空闲',
      working: '工作中',
      stopping: '正在停止',
      stopped: '已停止',
      offline: '未连接',
      starting: '正在启动',
      opening: '正在打开',
    },
    context: (percent: number) => `上下文 ${percent}%`,
    contextWindow: '上下文',
    usage: '这个任务用掉的 token',
    usageIn: '读入',
    usageOut: '写出',
    usageCached: '命中缓存',
    tools: '工具',
    taskId: '任务',

    verb: {
      read: '读取',
      write: '写入',
      edit: '修改',
      list: '列出',
      search: '搜索',
      run: '运行',
      todo: '记下待办',
      skill: '使用技能',
      subagent: '交给子代理',
      other: (tool: string) => `调用 ${tool}`,
    },
    input: '给它的内容',
    output: '返回的内容',
    noOutput: '没有返回内容',
    before: '改之前',
    after: '改之后',
    thought: '想了一下',
    thinking: '正在思考',
    writing: '正在写',
    preparing: '正在准备调用',
    saidOnTheWay: '中途补充',
    waiting: '排队中',
    summary: '较早对话的摘要',

    placeholderIdle: '交给 Columbina 一件事',
    placeholderRunning: '补充一句，它下一步会读到',
    placeholderNoProject: '先打开一个项目',
    placeholderStarting: 'Columbina 正在启动',
    placeholderOpening: '正在打开任务',
    placeholderStopped: '这个任务已经停止',

    consentTitle: '需要你同意',
    allowOnce: '同意这一次',
    allowAlways: '同意，以后不再问',
    reject: '不同意',
    stopTurn: '停下回合',
    whyItAsks: '为什么要问',

    emptyTitle: '今天想做点什么？',
    emptyNoProjectTitle: '从打开一个项目开始',
    recentTasks: '最近的任务',

    appearance: '外观',
    theme: '主题',
    themeNames: { moonveil: '月夜', moonwhite: '月白' },
    language: '语言',
    languageSystem: '跟随系统',
    model: '模型',
    permissions: '权限',
    presets: {
      readOnly: '只读',
      workspaceWrite: '可以修改项目里的文件',
      fullAccess: '完全访问',
    },
    openingAndMusic: '开场与音乐',
    playOpening: '启动时播放开场',
    playOpeningNow: '现在播放开场',
    connection: '连接',

    confirmFullAccess: '允许完全访问？',
    confirmWrite: '允许 Columbina 修改文件？',
    allowFullAccess: '允许完全访问',
    allowWrite: '允许修改',
    openLinkTitle: '打开这个链接？',
    openLink: '打开',
  },

  hint: {
    keysIdle: '回车发送，Shift+回车换行，输入斜杠打开命令。',
    keysRunning: '现在发送的话会在下一步被读到。连按两次 Esc 停下回合。',
    escAgain: '再按一次 Esc 停下回合。',
    returned: '你发出的话还没有被读到，已经放回输入框。',
    confirmLeave: 'Columbina 还在处理这个任务，离开会停下这个回合。',
    requestFailed: '这一步没有做成。',
    commandsWhenIdle: '命令要等 Columbina 空闲时才能执行。',
    starting: (folder: string) => `正在 ${folder} 里启动 Columbina。`,
    stillStarting: '还在启动。',
    openingTask: '正在打开任务。',
    reading: (read: number, of: number) => `已读取 ${read} / ${of} 条。`,
    compacting: '正在整理较早的对话。',
    noTasks: '这个项目里还没有任务。',
    noMatch: '没有匹配的任务。',
    emptyNoProject: '先选一个文件夹，Columbina 就在里面工作。',
    replay: '这是一段录好的会话的回放，不会请求任何模型。',
    taskStopped: '这个任务已经停止，不再接受输入。重新启动 Columbina 之后可以接着做。',
    serviceEnded: '服务进程已经结束。',
    serviceNotStarted: '服务进程没能启动。',
    tellMe: '只在这个窗口不在前台时通知。',

    consentRun: 'Columbina 想运行这条命令。',
    consentWrite: 'Columbina 想修改这个文件。',
    consentOther: (tool: string) => `Columbina 想使用 ${tool}。`,
    consentWaiting: 'Columbina 在等你的回答。',
    consentSent: '你的回答已经发出。',
    afterNo: '这一步没有执行。可以告诉 Columbina 换个做法。',
    scope: {
      thisCommand: '“以后不再问”只对这个任务里的这条命令有效，一字不差，到项目关闭为止。',
      anyCall: (tool: string) => `“以后不再问”对这个任务里 ${tool} 的每一次调用有效，到项目关闭为止。`,
      onTarget: (tool: string, target: string) =>
        `“以后不再问”对这个任务里 ${tool} 作用于 ${target} 的调用有效，到项目关闭为止。`,
      other: '“以后不再问”对这个任务里同样的调用有效，到项目关闭为止。',
    },

    settingsWhenIdle: '模型和权限要等 Columbina 空闲时才能改。',
    confirmFullAccess: 'Columbina 将不经询问就运行命令，也没有任何东西约束它们。',
    confirmWrite: 'Columbina 将可以修改这个项目里的文件。运行命令之前它仍然会问。',
    openLink: '它出现在对话里，会在你的浏览器里打开。',
    commandsList: '命令由 Columbina 自己执行，模型看不到它。',
    connectedService: '已连接到在这个文件夹里运行的服务进程。',
    contextOf: (used: string, window: string) => `最近一次请求用了 ${beforeWord(used)}个 token，上限是 ${window}。`,
    contextUnknown: (used: string) => `最近一次请求用了 ${beforeWord(used)}个 token。`,
    noToolsYet: '发出第一次请求之后才知道有哪些工具。',
    noChanges: '这个任务没有改动任何文件。',
    about: (version: string, protocol: number) => `Columbina ${version}，协议版本 ${protocol}。`,
    openingUnavailable: '开场无法播放。',

    key: {
      lead: '选择模型来自哪个服务，并填写它的密钥。',
      reading: '正在读取已有的设置。',
      unavailable: '应用运行真实的服务进程时才能在这里设置服务，回放时不能。',
      ownName: '自己起一个名字：小写字母、数字、- 和 _。',
      ownAddress: '服务所在的地址，例如 https://api.example.com/v1。',
      modelName: '模型的名字，以服务那边的写法为准。',
      windowSize: '模型能接收多少 token，可以留空。',
      notNeeded: '这个服务不需要密钥。',
      fromVariable: (variable: string) => `这个服务的密钥来自环境变量 ${variable}，它优先于保存的密钥。`,
      stored: '这个服务已经保存了密钥。留空就继续用它。',
      storedElsewhere: '这个服务保存过密钥，但那是给另一个地址的，不会发往现在的地址。',
      none: '这个服务还没有密钥。',
      whereItGoes: '密钥从这个窗口直接交给 Columbina，由它存进系统密钥环。显示对话的那个窗口始终拿不到它。',
      done: (service: string, model: string) => `Columbina 现在使用 ${service}，模型是 ${model}。`,
      inKeyring: '密钥已经存进系统密钥环。',
      inFile: '没有密钥环接收这个密钥。它原样存在 Columbina 文件夹里的一个文件中，没有加密。',
      restart: '重新启动 Columbina 之后，新保存的内容才会生效。',
    },

    music: {
      about: '音乐会循环播放，开场时和工作时都在。',
      kinds: '支持 MP3、WAV、M4A、OGG 和 FLAC，最大 100 MB。文件只保存在这台电脑上。',
      problem: {
        cannotPlay: '这个文件无法播放，请换一个。',
        cannotOpenKept: '应用文件夹里的音乐打不开，请换一个文件。',
        wrongKind: '请选择 MP3、WAV、M4A、OGG 或 FLAC 文件。',
        wrongSize: '请选择不为空、不超过 100 MB 的文件。',
        noKept: '应用文件夹里没有音乐，请选择一个文件。',
      },
      unkept: {
        cannotRead: '读不到之前的选择。现在的选择只对这个窗口有效。',
        thisWindowOnly: '这个文件只在这个窗口里播放，没能保存到下次启动。',
        notCleared: '音乐已经从这个窗口移除。保存的那一份没能清掉，下次启动时可能还会出现。',
        choiceNotKept: '应用文件夹里的音乐会在这个窗口里播放。这个选择没能保存下来。',
      },
    },
  },
};
