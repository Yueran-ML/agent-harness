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
      steps: (limit: number) => ({
        what: `This turn reached its limit of ${limit} ${limit === 1 ? 'step' : 'steps'}.`,
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
};

export type Messages = typeof en;
