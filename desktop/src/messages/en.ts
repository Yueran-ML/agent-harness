/**
 * What the interface says, in English.
 *
 * The type of this catalogue is the contract: every other language has the
 * same keys, and the compiler checks that.
 *
 * How the words are written:
 *
 * - Nothing here is a value of the protocol, a code or the name of an event.
 *   Those are for a view of the details.
 * - `what` says what happened. `why` says the reason where one is known,
 *   and is left out where none is.
 * - For a call of a tool, `what` is a few words for the row of the call and
 *   has no full stop. Everywhere else it is a sentence.
 * - Short, calm and concrete, without exclamation marks.
 * - A sentence that takes a number is a function.
 */
import { fairlightEn } from './fairlight';
import { splitDuration } from './duration';

function duration(seconds: number): string {
  const time = splitDuration(seconds);
  if (time.hours > 0) return time.minutes > 0 ? `${time.hours} h ${time.minutes} min` : `${time.hours} h`;
  if (time.minutes > 0) return time.seconds > 0 ? `${time.minutes} min ${time.seconds} s` : `${time.minutes} min`;
  return `${time.seconds} s`;
}

// A turn that reached a limit stops between two steps, whichever limit it was.
const afterLimit = 'It stopped before its next step. Tell it to go on if it should.';

type Weekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun';
// The days of the week, for a task that runs on one of them.
const weekdays: Record<Weekday, string> = { mon: 'Monday', tue: 'Tuesday', wed: 'Wednesday', thu: 'Thursday', fri: 'Friday', sat: 'Saturday', sun: 'Sunday' };

export const en = {
  /** A length of time, as short as a line of status needs it. */
  duration,

  /** How a call of a tool stands or ended. */
  tool: {
    running: { what: 'Running' },
    waitingForApproval: { what: 'Waiting for you to allow it' },
    done: { what: 'Done' },
    /** Said beside the outcome of a call that the user was asked about and allowed. */
    allowedByYou: 'You allowed this',
    exited: (code: number) => ({ what: `Exit code ${code}` }),
    timedOut: {
      call: { what: 'Timed out', why: 'It took longer than it may, and was stopped.' },
      command: (seconds: number) => ({
        what: 'Timed out',
        why: `The command was stopped after ${duration(seconds)}.`,
      }),
      search: { what: 'Timed out', why: 'The search took too long, and was stopped.' },
    },
    declined: { what: 'You did not allow this' },
    refused: {
      rule: { what: 'Refused by a rule', why: 'A rule of the permissions is against it.' },
      hook: { what: 'Stopped by a hook', why: 'A hook did not let this call through.' },
      policy: {
        what: 'Not allowed',
        why: 'This would have needed your consent, and this task is set to ask nobody.',
      },
      nobody: { what: 'Not allowed', why: 'This would have needed consent, and nobody could be asked.' },
      approval: { what: 'Not allowed', why: 'This would have needed consent, and it was not given.' },
      readOnly: { what: 'Not written', why: 'This task may only read files, so nothing was written.' },
      protected: { what: 'Not written', why: 'It lies in a folder that is kept from changes, such as .git.' },
      outside: {
        what: 'Outside the project',
        why: 'With the permissions of this task, only files in the project folder are read or written.',
      },
      notWritable: { what: 'Not written', why: 'Nothing in this task allows writing files.' },
      rulesUnreadable: {
        what: 'Not run',
        why: 'The permission rules of this project cannot be read. Nothing is run until they are put right.',
      },
      rulesNotRead: { what: 'Not run', why: 'The permission rules had not been read yet.' },
    },
    stopped: {
      turnStopped: { what: 'Stopped', why: 'It was stopped while it ran, and may have taken effect in part.' },
      programEnded: {
        what: 'Cut short',
        why: 'The program ended while this ran. Whether it took effect is not known.',
      },
    },
    notRun: {
      turnStopped: { what: 'Not run', why: 'The turn was stopped before this began.' },
      programEnded: { what: 'Not run', why: 'The program ended before this began.' },
      questionWithdrawn: {
        what: 'Not run',
        why: 'You were asked about it, and the question was withdrawn before it was answered.',
      },
    },
    unknownTool: { what: 'No such tool', why: 'The model asked for a tool that is not there.' },
    wrongArguments: { what: 'Arguments do not fit', why: 'The model gave this tool arguments that it cannot take.' },
    failed: {
      error: { what: 'Failed' },
      checkFailed: {
        what: 'Could not be checked',
        why: 'A check failed. A call that cannot be checked is not run, '
          + 'and a result that cannot be checked is not passed on.',
      },
      sandboxUnavailable: {
        what: 'Not run',
        why: 'What is set up to confine commands is not there, so no command is run.',
      },
      commandTooLong: { what: 'Command too long', why: 'The command is longer than the shell takes.' },
      projectMoved: {
        what: 'Project folder moved',
        why: 'The project folder leads somewhere else than when this task began. Start the task again.',
      },
      notFound: { what: 'Not found', why: 'There is no such file or folder.' },
      notAFile: { what: 'Not a file', why: 'The path leads to a folder, or to something else that is no file.' },
      notADirectory: { what: 'Not a folder', why: 'A folder was needed, and the path leads to a file.' },
      notText: { what: 'Not a text file' },
      tooLarge: { what: 'File too large', why: 'The file is larger than an edit takes.' },
      notRead: {
        what: 'Not read first',
        why: 'The file is there already and was not read first, so it was not written over.',
      },
      changedSinceRead: {
        what: 'Changed since it was read',
        why: 'The file changed after it was read, so it was not written over.',
      },
      noMatch: { what: 'Text not found', why: 'The text to replace does not occur in the file.' },
      ambiguous: {
        what: 'Text found more than once',
        why: 'The text to replace occurs several times, so it is not clear which one is meant.',
      },
      noChange: { what: 'Nothing to change', why: 'The new text is the same as the old.' },
      invalidPath: { what: 'Path not usable', why: 'The path is written in a way that is not used here.' },
      invalidPattern: {
        what: 'Pattern not usable',
        why: 'The pattern of the search cannot be used as it is written.',
      },
      moved: {
        what: 'Moved meanwhile',
        why: 'The file was not where it had been a moment before, so nothing was done.',
      },
      searchFailed: { what: 'Search failed' },
    },
    withheld: { what: 'Result withheld', why: 'It ran, and a hook kept its result from the model.' },
    noResult: { what: 'No result', why: 'The turn ended before a result was written down.' },
  },

  /** Why a request to the model failed. */
  model: {
    noKey: { what: 'No API key is set up for the model service.' },
    keyRefused: {
      what: 'The model service did not accept the API key.',
      why: 'The key may be mistyped or no longer valid, or it may not be allowed to use this model.',
    },
    keyProblem: {
      what: 'The model service cannot be used with the API key as it is.',
      why: 'No key is set up, or the service did not accept it.',
    },
    /** What is said of a key, where the window knows which service the request went to. */
    named: {
      noKey: (service: string) => ({ what: `No API key is set up for ${service}.` }),
      keyRefused: (service: string) => ({
        what: `${service} did not accept the API key.`,
        why: 'The key may be mistyped or no longer valid, or it may not be allowed to use this model.',
      }),
      keyProblem: (service: string) => ({
        what: `${service} cannot be used with the API key as it is.`,
        why: 'No key is set up, or the service did not accept it.',
      }),
    },
    quota: { what: 'The account at the model service has no credit left.' },
    rateLimit: {
      what: 'The model service is busy.',
      why: 'It is being sent more requests than it takes at the moment.',
    },
    contextTooLong: { what: 'The conversation has grown too long for the model.' },
    contentFilter: { what: 'The model service withheld the reply.', why: 'Its content filter stopped the reply.' },
    invalidRequest: {
      what: 'The model service did not take the request.',
      why: 'The name of the model or the address of the service may be wrong.',
    },
    thinkingRefused: {
      what: 'The model service refused what the model thought earlier in this conversation.',
      why: 'The beginning of the conversation changed after the model thought, and the service takes '
        + 'its thinking only with the beginning that it was made with. Begin a new conversation to go on.',
    },
    server: { what: 'The model service ran into an error.', why: 'The fault is on its side.' },
    timeout: { what: 'The model service took too long to answer.' },
    unreachable: {
      what: 'The model service could not be reached.',
      why: 'The network may be down, or the address of the service may be wrong.',
    },
    cutOff: { what: 'The connection broke off while the model was writing.' },
    malformed: { what: 'The model service sent a reply that could not be read.' },
    empty: { what: 'The model sent back an empty reply.' },
    internal: { what: 'Something went wrong in Columbina while the reply came in.' },
    unknown: { what: 'The request to the model failed.' },
  },

  /** How a turn ended. */
  turn: {
    /** The line for a turn that came to its end: how long it took, and in how many steps. */
    took: (seconds: number, steps: number) =>
      `Took ${duration(seconds)}, ${steps} ${steps === 1 ? 'step' : 'steps'}`,
    stopped: {
      you: { what: 'You stopped this turn.' },
      parent: { what: 'The agent that handed over this task stopped it.' },
      hook: { what: 'A hook stopped this turn.' },
      budget: { what: 'This turn was stopped at a limit that is set for it.' },
      closing: { what: 'This turn was stopped when its task was closed.' },
      unknown: { what: 'This turn was stopped.' },
    },
    limit: {
      // The limit is null where the log says which limit it was and not how much.
      steps: (limit: number | null) => ({
        what: limit === null
          ? 'This turn reached its limit of steps.'
          : `This turn reached its limit of ${limit} ${limit === 1 ? 'step' : 'steps'}.`,
        why: afterLimit,
      }),
      tokens: { what: 'This turn reached its limit of tokens.', why: afterLimit },
      time: { what: 'This turn reached its limit of time.', why: afterLimit },
    },
    inputStopped: { what: 'A hook stopped what you said.', why: 'It was not sent to the model.' },
    blocked: { what: 'This turn was not allowed to go on.' },
    crashed: { what: 'Something went wrong in Columbina, and the turn could not go on.' },
    outputLimit: {
      what: 'A reply was cut off.',
      why: 'The model had written as much as it may in one reply.',
    },
    interrupted: { what: 'This turn was cut short when the program ended.' },
    forked: { what: 'This turn was cut short where the task was copied.' },
    unknown: { what: 'This turn has ended.' },
  },

  /** What is waited for. */
  wait: {
    /** `retry` counts the attempts that follow the first: 1 for the second request. */
    retry: (seconds: number, retry: number) => (seconds > 0
      ? `Trying again in ${duration(seconds)} (retry ${retry}).`
      : `Trying again now (retry ${retry}).`),
    /** The same, told afterwards. */
    retried: (seconds: number, retry: number) => (seconds > 0
      ? `Tried again after ${duration(seconds)} (retry ${retry}).`
      : `Tried again at once (retry ${retry}).`),
    /** Of a request that was to be tried again, and never was. */
    gaveUp: 'It was stopped before it tried again.',
  },

  /** What else a conversation tells of. */
  notice: {
    compactionFailed: { what: 'The earlier part of the conversation could not be summed up.' },
    compactionInterrupted: {
      what: 'Summing up the earlier part of the conversation was cut short when the program ended.',
    },
  },

  /** What a task may do, in a sentence. */
  permissions: {
    readOnlyAsks: 'Can read the files of this project, and asks before it runs a command.',
    readOnlyStrict: 'Can read the files of this project, and runs only commands that a rule allows.',
    writeAsks: 'Can change the files of this project, and asks before it runs a command.',
    writeStrict: 'Can change the files of this project, and runs only commands that a rule allows.',
    fullAccess: 'Can read and change files anywhere on this computer, and runs commands without asking.',
    unknown: 'What this task may do is not known.',
    // A chat has no tool for either, whatever its preset says.
    chat: 'Changes no file and runs no command.',
  },

  /** How well this computer holds a command to what the task may do. */
  confinement: {
    none: 'Commands are not confined on this computer. A command that runs can do whatever you can do.',
    partial: 'Commands are confined only in part here. A command may reach further than these permissions allow.',
    full: 'Commands are confined to what these permissions allow.',
  },

  /** What can be done next. Each is the face of a button. */
  actions: {
    retry: 'Try again',
    openModelSettings: 'Open model settings',
    openPermissions: 'Open permissions',
    compact: 'Sum up earlier messages',
  },

  /** What the interface says to the agent in the place of the user, when a button asks for it. */
  prompt: {
    goOn: 'Go on from where the turn broke off.',
  },

  /**
   * The faces of controls, headings, and other short lines that are no
   * sentences. None ends with a full stop.
   */
  label: {
    /**
     * The tools of a task beside its conversation: what it cost, planning, export, what the user
     * says of a turn, going back, tasks at set times, worktrees and the groups of the assistant in QQ.
     */
    work: {
      spent: (amount: string) => `~${amount}`,
      plan: 'Plan',
      planning: 'Plan mode',
      exportTask: 'Export',
      right: 'It did what was asked',
      wrong: 'It did not do what was asked',
      rewind: 'Put the files back',
      rewindYes: 'Put them back',
      keep: 'Keep them',
      schedules: {
        title: 'Tasks at set times',
        name: 'Name',
        folder: 'Folder',
        chooseFolder: 'Choose',
        prompt: 'What the agent is asked',
        every: 'How often',
        kinds: { minutes: 'Every few minutes', hour: 'Every hour', day: 'Every day', week: 'Every week' },
        minutes: 'Minutes, 5 at least',
        at: 'At',
        minute: 'Minute',
        days: weekdays,
        weekday: 'Day',
        missed: 'A time that was missed while Columbina was closed',
        missedKinds: { run: 'Runs once when it opens', skip: 'Is skipped' },
        add: 'Add the task',
        remove: 'Remove',
        on: 'Runs',
        when: {
          minutes: (count: number) => `every ${count} minutes`,
          hour: (minute: string) => `every hour at minute ${minute}`,
          day: (time: string) => `every day at ${time}`,
          week: (day: Weekday, time: string) => `every ${weekdays[day]} at ${time}`,
        },
        last: (when: string, end: string) => `Last run ${when}: ${end}`,
      },
      worktrees: {
        title: 'Worktrees',
        new: 'Task in a worktree',
        name: 'Name of the task',
        merge: 'Merge',
        remove: 'Remove',
      },
      groups: {
        title: 'Groups',
        unnamed: 'No name',
        name: 'Name of the group',
        save: 'Keep the name',
        memory: (used: number, most: number) => `Memory: ${used} of ${most} characters`,
        show: 'Show the memory',
        hide: 'Hide the memory',
        forget: 'Forget the memory',
      },
    },

    newTask: 'New task',
    searchTasks: 'Search tasks',
    tasks: 'Tasks',
    openProject: 'Open a project',
    /** Asks the service again how much money its account has left. */
    balanceAgain: 'Check the balance',
    // A conversation that belongs to no project, as it is named, and the way into it.
    chat: 'Chat',
    justChat: 'Just chat',
    settings: 'Settings',
    hideTasks: 'Hide the list of tasks',
    showTasks: 'Show the list of tasks',
    changes: 'Changes',
    details: 'Details',
    close: 'Close',
    cancel: 'Cancel',
    done: 'Done',
    apply: 'Apply',
    copy: 'Copy',
    copied: 'Copied',
    send: 'Send',
    stop: 'Stop',
    stopping: 'Stopping',
    commands: 'Commands',
    restart: 'Start it again',
    latest: 'Latest',
    replay: 'Replay',
    showLog: 'Show the log',
    lastWords: 'What it wrote last',
    recentProjects: 'Recent projects',
    notices: 'Notices',
    tellMe: 'Tell me when Columbina waits for me or has finished',
    /** Whether the application starts with the system. */
    startup: 'Starting',
    loginItem: 'Start Columbina in the tray when I sign in to Windows',

    /** The window in which a service is chosen and its key is given. */
    key: {
      title: 'Model service',
      service: 'Service',
      ownService: 'A service of your own',
      name: 'Name',
      address: 'Address',
      model: 'Model',
      contextWindow: 'Context window',
      tokens: 'tokens',
      apiKey: 'API key',
      store: 'Store',
      storing: 'Storing',
      check: 'Check',
      checking: 'Checking',
      getKey: 'Get a key',
      forget: 'Delete the stored key',
      forgetConfirm: 'Delete it',
      keep: 'Keep it',
      setUp: 'Set up a service or a key',
      inUse: 'In use',
      from: {
        environment: 'Key from a variable',
        stored: 'Key stored',
        storedElsewhere: 'Key for another address',
        none: 'No key',
        notNeeded: 'No key needed',
      },
    },

    /** The models of the service in use, as the field and the settings offer them. */
    models: {
      /** The name of the button that shows the model in use, for whoever cannot see it. */
      button: (model: string) => `Model: ${model}`,
      filter: 'Filter, or write the name of a model',
      again: 'List the models again',
      /** A model that was written into the filter, and that the service does not list. */
      use: (model: string) => `Use ${model}`,
      tryAgain: 'Try again',
      settings: 'Model settings',
      /** The services that are set up, at the head of the list of models. */
      services: 'Services',
      /** The name of the service in use among them, for whoever cannot see its mark. */
      inUse: (service: string) => `${service}, in use`,
    },

    /** How hard the model thinks, as the field and the settings offer it. */
    effort: {
      title: 'How hard the model thinks',
      /** What the button that shows the level in use says. */
      button: (level: string) => `Thinking: ${level}`,
      names: { default: 'Default', off: 'Off', low: 'Low', high: 'High', max: 'Max', ultra: 'Ultra' },
    },

    /** The files that go with a message. */
    attachments: {
      attach: 'Attach files',
      list: 'Files that go with the message',
      remove: (name: string) => `Remove ${name}`,
      /** The line before the list of the files, at the end of the message. */
      heading: 'Attached files:',
    },

    /** Signing in to GitHub, and who is signed in. */
    github: {
      signIn: 'Sign in with GitHub',
      enterCode: 'Enter this code on GitHub',
      copyCode: 'Copy the code',
      openGitHub: 'Open GitHub',
      /** How long the code holds yet, as a clock shows it. */
      left: (clock: string) => `${clock} left`,
      newCode: 'Ask for a new code',
      tryAgain: 'Try again',
      checkAgain: 'Check again',
      repositories: 'Repositories that Columbina may reach',
      chooseRepositories: 'Choose repositories',
      manageRepositories: 'Manage repositories',
      signOut: 'Sign out',
      stay: 'Stay signed in',
      revoke: 'Take back the right on GitHub',
    },

    /** The platforms on which an assistant answers. */
    platforms: { feishu: 'Feishu', lark: 'Lark', wecom: 'WeCom', qq: 'QQ' },

    /** The assistant in Feishu: in the settings, and in the window that takes its app. */
    feishu: {
      title: 'Assistant in Feishu',
      setUp: 'Set up an app',
      change: 'Change the app',
      start: 'Start the assistant',
      starting: 'Starting',
      stop: 'Stop',
      stopping: 'Stopping',
      forget: 'Delete the app',
      forgetConfirm: 'Delete it',
      keep: 'Keep it',
      console: 'Open the developer console',
      log: 'Show the log',
      /** Whether the agent of the assistant may change the files of its folder. */
      writable: 'Let the assistant change the files in its folder',
      folder: 'Open its folder',
      checkAgain: 'Check again',
      copyCode: 'Copy the code',
      pair: (platform: string) => `Send the bot this code in ${platform}`,
      platform: 'Platform',
      choose: (platform: string) => `Choose ${platform}`,
      appId: 'App ID',
      appSecret: 'App Secret',
      store: 'Check and store',
      storing: 'Checking',
      steps: 'In the developer console',
      next: 'Next steps',
    },

    /** The assistant in WeCom: in the settings, and in the window that takes its bot. */
    wecom: {
      title: 'Assistant in WeCom',
      setUp: 'Set up a bot',
      change: 'Change the bot',
      start: 'Start the assistant',
      starting: 'Starting',
      stop: 'Stop',
      stopping: 'Stopping',
      forget: 'Delete the bot',
      forgetConfirm: 'Delete it',
      keep: 'Keep it',
      console: 'Open the admin console',
      log: 'Show the log',
      /** Whether the agent of the assistant may change the files of its folder. */
      writable: 'Let the assistant change the files in its folder',
      folder: 'Open its folder',
      checkAgain: 'Check again',
      copyCode: 'Copy the code',
      pair: (platform: string) => `Send the bot this code in a chat of your own in ${platform}`,
      botId: 'Bot ID',
      secret: 'Secret',
      store: 'Check and store',
      storing: 'Checking',
      steps: 'In the admin console',
      next: 'Next steps',
    },

    /** The assistant in QQ: in the settings, and in the window that takes its bot. */
    qq: {
      title: 'Assistant in QQ',
      setUp: 'Set up a bot',
      change: 'Change the bot',
      start: 'Start the assistant',
      starting: 'Starting',
      stop: 'Stop',
      stopping: 'Stopping',
      forget: 'Delete the bot',
      forgetConfirm: 'Delete it',
      keep: 'Keep it',
      console: 'Open the open platform',
      log: 'Show the log',
      folder: 'Open its folder',
      checkAgain: 'Check again',
      copyCode: 'Copy the code',
      pair: (platform: string) => `Send the bot this code in a chat of your own in ${platform}`,
      appId: 'AppID',
      secret: 'AppSecret',
      store: 'Check and store',
      storing: 'Checking',
      steps: 'On the open platform',
      next: 'Next steps',
    },

    /** The model of an assistant and how hard it thinks, in its part of the settings. */
    assistantThinking: {
      model: 'Model',
      effort: 'Thinking',
      followModel: 'The model that Columbina uses',
      keep: 'Keep',
      levels: {
        follow: 'As for all', default: 'As the service has it', off: 'Off', low: 'Low', high: 'High', max: 'The most',
      },
    },

    /** The first steps on the first page. */
    start: {
      title: 'Getting started',
      model: 'Model service',
      optional: 'Optional',
      project: 'Project',
      connect: 'Connect',
      change: 'Change',
      skip: 'Skip',
      hide: 'Hide the first steps',
      show: 'Show the first steps',
    },

    /** What the system shows of this application while its window is in the background. */
    notice: {
      consent: 'Columbina needs your consent',
      finished: 'Columbina has finished',
      stopped: 'Columbina has stopped',
    },
    mute: 'Turn the sound off',
    unmute: 'Turn the sound on',
    soundOn: 'Sound on',
    soundOff: 'Sound off',

    /** The film that plays when the application starts. */
    opening: {
      name: 'Opening',
      loading: 'Loading the opening',
      play: 'Play the opening',
      again: 'Play it again',
      skip: 'Skip',
      anyKey: 'Any key or a click skips',
      begin: 'Press any key to begin',
    },

    /** The music that plays behind the work. */
    music: {
      none: 'No music chosen',
      choose: 'Choose a file',
      change: 'Choose another file',
      play: 'Play',
      useKept: 'Use the music of the application folder',
      remove: 'Remove',
      stands: {
        loading: 'Loading',
        none: 'No music chosen',
        unavailable: 'Cannot be played',
        blocked: 'Waits for you to press play',
        muted: 'Muted',
        playing: 'Playing',
      },
    },
    otherProject: 'Open another project',
    skipToInput: 'Go to the input field',
    confirmLeave: 'Stop the turn that is under way?',
    stay: 'Stay here',
    stopAndLeave: 'Stop it and leave',

    /** How long ago a task was last worked on. */
    ago: {
      now: 'just now',
      minutes: (count: number) => `${count} min ago`,
      hours: (count: number) => `${count} h ago`,
      days: (count: number) => `${count} d ago`,
    },

    /** Where the agent stands, in a word. */
    status: {
      ready: 'At rest',
      working: 'At work',
      stopping: 'Stopping',
      stopped: 'Stopped',
      offline: 'Not connected',
      starting: 'Starting',
      opening: 'Opening',
    },
    context: (percent: number) => `Context ${percent}%`,
    contextWindow: 'Context',
    usage: 'Tokens of this task',
    usageIn: 'Read',
    usageOut: 'Written',
    usageCached: 'Cached',
    stats: {
      turns: (count: number) => (count === 1 ? '1 turn' : `${count} turns`),
      steps: (count: number) => (count === 1 ? '1 step' : `${count} steps`),
      speed: (rate: string) => `${rate} tok/s`,
      tokens: (count: string) => `${count} tokens`,
      cached: (percent: string) => `${percent}% cached`,
    },
    trace: {
      view: { conversation: 'Conversation', trace: 'Trace' },
      views: 'View of the task',
      kinds: { input: 'Input', thinking: 'Thinking', reply: 'Answer', tool: 'Call', notice: 'Notice' },
      overview: 'The steps of the task',
      filters: 'What the trace shows',
      onlyWrong: 'Only what went wrong',
      search: 'Search the trace',
      clear: 'Clear the search',
      found: (at: number, count: number) => `${at} of ${count}`,
      noneFound: 'Nothing found',
      turn: (number: number) => `Turn ${number}`,
      ends: {
        completed: 'done', aborted: 'stopped', blocked: 'blocked', error: 'failed', 'max-tokens': 'at the length limit',
        interrupted: 'cut short', forked: 'forked', running: 'under way', other: 'ended',
      },
      states: { refused: 'refused', cut: 'cut short', waiting: 'waiting for a yes', running: 'running' },
      onlyCalls: (count: number) => (count === 1 ? 'Calls a tool' : `Calls ${count} tools`),
      nothingWritten: 'Wrote nothing',
      tokens: (read: string, written: string) => `${read} in · ${written} out`,
      cut: 'cut short',
      copy: 'Copy',
      copied: 'Copied',
      arguments: 'Arguments',
      result: 'Result',
      error: 'Error',
    },
    tools: 'Tools',
    taskId: 'Task',

    /** What a tool does, in a word that fits before what it does it to. */
    verb: {
      read: 'Read',
      write: 'Write',
      edit: 'Edit',
      list: 'List',
      search: 'Search',
      run: 'Run',
      todo: 'Keep a list',
      skill: 'Use a skill',
      subagent: 'Hand over',
      githubRepos: 'List the repositories on GitHub',
      githubRead: 'Read on GitHub',
      githubChecks: 'See the checks of',
      githubClone: 'Clone',
      githubPush: 'Push',
      githubPullRequest: 'Open a pull request',
      githubComment: 'Comment on',
      other: (tool: string) => `Call ${tool}`,
    },
    /** What the name beside a task that was handed over is. */
    subagentKind: 'Kind of subagent',
    input: 'What it was given',
    output: 'What came back',
    noOutput: 'Nothing came back',
    showAllLines: (lines: number) => `Show all ${lines} lines`,
    before: 'Before',
    after: 'After',
    thought: 'Thought',
    thinking: 'Thinking',
    writing: 'Writing',
    preparing: 'Preparing a call',
    saidOnTheWay: 'Added on the way',
    steeredAway: 'Turned to what you said',
    waiting: 'Waiting',
    summary: 'Earlier messages, summed up',

    placeholderIdle: 'Give Columbina something to do',
    placeholderChat: 'Say something to Columbina',
    placeholderRunning: 'Add something for its next step',
    placeholderNoProject: 'Open a project first',
    placeholderStarting: 'Columbina is starting',
    placeholderOpening: 'The task is being opened',
    placeholderStopped: 'This task has stopped',

    consentTitle: 'Your consent is needed',
    allowOnce: 'Allow this once',
    allowAlways: 'Allow, and do not ask again',
    reject: 'Do not allow',
    stopTurn: 'Stop the turn',
    whyItAsks: 'Why it asks',

    emptyTitle: 'What shall we work on?',
    emptyNoProjectTitle: 'Open a project to begin',
    recentTasks: 'Recent tasks',
    skills: 'Skills',
    /** Over the text of a skill that the user handed to the agent. */
    skillGiven: (name: string | null) => (name ? `Skill: ${name}` : 'A skill'),
    /** Beside a skill that a person cannot hand over. */
    skillOfTheAgent: 'Columbina loads it itself',

    appearance: 'Appearance',
    theme: 'Theme',
    themeNames: { moonveil: 'Moonveil', moonwhite: 'Moonwhite' },
    language: 'Language',
    languageSystem: 'As the system has it',
    reasoningLanguage: 'Language of reasoning',
    reasoningLanguages: { interface: 'As the interface', zh: 'Chinese', en: 'English', none: 'Not set' },
    typeSize: 'Size of type',
    typeSizes: { small: 'Small', standard: 'Standard', large: 'Large' },
    model: 'Model',
    permissions: 'Permissions',
    account: 'GitHub account',
    presets: {
      readOnly: 'Read only',
      workspaceWrite: 'Change files of the project',
      fullAccess: 'Full access',
    },
    openingAndMusic: 'Opening and music',
    playOpening: 'Play the opening when the application starts',
    playOpeningNow: 'Play the opening now',
    about: 'About',

    /** Newer versions of the application. */
    updates: {
      title: 'Updates',
      /** The button in the bar of the window, short, and what it does, in full. */
      told: (version: string) => `Update ${version}`,
      toldInFull: (version: string) => `Columbina ${version} is out. Open the page to download it`,
      open: 'Open the download page',
      skip: 'Skip this version',
      checking: 'Look for a newer version at start and once a day',
      now: 'Check now',
    },

    pet: {
      title: 'Desktop pet',
      show: 'Show a pet that follows the task',
      choose: 'Pet',
      openFolder: 'Open the folder for your own pets',
      builtin: { moon: 'Little Moon', cat: 'Little Cat', star: 'Little Star' },
      mine: 'yours',
    },

    /** The studio: a workspace for editing the media of the project. */
    studio: {
      fairlightPage: fairlightEn,
      timeline: {
        select: 'Selection mode', trim: 'Trim edit mode', blade: 'Blade edit mode', snap: 'Snapping', linked: 'Linked selection', target: 'Target track',
        title: 'Timeline', position: 'Timeline position',
        zoomIn: 'Zoom in', zoomOut: 'Zoom out', fit: 'Fit timeline', follow: 'Follow playhead',
        lock: 'Lock', unlock: 'Unlock', mute: 'Mute', unmute: 'Unmute', hide: 'Hide track', show: 'Show track', height: 'Track height',
        color: 'Solid color', wavePending: 'Preparing waveform…', waveFailed: 'Waveform unavailable',
        dissolve: 'Dissolve', dip_black: 'Dip to black', dip_white: 'Dip to white', crossfade: 'Crossfade',
        addTrack: 'Add track', addSubtitleTrack: 'Add subtitle track', addCue: 'Add subtitle', trackMenu: 'Track menu', language: 'Language', noLanguage: 'Unspecified',
        hideSubtitles: 'Hide in the viewer', showSubtitles: 'Show in the viewer, in place of the subtitle track shown', newCue: 'Subtitle',
      },
      media: {
        bins: 'Bins', master: 'Master', newBin: 'New bin', name: 'Name', rename: 'Rename', remove: 'Remove from pool', removeBin: 'Remove empty bin',
        list: 'List view', thumbnails: 'Thumbnail view', details: 'Media details', path: 'Path', videoCodec: 'Video codec', audioCodec: 'Audio codec',
        dimensions: 'Dimensions', duration: 'Duration', frameRate: 'Frame rate', depth: 'Bit depth', sampleRate: 'Sample rate', channels: 'Channels',
        markIn: 'In', markOut: 'Out', noProxy: 'No proxy', selected: (count: number) => `${count} selected`,
        /** The right-click menu of media (Reference Manual 20, pp. 221–222, 230, 245). */
        menu: 'Media menu', inputColorSpace: 'Input Color Space', inputGamma: 'Input Gamma', acesInput: 'ACES Input Transform',
        projectDefault: 'Project Default', bypass: 'Bypass', dataLevels: 'Data Levels', levels: { auto: 'Auto', video: 'Video', full: 'Full' },
        /** The clip's own switch of the project's inverse DRT (p. 233), in DaVinci Resolve's words. */
        inverseDrt: 'Inverse DRT for SDR to HDR Conversion',
      },
      /** Its way in, beside the tasks, and its name. */
      open: 'Studio',
      close: 'Close the studio',
      pages: 'Pages of the studio',
      page: { media: 'Media', cut: 'Cut', edit: 'Edit', color: 'Color', fairlight: 'Fairlight' },
      /** The two viewers of the Edit page, as DaVinci Resolve names them. */
      monitor: {
        source: 'Source', timeline: 'Timeline', sourceTitle: 'Source viewer', timelineTitle: 'Timeline viewer', choose: 'Viewer shown',
        /** While a change of the colour management is made, the colour before it still shown (`color_pending`); and where it cannot be made. */
        colorPending: 'Applying color management…',
        colorFailed: (message: string) => `Color management could not be applied; the viewer shows the color before the change. ${message}`,
      },
      editPage: { inspector: 'Inspector', newTimeline: 'New timeline', create: 'Create', noTimeline: 'No timeline' },
      /** The Project Settings window (Reference Manual 20, pp. 19–20, 149–156), in DaVinci Resolve's words. */
      settings: {
        title: 'Project Settings', panels: 'Panels', colorManagement: 'Color Management', generalOptions: 'General Options',
        spaces: 'Color Space & Transforms', lookupTables: 'Lookup Tables', color: 'Color',
        science: 'Color science', automatic: 'Automatic color management', processingMode: 'Color processing mode',
        preset: 'Resolve color management preset', custom: 'Custom', output: 'Output color space', separateGamma: 'Use Separate Color Space and Gamma',
        input: 'Input color space', timeline: 'Timeline color space', inputGamma: 'Input gamma', timelineGamma: 'Timeline gamma', outputGamma: 'Output gamma',
        bypass: 'Bypass', workingLuminance: 'Timeline working luminance', nits: 'Nits', hdr203: 'Use 203 nits reference for Rec.2100 HDR',
        limitGamut: 'Limit output gamut to', none: 'None', inputDrt: 'Input DRT', outputDrt: 'Output DRT',
        drts: { none: 'None', simple: 'Simple', luminance: 'Luminance Mapping', saturation_preserving: 'Saturation Preserving' },
        rolloffStart: 'Sat. Rolloff Start', rolloffLimit: 'Sat. Rolloff Limit',
        inverseDrt: 'Use inverse DRT for SDR to HDR conversion', whitePoint: 'Use white point adaptation', aware: 'Use color space aware grading tools',
        resizeIn: 'Apply resize transformations in',
        resizes: { timeline: 'Timeline', linear: 'Linear', linear_mapped: 'Linear Mapped', gamma: 'Gamma', gamma_mapped: 'Gamma Mapped' },
        graphicsWhite: 'Graphics white level', displayHdr: 'Display HDR On Viewers If Available',
        acesVersion: 'ACES version', idt: 'ACES Input Device Transform', odt: 'ACES Output Device Transform',
        gamutCompress: 'Apply ACES reference gamut compress', nodeLuts: 'Process Node LUTs in',
        inputLut: 'Input Lookup Table', outputLut: 'Output Lookup Table', viewerLut: 'Color Viewer Lookup Table',
        lut1d: '1D lookup table', lut3d: '3D lookup table', noLut: 'No LUT selected', missing: 'missing',
        interpolation: '3D Lookup Table Interpolation', trilinear: 'Trilinear', tetrahedral: 'Tetrahedral', updateLists: 'Update Lists',
        openLutFolder: 'Open LUT Folder', sCurve: 'Use S-curve for contrast', save: 'Save',
      },
      /** Exporting a timeline: DaVinci Resolve's Quick Export, with the settings of its render. */
      render: {
        quickExport: 'Quick export', title: 'Export timeline', timeline: 'Timeline', range: 'Range', whole: 'Entire timeline', marked: 'In to out',
        codec: 'Codec', quality: 'Quality', high: 'High', medium: 'Medium', low: 'Low', subtitles: 'Subtitle tracks to save as SRT',
        exportTo: 'Export to…', exporting: 'Exporting', cancelling: 'Cancelling', encoder: 'Encoder', reveal: 'Show in folder',
        frames: (done: number, total: number) => `${done} of ${total} frames`,
      },
      /** The old projects of the folder that became timelines as the studio was opened. */
      migration: { duckYes: "Add ducking", duckNo: "No thanks", duckPending: "Applying…", details: 'Details', hideDetails: 'Hide details', timeline: 'Timeline', dropped: 'Left out', notes: 'Notes', failed: 'Not migrated' },
      /** The Cut page, in DaVinci Resolve's words. */
      cutPage: {
        viewerMode: 'Viewer mode', source: 'Source clip', tape: 'Source tape', timeline: 'Timeline',
        edits: 'Edits', smartInsert: 'Smart insert', append: 'Append', rippleOverwrite: 'Ripple overwrite', placeOnTop: 'Place on top',
        sourceOverwrite: 'Source overwrite', transitions: 'Transitions', dissolve: 'Dissolve', dipBlack: 'Dip to black',
        overview: 'Whole timeline', detail: 'Timeline around the playhead', tapeStrip: 'Source tape of the bin',
      },
      /** The Color page, in DaVinci Resolve's words (Reference Manual 20, Chapters 126–128 and 143). */
      colorPage: {
        viewerZoom: 'Viewer zoom', viewerFit: 'Fit', matching: 'Matching the selected clips…',
        windowControls: {
          addGradient: 'Add Gradient Window', addLinear: 'Add Linear Window', addPolygon: 'Add Polygon Window', addCurve: 'Add Curve Window',
          fields: { size: 'Size', aspect: 'Aspect', pan: 'Pan', tilt: 'Tilt', rotate: 'Rotate', opacity: 'Opacity', softness: 'Soft 1',
            soft1: 'Soft 1', soft2: 'Soft 2', soft3: 'Soft 3', soft4: 'Soft 4', inside: 'Inside Softness', outside: 'Outside Softness' },
          point: 'Point', inserted: 'New point', handleIn: 'in handle', handleOut: 'out handle',
          /** A Curve's Shift-selected points (RM20 p. 3195). */
          selectedPoints: 'Selected points: {count}',
          /** The parts of a refused point edit's explanation: the points and the key interval it names. */
          topology: {
            one: 'point {point}', two: 'points {first} and {second}',
            frames: '{clock} frames {from}–{to}', allFrames: 'every frame',
          },
          osc: 'On-screen controls', none: 'Off', qualifier: 'Qualifier', window: 'Power Window', image_wipe: 'Image Wipe', add: 'Add Circular Window',
          list: 'Windows', editing: 'Editing', static: 'Static', key: 'Key frame', frame: 'Frame', creating: 'Creating window…',
          /** The switches of each row of the Window list (RM20 p. 3189): Mask on adds the window, off subtracts it. */
          invert: 'Invert', mask: 'Mask',
          /** A row's shape button, which turns the window on and off, and its name field (RM20 p. 3189). */
          onOff: 'On/Off', layerName: 'Layer Name',
          clock: { source: 'Source clock', clip: 'Clip clock', timeline: 'Timeline clock' },
          shapes: { circular: 'Circular', linear: 'Linear', polygon: 'Polygon', curve: 'Curve', gradient: 'Gradient' },
          /** While a window is dragged; nothing is said while none is. */
          status: { arming: 'Preparing window drag…', armed: 'Dragging window · Esc to cancel' }
        },
        toolbar: 'Interface toolbar', gallery: 'Gallery', luts: 'LUTs', clips: 'Clips', clipFilter: 'Clip filter', timeline: 'Timeline', nodes: 'Nodes',
        filter: { all: 'All Clips', graded: 'Graded Clips', ungraded: 'Ungraded Clips', grouped: 'Grouped Clips', matched: 'Shot Matched Clips' },
        viewer: 'Viewer', lutBrowser: 'LUT browser', nodeEditor: 'Node editor', thumbnails: 'Thumbnail timeline', miniTimeline: 'Mini-timeline',
        videoTracks: 'Video tracks', level: 'Node graph shown',
        levels: { pre_clip: 'Group Pre-Clip', clip: 'Clip', post_clip: 'Group Post-Clip', timeline: 'Timeline' },
        nodeSource: 'Source', nodeOutput: 'Output',
        kinds: { corrector: 'Corrector', parallel_mixer: 'Parallel Mixer', layer_mixer: 'Layer Mixer', key_mixer: 'Key Mixer' },
        /** The node editor as it is edited, and its right-click menus (Reference Manual 20, pp. 3310–3322). */
        nodeMenu: {
          nodeMenu: 'Node menu', graphMenu: 'Node graph menu', link: 'Link', thumbMenu: 'Clip menu', tools: 'Node editor tools',
          selectTool: 'Selection tool', panTool: 'Hand tool', zoom: 'Zoom', labelField: 'Node label',
          addNode: 'Add Node', serial: 'Add Serial', before: 'Add Before Current', parallel: 'Add Parallel', layer: 'Add Layer', outside: 'Add Outside Node',
          /** A serial node with a window already turned on (RM20 p. 3187). */
          serialWindow: { circular: 'Add Serial Node + CPW', linear: 'Add Serial Node + LPW', polygon: 'Add Serial Node + PPW', curve: 'Add Serial Node + PCW' },
          label: 'Change Label', deleteNode: 'Delete Node', reset: 'Reset Node Grade', resetShotMatch: 'Reset Shot Match', shotMatch: 'Shot Match to This Clip',
          addInput: 'Add One Input', toLayer: 'Morph Into Layer Mixer Node', toParallel: 'Morph Into Parallel Node', composite: 'Composite Mode',
          deleteLink: 'Delete Link', resetAll: 'Reset All Grades and Nodes', cleanup: 'Cleanup Node Graph',
          zoomIn: 'Zoom In', zoomOut: 'Zoom Out', zoomToWindow: 'Zoom to Window', originalSize: 'Original Size', collapse: 'Collapse Group Grades',
          revealLut: 'Reveal Selected LUT', lock: 'Lock Node',
        },
        /** The composite modes of a Layer Mixer, as DaVinci Resolve names them (pp. 1055–1058). */
        composites: {
          normal: 'Normal', add: 'Add', color: 'Color', color_burn: 'Color Burn', color_dodge: 'Color Dodge', darken: 'Darken', darker_color: 'Darker Color',
          difference: 'Difference', divide: 'Divide', exclusion: 'Exclusion', hard_mix: 'Hard Mix', hardlight: 'Hardlight', hue: 'Hue', lighten: 'Lighten',
          lighter_color: 'Lighter Color', linear_burn: 'Linear Burn', linear_dodge: 'Linear Dodge', linear_light: 'Linear Light', luminosity: 'Luminosity',
          multiply: 'Multiply', overlay: 'Overlay', pin_light: 'Pin Light', saturation: 'Saturation', screen: 'Screen', softlight: 'Softlight',
          subtract: 'Subtract', vivid_light: 'Vivid Light',
        },
        disabled: 'Disabled', locked: 'Locked', matched: 'Shot matched', graded: 'Graded', grouped: 'Group', solidColor: 'Solid color', title: 'Title',
        palettes: 'Palettes', leftPalettes: 'Left palettes', centerPalettes: 'Center palettes', rightPanel: 'Keyframes and scopes',
        palette: {
          primaries: 'Primaries', curves: 'Curves', qualifier: 'Qualifier', window: 'Window', tracker: 'Tracker', blur: 'Blur', key: 'Key',
          keyframes: 'Keyframes', scopes: 'Scopes',
        },
        timecode: 'Viewer timecode',
        timecodes: { source: 'Source Timecode', record: 'Timeline Timecode', sourceFrame: 'Source Frame', recordFrame: 'Timeline Frame' },
        realtime: 'Playing in real time', dropping: 'Dropping frames', stopped: 'Not playing',
        enhanced: 'Enhanced viewer', full: 'Full page viewer', cinema: 'Cinema viewer', exitCinema: 'Exit cinema viewer',
        missingLut: (name: string) => `Missing LUT: ${name}`, missingLuts: 'Multiple LUTs missing',
        /** The LUT browser (Reference Manual 20, pp. 2970–2971, 3410–3413), in DaVinci Resolve's words. */
        lutPanel: {
          folders: 'LUT folders', list: 'LUTs', project: 'Project LUTs', library: 'LUT library', favorites: 'Favorites', missing: 'Missing LUTs',
          search: 'Search', refresh: 'Refresh', menu: 'LUT menu', apply: 'Apply LUT to Current Node', addFavorite: 'Add to Favorites',
          removeFavorite: 'Remove from Favorites', name: 'LUT name', clips: 'Clips', none: 'None',
          broken: 'cannot be read',
          /** The option menu of the LUT browser (pp. 3259, 3411), in DaVinci Resolve's words. */
          options: 'LUT browser options', livePreview: 'Live Preview', hoverScrub: 'Hover Scrub Preview',
          scrubs: { both: 'Thumbnail and Viewer', thumbnail: 'Thumbnail Only', off: 'Off' },
        },
        /** The options menu of the viewer (p. 2986) with what the plan adds to it (p. 244). */
        viewerOptions: 'Viewer options', displayHdr: 'Display HDR On Viewers If Available', viewerLut: 'Viewer LUT',
        /** The Gallery, its stills and memories (Reference Manual 20, ch. 141–142), in DaVinci Resolve's words. */
        galleryPanel: {
          albums: 'Albums', memories: 'Memories', stills: 'Stills', stillSize: 'Still size', search: 'Search',
          albumMenu: 'Album menu', stillMenu: 'Still menu', galleryMenu: 'Gallery menu', memoryMenu: 'Memory menu', viewerMenu: 'Viewer menu',
          addStillAlbum: 'Add Still Album', addPowerGradeAlbum: 'Add PowerGrade Album', removeAlbum: 'Remove Current Album', renameAlbum: 'Album name',
          applyGrade: 'Apply Grade', appendNodeGraph: 'Append Node Graph', deleteSelected: 'Delete Selected', properties: 'Properties',
          changeLabel: 'Change Label', export: 'Export', import: 'Import', grabStill: 'Grab Still',
          applyGradesUsing: 'Apply Grades Using',
          keyframes: { none: 'No Keyframes', source_timecode: 'Keyframes Aligning Source Timecode', start_frame: 'Keyframes Aligning Start Frames' },
          preserveNodes: 'Copy Grade: Preserve number of nodes',
          memory: 'Memory', clearMemory: 'Clear', memoriesAlbum: 'Memories',
          appendToSelected: 'Append Node to Selected Clips', wipeTimelineClip: 'Wipe Timeline Clip',
          propertiesTitle: 'Still Properties', stillId: 'Still ID', labelField: 'Label', created: 'Created', timeline: 'Timeline', clip: 'Clip',
          recordTimecode: 'Record TC', sourceTimecode: 'Source TC', size: 'Size', imported: 'Imported from', ok: 'OK', delete: 'Delete',
        },
        /** Wipes and split screens of the viewer (pp. 2999–3006), in DaVinci Resolve's words. */
        compare: {
          modes: 'Viewer modes', imageWipe: 'Image Wipe', splitScreen: 'Split Screen', wipeOnOff: 'Wipe On/Off', wipeStyle: 'Wipe Style',
          styles: {
            horizontal: 'Horizontal', vertical: 'Vertical', diagonal: 'Diagonal', mix: 'Mix', difference: 'Difference', box: 'Box',
            venetian_blind: 'Venetian Blind', checkerboard: 'Checkerboard',
          },
          invert: 'Invert Wipe', referenceMode: 'Reference Wipe Mode', referenceModes: { gallery: 'Gallery', timeline: 'Timeline' },
          gang: 'Gang Timeline Wipe With Current Clip', splitMode: 'Split screen mode',
          splitModes: { current_group: 'Current Group', neighbor_clips: 'Neighbor Clips', selected_clips: 'Selected Clips', selected_stills: 'Selected Still Images' },
          showOutline: 'Show Outline', reference: 'Wipe reference clip',
        },
        /** The Qualifier palette (Reference Manual 20, pp. 3157–3184), in DaVinci Resolve's words. */
        qualifierPalette: {
          modes: 'Qualifier mode', hsl: 'HSL', rgb: 'RGB', lum: 'LUM', reset: 'Reset', invert: 'Invert', selectionRange: 'Selection Range',
          tools: { pick: 'Sample Eyedropper', subtract: 'Subtract Color Range', add: 'Add Color Range', harden: 'Subtract Softness', soften: 'Add Softness' },
          components: { hue: 'Hue', sat: 'Sat', lum: 'Lum', red: 'Red', green: 'Green', blue: 'Blue' },
          center: 'Center', width: 'Width', soft: 'Soft', symmetry: 'Sym', low: 'Low', high: 'High', lowSoft: 'L. Soft', highSoft: 'H. Soft',
          finesse: {
            title: 'Matte Finesse', pre_filter: 'Pre-Filter', clean_black: 'Clean Black', black_clip: 'Black Clip', clean_white: 'Clean White',
            white_clip: 'White Clip', blur_radius: 'Blur Radius', in_out_ratio: 'In/Out Ratio', morph: 'Morph Operation', morph_radius: 'Morph Radius',
            morphs: { shrink: 'Shrink', grow: 'Grow', opening: 'Opening', closing: 'Closing' },
          },
        },
        /** The Key palette (Reference Manual 20, pp. 3358–3363), in DaVinci Resolve's words. */
        keyPalette: {
          input: 'Key Input', output: 'Key Output', qualifier: 'Qualifier', invert: 'Invert', matte: 'Matte', mask: 'Mask', gain: 'Gain', offset: 'Offset',
          blurRadius: 'Blur Radius', blurRatio: 'Blur H/V Ratio', reset: 'Reset', mixerInput: (number: number) => `Input ${number}`,
        },
        /** Highlight (pp. 2986, 3170–3171), Bypass Color (p. 2987), Generate LUT (p. 3413) and Update All Thumbnails (p. 3026). */
        highlight: { menu: 'Highlight', off: 'Off', highlight: 'Highlight', bw: 'Highlight B/W' },
        bypass: 'Bypass Color Grades', generateLut: 'Generate LUT', lutSize: (size: number) => `Generate 3D LUT (${size} Point Cube)`,
        updateThumbnails: 'Update All Thumbnails',
        /** The orange dot of a palette's button: the current node is adjusted there. */
        changed: 'Adjusted',
        /** The Primaries palette (Reference Manual 20, Chapter 131), in DaVinci Resolve's words. */
        primariesPalette: {
          modes: 'Primaries mode', wheels: 'Color Wheels', bars: 'Color Bars', log: 'Log Wheels', reset: 'Reset',
          wheel: { lift: 'Lift', gamma: 'Gamma', gain: 'Gain', offset: 'Offset', shadow: 'Shadow', midtone: 'Midtone', highlight: 'Highlight' },
          resetWheel: 'Reset the wheel', master: 'Master wheel', balance: 'Color balance',
          settings: {
            temperature: 'Temp', tint: 'Tint', contrast: 'Contrast', pivot: 'Pivot', midtone_detail: 'Mid/Detail', color_boost: 'Col Boost',
            shadows: 'Shad', highlights: 'Hi/Light', saturation: 'Sat', hue: 'Hue', lum_mix: 'L. Mix', low_range: 'Low Range', high_range: 'High Range',
          },
          auto: 'Auto Color', blackPoint: 'Pick Black Point', whitePoint: 'Pick White Point',
        },
        /** The Curves palette (Reference Manual 20, Chapter 134), in DaVinci Resolve's words. */
        curvesPalette: {
          modes: 'Curve', custom: 'Custom', hue_vs_hue: 'Hue vs Hue', hue_vs_sat: 'Hue vs Sat', hue_vs_lum: 'Hue vs Lum',
          lum_vs_sat: 'Lum vs Sat', sat_vs_sat: 'Sat vs Sat', sat_vs_lum: 'Sat vs Lum',
          editor: 'Curve editor', point: 'Control point', resetCustom: 'Reset Custom Curve', reset: 'Reset',
          gang: 'Gang Custom Curves', channels: { y: 'Luma', r: 'Red', g: 'Green', b: 'Blue' }, intensity: 'Intensity', resetChannel: 'Reset the curve',
          options: 'Options', defaultAnchors: 'Default Anchors', copyToRed: 'Copy to Red', copyToGreen: 'Copy to Green', copyToBlue: 'Copy to Blue',
          softClip: 'Soft Clip', gangSoftClip: 'Gang Soft Clip', low: 'Low', lowSoft: 'Low Soft', high: 'High', highSoft: 'High Soft',
          input: 'Input', hueRotate: 'Hue Rotate', saturation: 'Saturation', lum: 'Lum',
          vectors: { red: 'Red', yellow: 'Yellow', green: 'Green', cyan: 'Cyan', blue: 'Blue', magenta: 'Magenta' },
          picker: 'Sample in the viewer',
        },
        /** The Blur palette (Reference Manual 20, pp. 3454–3456), in DaVinci Resolve's words. */
        /** The Window palette's option menu: its window presets (Reference Manual 20, pp. 3198–3199). */
        windowPresets: {
          options: 'Window options', saveNew: 'Save as New Preset', update: 'Update Preset', delete: 'Delete Preset',
          presets: 'Presets', none: 'No presets', name: 'Preset name', ok: 'OK',
        },
        blurPalette: {
          modes: 'Mode', blur: 'Blur', sharpen: 'Sharpen', mist: 'Mist',
          radius: 'Radius', ratio: 'H/V Ratio', scaling: 'Scaling', coring_softness: 'Coring Softness', level: 'Level', mix: 'Mix',
          gang: 'Gang the red, green and blue sliders', reset: 'Reset', resetColumn: 'Reset',
        },
        /** The Tracker palette in Window mode (Reference Manual 20, pp. 3234–3238), in DaVinci Resolve's words. */
        trackerPalette: {
          transforms: 'Transforms to track', analyze: 'Analyze',
          trackOneReverse: 'Track One Frame Reverse', trackReverse: 'Track Reverse', stop: 'Stop Tracking', trackForward: 'Track Forward',
          trackForwardBack: 'Track Forward and Back', trackOneForward: 'Track One Frame Forward',
          pan: 'Pan', tilt: 'Tilt', zoom: 'Zoom', rotate: 'Rotate', perspective: 'Perspective 3D',
          mode: 'Clip or Frame', clip: 'Clip', frame: 'Frame', graph: 'Tracker graph', scale: 'Graph height',
          tracker: 'Tracker type', cloud: 'Cloud Tracker', point: 'Point Tracker',
          addTracker: 'Add Tracker', deleteTracker: 'Delete Tracker', crosshair: (number: number) => `Tracker ${number}`, x: 'X %', y: 'Y %',
          options: 'Options', reset: 'Reset Track Data on Active Window', clearSelected: 'Clear Selected Track Data',
          addKey: 'Add Keyframe', deleteKey: 'Delete Keyframe', copy: 'Copy Track Data', paste: 'Paste Track Data',
        },
        /** The video scopes (Reference Manual 20, pp. 3007–3022), in DaVinci Resolve's words. */
        scopes: {
          kinds: { waveform: 'Waveform', parade: 'Parade', vectorscope: 'Vectorscope', histogram: 'Histogram', chromaticity: 'CIE Chromaticity' },
          scope: 'Scope', controls: 'Scope settings', menu: 'Video scope options', expand: 'Open in a floating window', close: 'Close',
          window: 'Video Scopes', layout: 'Scope layout', layouts: { one: '1 Up', two: '2 Up', four: '4 Up', nine: '9 Up' },
          waveformScale: 'Waveform Scale Style', tenBit: '10-bit', percent: 'Percentage',
          vectorscopeScale: 'Vectorscope Scale Style', off: 'Off', standard: 'Standard', both: 'Both', hueLabels: 'Show Labels',
          mode: 'Mode', tones: { all: 'ALL', low: 'Low', mid: 'Mid', high: 'High' }, lowRange: 'Low Range', highRange: 'High Range',
          zoom2x: 'Show 2X Zoom', skinTone: 'Show Skin Tone Indicator', showGraticule: 'Show Graticule', graticule: 'Graticule', gain: 'Gain',
          reference: 'Show Reference Levels', low: 'Low', high: 'High', extraGamut: 'Additional Gamut', none: 'None',
        },
      },
      /** The fields of the inspector, as DaVinci Resolve names them. */
      inspector: {
        video: 'Video', audio: 'Audio', title: 'Title', generator: 'Generator', transition: 'Transition', settings: 'Timeline settings',
        transform: 'Transform', composite: 'Composite', fades: 'Fades',
        zoom: 'Zoom', positionX: 'Position X', positionY: 'Position Y', rotation: 'Rotation angle', opacity: 'Opacity',
        fadeIn: 'Fade in (frames)', fadeOut: 'Fade out (frames)', volume: 'Volume', pan: 'Pan',
        text: 'Text', size: 'Size', color: 'Color', align: 'Alignment', alignLeft: 'Align left', alignCenter: 'Align center', alignRight: 'Align right',
        kind: 'Type', length: 'Duration (frames)', start: 'Start on edit', middle: 'Center on edit', end: 'End on edit', removeTransition: 'Delete transition',
        name: 'Name', resolution: 'Resolution', width: 'Width', height: 'Height', custom: 'Custom', frameRate: 'Frame rate', sampleRate: 'Audio sample rate',
        changeSpeed: 'Change clip speed', titleText: 'Title', useLongest: (frames: number) => `Use ${frames} frames`,
        subtitle: 'Subtitle', cueStart: 'Start (frame)', removeCue: 'Delete subtitle',
      },
      /** DaVinci Resolve's Change Clip Speed. */
      speed: { title: 'Change clip speed', percent: 'Speed', reverse: 'Reverse speed', ripple: 'Ripple sequence', duration: 'Duration', apply: 'Change' },
      /** The effects library, as DaVinci Resolve's toolbox. */
      effects: { title: 'Effects library', videoTransitions: 'Video transitions', audioTransitions: 'Audio transitions', titles: 'Titles', generators: 'Generators', text: 'Text' },
      pool: 'Media pool',
      import: 'Import',
      /** A rate of frames as an editor writes it: 25, 29.97. */
      fps: (rate: string) => `${rate} fps`,
      bits: (bits: number) => `${bits}-bit`,
      audioOnly: 'Audio only',
      proxy: {
        queued: 'Proxy queued',
        making: (percent: number) => `Making proxy ${percent}%`,
        ready: 'Proxy ready',
        failed: 'Proxy failed',
      },
      makeAgain: 'Make again',
      makeProxyAgain: 'Make the proxy again',
      nothingLoaded: 'Nothing loaded',
      hideConversation: 'Hide the conversation',
      showConversation: 'Show the conversation',
      /** The name of the key, beside what it does. */
      space: 'Space',
      play: 'Play',
      pause: 'Pause',
      back: 'Previous frame',
      forward: 'Next frame',
      position: 'Position',
      useProxies: 'Use proxies',
      source: { proxy: 'Proxy', original: 'Original', mixed: 'Mixed' },
      sound: { on: 'Sound on', off: 'Sound off', unavailable: 'Sound unavailable', none: 'No sound' },
      retry: 'Try again',
    },

    confirmFullAccess: 'Allow full access?',
    confirmWrite: 'Let Columbina change files?',
    allowFullAccess: 'Allow full access',
    allowWrite: 'Allow changes',
    openLinkTitle: 'Open this link?',
    openLink: 'Open it',
  },

  /** Sentences that explain: under a field, in an empty place, while something is waited for. */
  hint: {
    /** What is said of the tools of a task beside its conversation, as `label.work` names them. */
    work: {
      spent: 'About what the requests of this task cost, by the prices that Columbina knows. The bill of the service is what counts.',
      plan: 'In plan mode the agent reads and plans, and changes no file and runs no command. Press again to leave it.',
      exportTask: 'Write this conversation into a page of HTML, in the folder of exports of Columbina.',
      rewindAsk: (turn: number) => `Put the files of the project back as they were before turn ${turn}? The command /rewind back undoes it.`,
      scheduled: (name: string) => `The task ${name} ran.`,
      schedules: {
        about: 'A task runs while Columbina is open, in a conversation of its own, and changes the files of its folder only. '
          + 'Nobody is asked anything while it runs, so a call that needs a yes does not run.',
        none: 'There are no tasks yet.',
        failed: 'The tasks could not be changed.',
      },
      worktrees: {
        about: 'A worktree is a copy of the project on a branch of its own. A task there changes none of your files until it is merged.',
        mergeAsk: (name: string) => `Merge the work of ${name} into the project, and remove its worktree? Your files change where its work changed them.`,
        removeAsk: (name: string) => `Remove the worktree ${name}, and drop its work? What was done there cannot be had back.`,
        failed: 'The worktree could not be changed.',
      },
      groups: {
        none: 'The bot has not been asked in a group yet.',
        empty: 'The memory is empty.',
        forgetAsk: (group: string) => `Forget what the assistant remembers of ${group}? It cannot be had back.`,
        failed: 'The groups could not be changed.',
      },
    },

    keysIdle: 'Enter sends. Shift+Enter starts a new line. A slash opens the commands.',
    keysRunning: (key: string) => `What you send with ${key}+Enter is read at once, and with Enter at the next step. Esc, pressed twice, stops the turn.`,
    escAgain: 'Press Esc once more to stop the turn.',
    returned: 'What you had sent was not read. It is back in the field.',
    confirmLeave: 'Columbina is still at work on this task. Leaving it stops the turn.',
    requestFailed: 'That did not go through.',
    commandsWhenIdle: 'A command waits until Columbina is at rest.',
    starting: (folder: string) => `Starting Columbina in ${folder}.`,
    stillStarting: 'It is still starting.',
    openingTask: 'Opening the task.',
    reading: (read: number, of: number) => `Read ${read} of ${of} events.`,
    compacting: 'Summing up earlier messages.',
    noTasks: 'No task yet in this project.',
    noMatch: 'No task matches.',
    emptyNoProject: 'Choose the folder that Columbina is to work in.',
    justChat: 'A conversation that belongs to no project. Columbina changes no file in it and runs no command.',
    moreSkills: (count: number) => (count === 1 ? 'And 1 more. /skills lists them all.' : `And ${count} more. /skills lists them all.`),
    replay: 'This is a replay of a recorded session. No model is asked for anything.',
    stats: (told: {
      turns: number; steps: number; speed: string | null; tokens: string; read: string; cached: string | null; written: string;
    }) => [
      `${told.turns === 1 ? '1 turn' : `${told.turns} turns`} and ${told.steps === 1 ? '1 step' : `${told.steps} steps`}.`,
      told.speed === null ? '' : `The model wrote ${told.speed} tokens a second on average, from when a request was made to when its answer was in.`,
      `${told.tokens} tokens in all: ${told.read} read${told.cached === null ? '' : `, ${told.cached} of them from the cache`}, and ${told.written} written.`,
    ].filter(Boolean).join(' '),
    trace: {
      empty: 'Nothing has happened in this task yet.',
      step: (told: { turn: number; step: number; took: string | null; calls: number; wrong: number; failed: number }) => [
        `Turn ${told.turn}, step ${told.step}.`,
        told.took === null ? 'The model has not answered yet.' : `The model took ${told.took}.`,
        told.failed > 0 ? `${told.failed === 1 ? 'A request' : `${told.failed} requests`} failed before.` : '',
        told.calls > 0 ? `${told.calls === 1 ? 'One call' : `${told.calls} calls`}${told.wrong > 0 ? `, ${told.wrong} of them went wrong` : ''}.` : '',
      ].filter(Boolean).join(' '),
    },
    taskStopped: 'This task has stopped and takes nothing more. Start Columbina again to go on with it.',
    serviceEnded: 'The service process has ended.',
    serviceNotStarted: 'The service process could not be started.',
    tellMe: 'Only while this window is in the background.',
    loginItem: 'The assistant in Feishu starts with it where it ran when Columbina last quit. A click on the icon in the '
      + 'tray opens the window.',

    consentRun: 'Columbina wants to run this command.',
    consentWrite: 'Columbina wants to change this file.',
    consentOther: (tool: string) => `Columbina wants to use ${tool}.`,
    /** What Columbina wants to do on GitHub, in its name of the user. */
    consentGitHub: {
      clone: 'Columbina wants to clone a repository from GitHub.',
      push: 'Columbina wants to push a branch to GitHub.',
      pullRequest: 'Columbina wants to open a pull request on GitHub.',
      comment: 'Columbina wants to write a comment on GitHub.',
    },
    consentWaiting: 'Columbina waits for your answer.',
    consentSent: 'Your answer is on its way.',
    afterNo: 'It was not done. You can tell Columbina how to go about it instead.',
    /**
     * What a longer yes covers. The service process keeps it in mind, and that
     * process ends when the project is closed.
     */
    scope: {
      thisCommand: 'Not asking again holds for this command, word for word, in this task, until the project is closed.',
      anyCall: (tool: string) =>
        `Not asking again holds for every call of ${tool} in this task, until the project is closed.`,
      onTarget: (tool: string, target: string) =>
        `Not asking again holds for ${tool} on ${target} in this task, until the project is closed.`,
      other: 'Not asking again holds for the same call in this task, until the project is closed.',
    },

    settingsWhenIdle: 'The model and the permissions can be changed when Columbina is at rest.',
    reasoningLanguage: 'The model is asked to reason in it from the next turn on, in every conversation.',
    confirmFullAccess: 'Columbina will run commands without asking, and nothing will hold them back.',
    confirmWrite: 'Columbina will be able to change the files of this project. It still asks before it runs a command.',
    openLink: 'It was written in the conversation. It opens in your browser.',
    commandsList: 'A command is carried out by Columbina itself. The model does not see it.',
    connectedService: 'Connected to the service process that runs in this folder.',
    contextOf: (used: string, window: string) => `${used} of ${window} tokens in the last request.`,
    contextUnknown: (used: string) => `${used} tokens in the last request.`,
    noToolsYet: 'The tools are known once the first request is made.',
    noChanges: 'No file was changed in this task.',
    about: (version: string, protocol: number) => `Columbina ${version}, protocol ${protocol}.`,
    logKept: 'What goes wrong in the service process is written to a log on this computer.',

    /** Newer versions of the application. */
    updates: {
      version: (version: string) => `Desktop application ${version}.`,
      fromSource: 'Run from source, the application does not look for newer versions.',
      asking: 'Asking GitHub for the newest version.',
      newer: (version: string) => `Columbina ${version} is out.`,
      newest: 'This is the newest version.',
      failed: {
        network: 'GitHub could not be reached.',
        busy: 'GitHub turned the question away for now. Try again in a while.',
        unreadable: 'No list of releases came from GitHub.',
      },
      sent: 'The check asks GitHub for the list of releases of Columbina, and sends nothing else.',
    },
    pet: {
      about: 'The pet stands over the other windows. A click on it brings Columbina back, and it can be dragged where it suits you.',
      stateFile: (file: string) => `While Columbina works here, the state of the task and the name of a tool are written to ${file}, and a pet program of your own can follow it. Nothing of the conversation is written.`,
      own: 'A pet of your own is a folder with pet.json and a sheet of frames, in the layout of the pets of Codex. Put it in the folder, and it can be chosen here.',
      none: 'No pet can be shown.',
    },
    openingUnavailable: 'The opening cannot be played.',

    /** The window in which a service is chosen and its key is given. */
    key: {
      lead: 'Which service the model comes from, and the key for it.',
      reading: 'Reading what is set up.',
      unavailable: 'Services are set up here when the application runs the service itself, not a replay.',
      ownName: 'A name of your own. It begins with a small letter and goes on with small letters, digits, - and _, '
        + '32 characters at most.',
      ownAddress: 'Where the service is, such as https://api.example.com/v1.',
      modelName: 'The name of the model, as the service has it.',
      windowSize: 'How many tokens the model takes. It may be left empty.',
      notNeeded: 'This service asks for no key.',
      fromVariable: (variable: string) =>
        `The key of this service comes from the variable ${variable}, which goes before a stored key.`,
      stored: 'A key is stored for this service. Leave the field empty to keep it.',
      storedElsewhere: 'A key is stored for this service, for another address than it has now. It is not sent to this address.',
      none: 'No key is there for this service yet.',
      whereItGoes: 'The key goes from this window to Columbina, which keeps it in the keyring of the system. '
        + 'The window that shows the conversation never has it.',
      done: (service: string, model: string) => `Columbina talks to ${service} now, with the model ${model}.`,
      inKeyring: 'The key is in the keyring of the system.',
      inFile: 'No keyring took the key. It lies in a file in the folder of Columbina, as it is and not encrypted.',
      restart: 'What was stored is used once Columbina is started again.',
      /** After a key alone was stored for the service that is chosen. */
      atOnce: 'Columbina looks for the key with every request, so it needs no new start.',
      reached: (service: string, count: number) => (count === 1
        ? `${service} answered, and lists 1 model.`
        : `${service} answered, and lists ${count} models.`),
      noModels: (service: string) => `${service} answered, and lists no model.`,
      notListed: (model: string) => `${model} is not among the models that the service lists.`,
      likeAKey: 'This looks like a key, and not like the name of a model.',
      blankInKey: 'A key has no blank in it. See whether what was pasted brought more along.',
      failed: {
        network: 'The service could not be reached. See whether this computer is online, and that the proxy is on if there is one.',
        timeout: 'The service did not answer in time. Try again in a moment.',
        key: 'The service does not take this key. See whether it was copied whole.',
        forbidden: 'The service takes the key and does not allow this: the account may have no money left, or the key no right to the models.',
        address: 'Nothing that serves models answers at this address. See whether it was written right.',
        busy: 'The service is busy. Try again in a moment.',
        unknown: 'The check did not succeed.',
      },
      notRunning: {
        ollama: 'Ollama does not answer on this computer. Start it, for instance with ollama serve, and check again.',
        lmstudio: 'LM Studio does not answer on this computer. Start its local server, and check again.',
        other: 'The service does not answer on this computer. Start it, and check again.',
      },
      forgetAsk: (service: string) => `Delete the key that is stored for ${service}? What else is set up for it stays.`,
      forgotten: (service: string) => `The key of ${service} was deleted.`,
      noneToForget: (service: string) => `No key was stored for ${service}.`,
    },

    /** How hard the model thinks. */
    effort: {
      /** What each level does, beneath its name. */
      notes: {
        default: 'As the service decides.',
        off: 'No thinking: the fastest answers.',
        low: 'A little thinking.',
        high: 'Thinks the task through.',
        max: 'The most thinking that the model does.',
        ultra: 'The most thinking, and subagents that share the work and review it. The slowest and the dearest, and only for this run.',
        /** Ultra, where the service is not told how much to think. */
        ultraTeam: 'Subagents that share the work and review it. The service is not asked to think harder. Only for this run.',
      },
      /** Before the levels: why none can be chosen now. */
      whenIdle: 'How hard the model thinks can be changed when Columbina is at rest.',
      /** The level was changed for the service process, and the setup program did not store it. */
      notKept: 'The level of thinking is in use now, and could not be kept for the next start.',
    },

    /** The models of the service in use. */
    models: {
      asking: (service: string) => `Asking ${service} for its models.`,
      none: (service: string) => `${service} lists no model.`,
      noMatch: 'No model matches.',
      whenIdle: 'The model can be changed when Columbina is at rest.',
      unavailable: 'The models of a service are listed where the application runs the service itself, not in a replay.',
      /** The model was changed for the service process, and the setup program did not store it. */
      notKept: 'The model is in use now, and could not be kept for the next start.',
      /** A model of another service is shown, and can be chosen. */
      switch: (service: string) => `Choosing a model of ${service} starts Columbina again with ${service}, and the task goes on.`,
      /** The setup program did not store the service and the model that were chosen. */
      notSwitched: 'The service was not changed.',
      failed: {
        network: 'The service could not be reached. See whether this computer is online, and that the proxy is on if there is one.',
        timeout: 'The service did not answer in time. Try again in a moment.',
        key: 'The service does not take the key that is set up for it.',
        forbidden: 'The service takes the key and does not let it list the models: the account may have no money left, or the key no right to them.',
        address: 'Nothing that serves models answers at the address of this service.',
        busy: 'The service is busy. Try again in a moment.',
        unknown: 'The models could not be listed.',
      },
      notRunning: {
        ollama: 'Ollama does not answer on this computer. Start it, for instance with ollama serve, and try again.',
        lmstudio: 'LM Studio does not answer on this computer. Start its local server, and try again.',
        other: 'The service does not answer on this computer. Start it, and try again.',
      },
    },

    /** The files that go with a message. */
    attachments: {
      picture: 'The model looks at a picture with its tool read, where its service takes pictures.',
      refused: {
        gone: (name: string) => `${name} is not there any more.`,
        notAFile: (name: string) => `${name} is a folder, and only files can be attached.`,
        tooBig: (name: string) => `${name} is larger than 25 MB.`,
        tooMany: (name: string) => `${name} was left out: ten files at most go with one message.`,
      },
      command: 'A command takes no files. They wait for the next message.',
      drop: 'Let go of the files to attach them.',
      /** The files could not be copied into the project. The message was kept back with them. */
      notKept: 'The files could not be kept in the project, so the message was not sent.',
    },

    /** The first steps on the first page. */
    start: {
      lead: 'A few steps, and Columbina is ready to work.',
      noKey: (service: string) => `No key is set up for ${service} yet.`,
      github: 'Once you are signed in, Columbina can work on your repositories, issues and pull requests. '
        + 'You can also sign in later in the settings.',
    },

    /** Signing in to GitHub, and who is signed in. */
    github: {
      lead: 'Once you are signed in, Columbina can read repositories, issues, pull requests and checks in your name, '
        + 'clone and push, open pull requests and comment. It asks you before each push, pull request and comment, '
        + 'unless full access or a rule allows it.',
      checking: 'Asking who is signed in to GitHub.',
      asking: 'Asking GitHub for a code.',
      waiting: 'Columbina waits until you have confirmed it in the browser.',
      expired: 'The code has run out.',
      signedIn: (login: string) => `Signed in to GitHub as @${login}.`,
      noRepositories: 'Columbina reaches only the repositories that you choose for it on GitHub, and none is chosen yet.',
      moreRepositories: (count: number) => (count === 1 ? 'And 1 more.' : `And ${count} more.`),
      offline: 'GitHub could not be asked which repositories Columbina may reach.',
      ended: 'The sign-in to GitHub has run out. Sign in again.',
      unavailable: 'Signing in to GitHub works where the application runs the service itself, and not in a replay.',
      unreadable: 'What this computer keeps of the sign-in to GitHub cannot be read.',
      statusFailed: 'Columbina could not find out who is signed in to GitHub.',
      signOutAsk: (login: string) =>
        `Sign out of GitHub on this computer? Columbina can then no longer act in the name of @${login}.`,
      signedOut: 'Signed out on this computer. The app keeps the right that you gave it until you take it back on GitHub.',
      signOutFailed: 'The sign-out did not go through.',
      failed: {
        unreachable: 'GitHub could not be reached. See whether this computer is online, and that the proxy is on if you use one.',
        timeout: 'GitHub did not answer in time. Try again in a moment.',
        busy: 'GitHub is busy. Try again in a moment.',
        expired: 'The code ran out before it was confirmed. Ask for a new one.',
        denied: 'The sign-in was turned down on GitHub.',
        rateLimited: 'GitHub asks Columbina to wait before it asks again. The details say for how long.',
        unregistered: 'This build of Columbina cannot sign in to GitHub yet.',
        store: 'The sign-in could not be kept on this computer.',
        ended: 'The sign-in stopped before it was done.',
        notStarted: 'The sign-in could not be started.',
        unknown: 'The sign-in did not go through.',
      },
    },

    /** The assistant in Feishu. */
    feishu: {
      lead: 'Talk to Columbina in Feishu or Lark, through an app of your own on its open platform. The assistant answers '
        + 'you alone and reads the documents that are shared with the app. It runs no commands and changes none of your '
        + 'files: the files and skills that you send it are kept in a folder of its own.',
      reading: 'Asking how the assistant in Feishu is doing.',
      unavailable: 'The assistant in Feishu is set up here where the application runs the service itself, and not in a replay.',
      statusFailed: 'Columbina could not find out how the assistant in Feishu is doing.',
      notRunning: 'The assistant does not run.',
      connecting: (platform: string) => `Connecting to ${platform}.`,
      reconnecting: (platform: string) => `${platform} cannot be reached just now. The assistant keeps trying.`,
      connected: (platform: string) => `Connected. The assistant answers you in ${platform}.`,
      trouble: 'Something did not go through lately.',
      pairNote: 'Whoever sends it first is the one whom the assistant answers. The code holds for 10 minutes. '
        + 'After that, a message to the bot makes a new one here.',
      pairNotHeard: 'No answer from the bot? In the developer console, see that events come through a persistent '
        + 'connection, that the event for receiving messages is added, and that a version of the app is released.',
      noCode: 'The code was given up on after too many wrong tries. Stop the assistant and start it again for a new one.',
      stopping: 'Stopping the assistant.',
      elsewhere: 'The assistant runs elsewhere on this computer, such as in a terminal, and is stopped there.',
      withColumbina: 'While the assistant runs, closing the window keeps Columbina in the tray, where the assistant goes on '
        + 'answering. Quit from the tray to stop both. If it runs when Columbina quits, it starts again the next time.',
      writable: 'It can then make and change files in its own folder, and still runs no commands. A running assistant starts '
        + 'again for the change.',
      writableFailed: 'Whether the assistant may change files could not be changed.',
      folderFailed: 'The folder of the assistant could not be opened.',
      service: 'The assistant talks to the service that Columbina uses when the assistant starts. After another service is '
        + 'chosen, stop the assistant and start it again.',
      ended: 'The assistant stopped.',
      lastError: 'The last run of the assistant did not end well.',
      startFailed: 'The assistant could not be started.',
      stopFailed: 'The assistant could not be stopped.',
      forgetAsk: (appId: string) => `Delete the app ${appId} on this computer? Its App Secret and whom the assistant answers `
        + 'are deleted, and an assistant that runs stops first. The app on the open platform stays as it is.',
      forgotten: 'The app was deleted on this computer.',
      forgetFailed: 'The app could not be deleted.',
      windowLead: 'An app that you made on the open platform of Feishu or Lark. Its App ID and App Secret are on its page of '
        + 'credentials.',
      appId: 'It begins with cli_.',
      appIdWrong: 'An App ID begins with cli_, followed by letters and digits.',
      secret: 'The App Secret goes from this window to Columbina, which keeps it in the keyring of the system. '
        + 'The window that shows the conversation never has it.',
      secretAgain: 'The App Secret that is stored is not shown. Enter it again to store the app anew.',
      blankInSecret: 'An App Secret has no blank in it. See whether what was pasted brought more along.',
      wrongPlatform: (asked: string, other: string) => `${asked} does not know this app. An app that was made in the `
        + `developer console of ${other} is one of ${other}: choose ${other}, and store it again.`,
      done: (appId: string, platform: string) => `${platform} takes the app ${appId}, and it is stored.`,
      inKeyring: 'The App Secret is in the keyring of the system.',
      inFile: 'No keyring took the App Secret. It lies in a file in the folder of Columbina, as it is and not encrypted.',
      steps: (platform: string) => [
        'In the developer console, make a custom app. Its App ID and App Secret are under Credentials & Basic Info.',
        'Under Add Features, add a bot.',
        'Under Permissions & Scopes, add the permissions below.',
        'Once the app is stored here, start the assistant in the settings.',
        'While the assistant runs, set the subscription mode under Events & Callbacks to a persistent connection, and add '
          + 'the event for receiving messages, im.message.receive_v1.',
        'Under Version Management & Release, make a version and ask for its release, with yourself among those who may use it.',
        `Find the bot by its name in ${platform}, and send it the code in a chat of your own.`,
      ],
      scopes: 'The last three are for reading documents, and may be left out.',
    },

    /** How much money the account of the service in use has left. */
    balance: {
      reading: 'Asking the service how much money its account has left.',
      amount: (service: string, total: string, paid: string, given: string) =>
        `${service}: ${total} left, of which ${paid} paid in and ${given} given.`,
      none: (service: string) => `${service} names no money in its account.`,
      cannotPay: 'The account cannot pay for calls: top it up on the page of the service.',
      unsupported: (service: string) => `${service} does not tell a program how much money its account has left. `
        + 'See it on the page of the service.',
      failed: (service: string) => `${service} could not be asked how much money its account has left.`,
    },

    /** The assistant in WeCom. */
    wecom: {
      lead: 'Talk to Columbina in WeCom, through a smart bot of your own. The assistant answers you alone, in a chat of your '
        + 'own with the bot. It runs no commands and changes none of your files: the files and skills that you send it are '
        + 'kept in a folder of its own. In a group, it answers whoever mentions the bot, in a conversation for each group, '
        + 'and only reads: it sees none of your files, skills or private instructions.',
      reading: 'Asking how the assistant in WeCom is doing.',
      unavailable: 'The assistant in WeCom is set up here where the application runs the service itself, and not in a replay.',
      statusFailed: 'Columbina could not find out how the assistant in WeCom is doing.',
      notRunning: 'The assistant does not run.',
      connecting: (platform: string) => `Connecting to ${platform}.`,
      reconnecting: (platform: string) => `${platform} cannot be reached just now. The assistant keeps trying.`,
      connected: (platform: string) => `Connected. The assistant answers you in ${platform}.`,
      trouble: 'Something did not go through lately.',
      pairNote: 'Whoever sends it first is the one whom the assistant answers. The code holds for 10 minutes. '
        + 'After that, a message to the bot makes a new one here.',
      pairNotHeard: 'No answer from the bot? In the admin console, see that the bot uses the API mode with a long connection, '
        + 'and not an address for callbacks.',
      noCode: 'The code was given up on after too many wrong tries. Stop the assistant and start it again for a new one.',
      stopping: 'Stopping the assistant.',
      elsewhere: 'The assistant runs elsewhere on this computer, such as in a terminal, and is stopped there.',
      withColumbina: 'While the assistant runs, closing the window keeps Columbina in the tray, where the assistant goes on '
        + 'answering. Quit from the tray to stop both. If it runs when Columbina quits, it starts again the next time.',
      writable: 'It can then make and change files in its own folder, and still runs no commands. A running assistant starts '
        + 'again for the change.',
      writableFailed: 'Whether the assistant may change files could not be changed.',
      folderFailed: 'The folder of the assistant could not be opened.',
      service: 'The assistant talks to the service that Columbina uses when the assistant starts. After another service is '
        + 'chosen, stop the assistant and start it again.',
      ended: 'The assistant stopped.',
      lastError: 'The last run of the assistant did not end well.',
      startFailed: 'The assistant could not be started.',
      stopFailed: 'The assistant could not be stopped.',
      forgetAsk: (botId: string) => `Delete the bot ${botId} on this computer? Its Secret and whom the assistant answers `
        + 'are deleted, and an assistant that runs stops first. The bot in WeCom stays as it is.',
      forgotten: 'The bot was deleted on this computer.',
      forgetFailed: 'The bot could not be deleted.',
      windowLead: 'A smart bot that you made in the admin console of WeCom. Once it uses the API mode with a long connection, '
        + 'its page shows its Bot ID and Secret.',
      botId: 'As the page of the bot shows it.',
      botIdWrong: 'A Bot ID is made of letters, digits and signs, with no blank in it.',
      secret: 'The Secret goes from this window to Columbina, which keeps it in the keyring of the system. '
        + 'The window that shows the conversation never has it.',
      secretAgain: 'The Secret that is stored is not shown. Enter it again to store the bot anew.',
      blankInSecret: 'A Secret has no blank in it. See whether what was pasted brought more along.',
      done: (botId: string) => `WeCom takes the bot ${botId}, and it is stored.`,
      inKeyring: 'The Secret is in the keyring of the system.',
      inFile: 'No keyring took the Secret. It lies in a file in the folder of Columbina, as it is and not encrypted.',
      steps: [
        'In the admin console of WeCom, make a smart bot.',
        'On the page of the bot, turn on the API mode and choose the long connection. The page then shows its Bot ID and Secret.',
        'Once the bot is stored here, start the assistant in the settings.',
        'Find the bot in WeCom, and send it the code in a chat of your own.',
      ],
    },

    /** The assistant in QQ. */
    qq: {
      lead: 'Chat with Columbina in QQ, through a bot of your own. In a chat of your own it answers you alone; put it in '
        + 'a group, and it answers whoever mentions it there, in a conversation for each group. It answers quickly and '
        + 'thinks little, changes no file and runs no command, and in a group it sees none of your files or private instructions.',
      reading: 'Asking how the assistant in QQ is doing.',
      unavailable: 'The assistant in QQ is set up here where the application runs the service itself, and not in a replay.',
      statusFailed: 'Columbina could not find out how the assistant in QQ is doing.',
      notRunning: 'The assistant does not run.',
      connecting: (platform: string) => `Connecting to ${platform}.`,
      reconnecting: (platform: string) => `${platform} cannot be reached just now. The assistant keeps trying.`,
      connected: (platform: string) => `Connected. The assistant answers you in ${platform}.`,
      trouble: 'Something did not go through lately.',
      pairNote: 'Whoever sends it first is the one whom the assistant answers in a chat of two. The code holds for 10 minutes. '
        + 'After that, a message to the bot makes a new one here.',
      pairNotHeard: 'No answer from the bot? On the open platform, see that its events come by WebSocket, and that you are '
        + 'among the people who may use it while it is not public.',
      noCode: 'The code was given up on after too many wrong tries. Stop the assistant and start it again for a new one.',
      stopping: 'Stopping the assistant.',
      elsewhere: 'The assistant runs elsewhere on this computer, such as in a terminal, and is stopped there.',
      withColumbina: 'While the assistant runs, closing the window keeps Columbina in the tray, where the assistant goes on '
        + 'answering. Quit from the tray to stop both. If it runs when Columbina quits, it starts again the next time.',
      folderFailed: 'The folder of the assistant could not be opened.',
      service: 'The assistant talks to the service that Columbina uses when the assistant starts. After another service is '
        + 'chosen, stop the assistant and start it again.',
      ended: 'The assistant stopped.',
      lastError: 'The last run of the assistant did not end well.',
      startFailed: 'The assistant could not be started.',
      stopFailed: 'The assistant could not be stopped.',
      forgetAsk: (appId: string) => `Delete the bot ${appId} on this computer? Its AppSecret and whom the assistant answers `
        + 'are deleted, and an assistant that runs stops first. The bot in QQ stays as it is.',
      forgotten: 'The bot was deleted on this computer.',
      forgetFailed: 'The bot could not be deleted.',
      windowLead: 'A bot that you made on the open platform of QQ. Its development settings show its AppID and AppSecret.',
      appId: 'As the development settings of the bot show it.',
      appIdWrong: 'An AppID is made of letters, digits and signs, with no blank in it.',
      secret: 'The AppSecret goes from this window to Columbina, which keeps it in the keyring of the system. '
        + 'The window that shows the conversation never has it.',
      secretAgain: 'The AppSecret that is stored is not shown. Enter it again to store the bot anew.',
      blankInSecret: 'An AppSecret has no blank in it. See whether what was pasted brought more along.',
      done: (appId: string) => `QQ takes the bot ${appId}, and it is stored.`,
      inKeyring: 'The AppSecret is in the keyring of the system.',
      inFile: 'No keyring took the AppSecret. It lies in a file in the folder of Columbina, as it is and not encrypted.',
      steps: [
        'On the open platform of QQ, make a bot.',
        'In its development settings, have its events come by WebSocket. They show its AppID and AppSecret.',
        'Once the bot is stored here, start the assistant in the settings.',
        'Find the bot in QQ and send it the code in a chat of your own. Put it in a group, and anyone can mention it there.',
      ],
    },

    /** The model of an assistant and how hard it thinks. */
    assistantThinking: {
      note: 'What is left empty follows what holds for all. An assistant that runs starts again for a change.',
      failed: 'The model and the thinking of the assistant could not be changed.',
    },

    music: {
      about: 'It plays on repeat, through the opening and while you work.',
      kinds: 'MP3, WAV, M4A, OGG or FLAC, up to 100 MB. The file is kept on this computer.',
      problem: {
        cannotPlay: 'This file cannot be played. Choose another one.',
        cannotOpenKept: 'The music of the application folder cannot be opened. Choose another file.',
        wrongKind: 'Choose a file of one of these kinds: MP3, WAV, M4A, OGG, FLAC.',
        wrongSize: 'Choose a file that is not empty and no larger than 100 MB.',
        noKept: 'There is no music in the application folder. Choose a file.',
      },
      unkept: {
        cannotRead: 'What was chosen earlier cannot be read. A choice made now holds for this window only.',
        thisWindowOnly: 'This file plays in this window only. It could not be kept for the next start.',
        notCleared: 'The music was removed from this window. The kept copy could not be cleared, '
          + 'and may be back after the next start.',
        choiceNotKept: 'The music of the application folder plays in this window. The choice could not be kept.',
      },
    },

    /** What the studio says: in its viewer, under its tools, and when something did not work. */
    studio: {
      emptyBin: 'This bin is empty.', mediaUsed: 'This media is used on a timeline. Remove its clips first.', binUsed: 'This bin contains media or other bins. Move them out first.', mediaEditFailed: 'The media pool could not be changed.', thumbnailFailed: 'The thumbnail could not be read.',
      edit: {
        failed: "The edit could not be completed.",
        locked: "A track affected by this edit is locked.",
        ripple_blocked: "A clip on another track prevents this ripple edit.",
        no_room: "There is not enough room on the target track.",
        no_handles: "The source does not have enough frames for this edit.",
        empty_range: "Choose a range containing at least one frame.",
        nothing_at: "There is no clip at this position.",
        no_track: "Choose a track for this media.",
        stale_revision: "The project changed during this edit; try again.",
        pending: "The edit before is still being applied; try again in a moment.",
        no_edit_point: "No two clips meet on a video track near the playhead.",
        no_edit_at_playhead: "No two clips meet at the playhead on the target tracks.",
        no_edit_chosen: "The clips chosen meet no other clip.",
        missing: "An item used by this edit is no longer in the project.",
        taken: "Another item already uses this name.",
        out_of_list: "This position is outside the track list.",
        no_stream: "This media does not have the stream required by this track.",
        wrong_kind: "Move clips to a track of the same kind.",
        wrong_track: "This track cannot hold the selected item.",
        out_of_source: "The chosen range extends beyond the source.",
        scale: "The source time could not be converted.",
        overlap: "Another clip occupies these frames.",
        cue_overlap: "Another subtitle occupies these frames.",
        copy_locked: "A copy would go on a locked track, so nothing was copied.",
        copy_no_track: "There is no track there for every copy, so nothing was copied.",
        move_no_track: "There is no track there for every clip and its linked clips, so nothing was moved.",
        subtitle_shown: "Another subtitle track is shown. A timeline shows one subtitle track at a time.",
        out_of_timeline: "A clip cannot start before the timeline.",
        in_use: "This item is still used by the project.",
        too_large: "This edit exceeds the supported size.",
        out_of_range: "A setting is outside its allowed range.",
        zero_rate: "The frame rate must be greater than zero.",
        negative_duration: "The duration cannot be negative.",
        reserved: "This identifier is reserved.",
        ids_exhausted: "The project cannot create more identifiers.",
        dollar: "This item reference could not be resolved.",
        no_id: "The edit is missing an item reference.",
        not_absolute: "Choose a file with an absolute path.",
        not_a_proxy_path: "The proxy path is outside the project cache.",
        has_transition: "Remove the existing transition before this edit.",
        not_adjacent: "The clips must meet at the same edit point.",
        cut_taken: "This edit point already has a transition.",
        too_long: "The transition is longer than the available clip.",
        source: "The source cannot be changed by this edit.",
        cycle: "A bin cannot be placed inside itself.",
        fade_at_transition: "A fade and a transition cannot share the same clip edge.",
        not_empty: "The bin still contains items.",
        no_port: "That node has no such input or output.",
        input_taken: "That input already has a link.",
        loop: "This link would make the grade loop back on itself.",
        grouped: "A clip is still in this group. Take it out of the group first.",
        ungrouped: "This clip is not in a group.",
        params: "These settings do not fit this kind of node.",
        wrong_node: "This cannot be done with the chosen node.",
        camera_raw: "The color space of camera raw media comes from its camera and cannot be set.",
        node_locked: "This node is locked. Unlock it to change its settings.",
        unknown_space: "That color space or transform is not one that the engine knows.",
        inapplicable: "This setting does not apply to the current color management.",
        color_settings: "These color settings could not be read.",
      },
      timeline: { empty: 'Choose or create a timeline to begin editing.' },
      settings: {
        applied: 'Changes are made when you click Save, as one step that can be undone. Alt+click Save to keep this window open.',
        changedElsewhere: 'The settings of the project were changed elsewhere. What you have not changed here shows the new values.',
        folderAbsent: 'This folder does not exist yet.',
        folderFailed: 'The folder could not be opened.',
        noProject: 'Open the project in the studio to change its settings.',
        regrade: 'Changing this after grading has begun changes how every grade looks.',
        displayHdr: 'This is kept on this computer for every project. The viewers show HDR only where HDR is on in the display settings of Windows.',
        hdrOff: 'Windows has HDR off for the display of the viewer, so it shows SDR.',
        notYet: 'Not in effect yet: kept with the project, it does not change the picture in this version.',
        lutFolders: 'A LUT is looked for in these folders, in this order.',
      },
      loadSource: 'Double-click media in the media pool to see it here.',
      newTimeline: 'The timeline takes the size of the first video in the media pool, and its frame rate unless you choose another. It starts with one video track and one audio track. Once it has content, its frame rate is locked.',
      newTimelineFailed: 'The timeline could not be created.',
      inspectorEmpty: 'Select a clip on the timeline.',
      inspector: {
        longest: (frames: number) => `The longest that fits is ${frames === 1 ? '1 frame' : `${frames} frames`}.`,
        rate: 'The frame rate is locked because the timeline has content. A new timeline can use another rate.',
        rateRefused: 'At this frame rate, a clip would run past the end of its media.',
        fadeAtTransition: 'A transition is at this edge, so a fade cannot be set here.',
        locked: 'These clips are on locked tracks. Unlock the track to change them.',
        speedMedia: 'Only clips of media can change speed.',
        speedRange: 'The speed is from 1% to 10000%.',
        library: 'Drag a transition onto an edit point on the timeline, or a title or a solid color onto a video track, where it overwrites 5 seconds.',
        addCue: 'Double-click empty space on a subtitle track to add a subtitle at the playhead, or right-click it and choose Add subtitle to add one where you clicked.',
      },
      render: {
        marks: 'Mark an In and an Out point on the timeline to export only the part between them.',
        codec: 'H.264 and HEVC are encoded on the graphics card. Where it has no encoder for them, ProRes is written into a .mov file instead.',
        subtitles: 'Each subtitle track checked, shown in the viewer or not, is saved beside the film as an SRT file.',
        running: 'An export is running. Wait for it to finish, or cancel it.',
        noTimeline: 'There is no timeline to export yet.',
        done: 'The export is finished.',
        failed: 'The export failed.',
        cancelled: 'The export was cancelled, and nothing was written.',
        stopped: 'The engine of the studio stopped during the export.',
        fallback: 'This computer has no hardware encoder for H.264 or HEVC, so the film was written as ProRes into a .mov file.',
        refused: {
          busy: 'Another export of this project is running, maybe one that the agent started. Try again when it has finished.',
          exists: 'Files that this export writes beside the film are there already, such as an SRT or a .mov file. Choose another name, or move them away.',
          part_exists: 'A partial file of an export that did not finish is there. Delete it, or choose another name.',
          source: 'That file is media of this project, and is not written over. Choose another name.',
          invalid: 'The engine did not take these settings.',
          other: 'The export could not start.',
        },
      },
      migration: {
        duck: "This old project lowered music while someone spoke. Add that back?",
        duckFailed: "Ducking was not completed. You can try again.",
        /** `names` are those of the first few of `count`; the rest are counted. */
        done: (names: readonly string[], count: number) => (count === 1
          ? `An old project became a timeline: ${names[0]}. Its files are kept as they were.`
          : `${count} old projects became timelines: ${names.join(', ')}${count > names.length ? `, and ${count - names.length} more` : ''}. Their files are kept as they were.`),
        failed: (count: number) => (count === 1
          ? 'An old project could not be migrated. Its files are untouched.'
          : `${count} old projects could not be migrated. Their files are untouched.`),
        checkFailed: 'The old projects in this folder could not be looked at.',
      },
      cutPage: {
        smartInsert: 'Inserts the source at the edit point nearest the playhead, which the timeline marks.',
        append: 'Puts the source after the last clip of the timeline.',
        rippleOverwrite: 'Replaces the clip at the playhead with the source, and moves what follows by the difference in length.',
        placeOnTop: 'Puts the source at the playhead on the first free track above.',
        sourceOverwrite: 'Puts the source on the track above, in sync by timecode. The studio does not read the timecode of media yet, so there is nothing to line up by: as in DaVinci Resolve without overlapping timecode, this edit does nothing.',
        transition: 'Adds the transition at the edit point nearest the playhead.',
        emptyBin: 'This bin has no media for the source tape.',
        noTimeline: 'Create a timeline on the Edit page to begin.',
      },
      colorPage: {
        noClip: 'There is no clip at the playhead.',
        noClips: 'There are no clips on the video tracks shown.',
        noNodes: 'This grade has no nodes yet.',
        incomplete: 'The grade is off: a node is not connected.',
        gallery: {
          empty: 'There are no stills in this album. Press Ctrl+Alt+G to grab one.',
          noneFound: 'No still here has a label like this.',
          failed: 'The Gallery could not be read.',
          deleted: 'Deleted. Press Ctrl+Z in the Gallery to bring them back.',
          confirmRemoveAlbum: 'The album will be removed with all its stills. This cannot be undone.',
          /** Why the engine refused a change of the Gallery, or of grades from it, by its reason. */
          refusals: {
            last_album: 'The last album cannot be removed.',
            exists: 'Files of these names are there already. Choose another name or place.',
            no_grade: 'This still holds no grade for the mode of the node editor.',
            not_a_still: 'This file cannot be read as a still.',
            missing: 'This still or album is not in the Gallery any more.',
            link: 'The folder of the Gallery is a link, so it is not used.',
            gallery: 'The folder of the Gallery could not be read or written.',
            write: 'The stills could not be written there.',
            not_loaded: 'Open a timeline in the viewer to grab a still.',
            draw: 'The frame could not be read from the viewer.',
          },
        },
        lutsFailed: 'The LUTs could not be listed.',
        lutsTruncated: 'There are too many LUTs to list them all.',
        noLuts: 'No LUTs were found. Put .cube files in the folder of the project or in the LUT library, then refresh.',
        noLutsHere: 'There are no LUTs here.',
        noFavorites: 'Point at a LUT and click its star to add it to the favorites.',
        lutShadowed: 'A LUT of the same name in the folder of the project is used instead.',
        lutBroken: 'This LUT cannot be read, so it cannot be used.',
        missingLuts: 'These LUTs could not be read. The nodes that use them are graded without them.',
        palette: 'The controls of this palette come in a later version.',
        noCorrector: 'Choose a corrector node in the node editor to adjust it.',
        autoFlat: 'Auto Color found too little range in this frame to balance it.',
        /** The Tracker palette (pp. 3229–3238). */
        tracker: {
          noWindow: 'Select a window in the Window palette to track it.',
          tracking: (frame: number, done: number, total: number) => `Tracking frame ${frame} (${done} of ${total}).`,
          done: (frames: number) => `Tracked ${frames} frames.`,
          stopped: 'Tracking stopped. The frames already tracked are kept.',
          changed: 'Tracking stopped because the project changed.',
          copied: 'Track data copied. Select another window of this node and paste it.',
          failed: {
            no_features: 'Nothing inside the window can be tracked. Place it over a feature with detail.',
            no_points: 'Add a tracker first.',
            missing_frame: 'A frame could not be read. The frames already tracked are kept.',
            cannot_read: 'The media of this clip could not be read.',
            untrackable: 'This window cannot be tracked here. Only the windows of a clip of moving pictures can be.',
            frame: 'The playhead is outside the clip.',
            busy: 'Another track is going on.',
            untracked: 'Only the windows of a clip of moving pictures can be keyframed here.',
            nothing_to_paste: 'The copied window has no track data.',
            stale_revision: 'The project changed. Try again.',
            other: 'Tracking failed.',
          },
        },
        windowControls: {
          refused: 'Window controls are unavailable for this frame. Select the window again to retry.',
          /** What a Curve's selected points take (RM20 p. 3195). */
          selectedPoints: 'Drag inside the box to move them, a corner to scale them, a side to stretch them, or just beside a corner to rotate them. Backspace deletes them; Esc deselects them.',
          /** Why a Polygon point was not added or deleted; nothing was changed. */
          topology: {
            maximum: 'A polygon window holds at most {most} points, so no point was added.',
            minimum: 'A polygon window keeps at least {least} points, so the point was not deleted.',
            minimumCurve: 'A closed curve window keeps at least {least} points, so the point was not deleted.',
            minimumCurvePoints: 'A closed curve window keeps at least {least} points, so the selected points were not deleted.',
            unsupported: 'This point cannot be added or deleted.',
            jump: 'No point was added: the {axis} of {points} mixes a held jump with a continuous change ({frames}), so a new point would change the shape between keyframes. Give those keyframes the same times and interpolation (for example Linear), then add the point. The animation was not changed.',
            timing: 'No point was added: the {axis} of {points} uses different easing curves ({frames}), so a new point would change the shape between keyframes. Give those fields the same easing, then add the point. The animation was not changed.',
            overshoot: 'No point was added: the {axis} curve of {points} overshoots the coordinate range and is clipped ({frames}). Reduce that overshoot, or use an interpolation without overshoot, then add the point. The animation was not changed.',
            range: 'No point was added: the new point would lie outside the coordinate range ({frames}). Adjust the window shape or click elsewhere on this edge. The animation was not changed.',
            encoding: 'No point was added: splitting the easing of the {axis} of {points} ({frames}) needs a keyframe handle that keyframes cannot store. Ask the agent to align those keyframes, or adjust that easing, then add the point. The animation was not changed.',
          },
        },
        shotMatch: {
          done: (count: number) => `Matched ${count} ${count === 1 ? 'clip' : 'clips'}.`,
          changed: 'A selected clip or the reference is no longer available. Choose the clips again.',
          wrongNode: 'Choose a corrector node in each clip to match.', locked: 'Unlock the selected nodes before matching.',
          animated: 'The existing Shot Match has keyframes. Reset Shot Match before matching again.',
          cleanup: 'Matching did not finish, and the temporary nodes could not be safely restored. Added default nodes may remain; your other edits were preserved.',
          graded: 'Some clips already had adjustments; these can affect the match.',
          space: 'Shot Match is designed for Rec.709 Gamma 2.4 and measures each node in the timeline working space.',
        },
        picker: 'Click on the viewer to take the color of a pixel, or drag to take the colors of a range.',
        qualifierEmpty: 'Choose the Sample Eyedropper and click or drag on the viewer to pick the colors to key.',
        noKeyNode: 'Choose a corrector or a Key Mixer in the node editor to adjust its key.',
        lutGenerated: (path: string, ignored: number) => (ignored === 0 ? `The LUT was saved as ${path}.`
          : `The LUT was saved as ${path}. ${ignored === 1 ? 'A node' : `${ignored} nodes`} with qualifiers, windows or filters ${ignored === 1 ? 'was' : 'were'} left out of it.`),
        lutFailed: (message: string) => `The LUT could not be generated. ${message}`,
        keyed: 'This setting has keyframes: a change goes into the keyframe at the playhead or to its left.',
        /** Why a window preset was not saved, recalled or deleted. */
        windowPresets: {
          nameTaken: 'A preset named “{name}” already exists. Choose another name.',
          noWindows: 'This node has no window to save as a preset.',
          missing: 'This preset is no longer in the library.',
          library: 'The window presets could not be read or written.',
        },
        scopes: {
          preparing: 'The scope is being prepared. The first time takes a while.',
          empty: 'There is nothing in the viewer to measure.',
          failed: 'The scope could not be shown.',
          tooMany: 'No more scopes can be open at once.',
          unavailable: 'This engine has no viewer, so it has no scopes.',
          floating: 'The scopes are shown in their floating window.',
          browser: 'The scopes appear only in the desktop app.',
        },
      },
      windowsOnly: 'The editing studio runs only on Windows for now. macOS will come later.',
      browser: 'The viewers appear only in the desktop app.',
      opening: 'The project is being opened in the studio.',
      takingOver: 'The project is being taken over from the agent.',
      stopped: 'The engine of the studio has stopped.',
      missing: 'The engine of the studio is not installed.',
      install: 'Download Columbina again to get its engine. In a copy of the source code, build the engine first.',
      notOpened: 'The studio could not be opened.',
      unsupported: 'The preview cannot be shown on this system yet.',
      headless: 'The project is open in an engine without a window, so the preview cannot be shown.',
      noMedia: 'There is no media yet. Import some to begin.',
      load: 'Double-click media on the Media page to see it here.',
      inexact: 'A frame near the playhead is shown until the frame itself is ready.',
      importFailed: 'The media could not be imported.',
      loadFailed: 'This media could not be opened.',
      failed: 'The engine did not do what was asked.',
      engineError: 'The engine of the studio reported a problem.',
      proxyFailed: 'The proxy could not be made.',
      useProxies: 'Proxies are copies of the media that are quick to scrub. They are made automatically.',
      source: {
        proxy: 'The picture is read from the proxy.',
        original: 'The picture is read from the media itself.',
        mixed: 'The picture of some clips is read from their proxies, and of the others from the media itself.',
      },
      sound: {
        on: 'The sound plays.',
        off: 'No sound is played while paused.',
        offSpeed: 'The sound plays at normal speed only.',
        unavailable: 'No sound device can be used, so the picture plays alone.',
        none: 'This media has no sound.',
      },
    },
  },
};

export type Messages = typeof en;
export type { Weekday };
