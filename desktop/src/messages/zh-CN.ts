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
};
