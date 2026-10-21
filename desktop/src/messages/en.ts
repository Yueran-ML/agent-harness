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
import { splitDuration } from './duration';

function duration(seconds: number): string {
  const time = splitDuration(seconds);
  if (time.hours > 0) return time.minutes > 0 ? `${time.hours} h ${time.minutes} min` : `${time.hours} h`;
  if (time.minutes > 0) return time.seconds > 0 ? `${time.minutes} min ${time.seconds} s` : `${time.minutes} min`;
  return `${time.seconds} s`;
}

// A turn that reached a limit stops between two steps, whichever limit it was.
const afterLimit = 'It stopped before its next step. Tell it to go on if it should.';

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
    newTask: 'New task',
    searchTasks: 'Search tasks',
    tasks: 'Tasks',
    openProject: 'Open a project',
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
      checkAgain: 'Check again',
      copyCode: 'Copy the code',
      pair: (platform: string) => `Send the bot this code in ${platform}`,
      platform: 'Platform',
      platforms: { feishu: 'Feishu', lark: 'Lark' },
      choose: (platform: string) => `Choose ${platform}`,
      appId: 'App ID',
      appSecret: 'App Secret',
      store: 'Check and store',
      storing: 'Checking',
      steps: 'In the developer console',
      next: 'Next steps',
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
    waiting: 'Waiting',
    summary: 'Earlier messages, summed up',

    placeholderIdle: 'Give Columbina something to do',
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

    confirmFullAccess: 'Allow full access?',
    confirmWrite: 'Let Columbina change files?',
    allowFullAccess: 'Allow full access',
    allowWrite: 'Allow changes',
    openLinkTitle: 'Open this link?',
    openLink: 'Open it',
  },

  /** Sentences that explain: under a field, in an empty place, while something is waited for. */
  hint: {
    keysIdle: 'Enter sends. Shift+Enter starts a new line. A slash opens the commands.',
    keysRunning: 'What you send now is read at the next step. Esc, pressed twice, stops the turn.',
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
      picture: 'The model gets the file, and does not see the picture.',
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
      withColumbina: 'The assistant runs only while Columbina is open. If it runs when you close Columbina, it starts again the '
        + 'next time.',
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
  },
};

export type Messages = typeof en;
