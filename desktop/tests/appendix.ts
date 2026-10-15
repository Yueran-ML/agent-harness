/**
 * The codes that the design of the interface lists as those to be put into
 * plain words, as its first appendix has them (`desktop_design_2026-10-12.md`).
 * The tests go through every one of them.
 */

/** Why a request to the model failed: `failure.code`. */
export const modelFailureCodes = [
  'AUTH', 'QUOTA', 'RATE_LIMIT', 'CONTEXT_WINDOW_EXCEEDED', 'CONTENT_FILTER', 'INVALID_REQUEST',
  'SERVER', 'TIMEOUT', 'TRANSPORT', 'STREAM_CLOSED', 'MALFORMED_RESPONSE', 'EMPTY_RESPONSE', 'INTERNAL',
] as const;

/** Why a tool call did not succeed, as the pipeline says it: `error.code` of a result. */
export const toolPipelineCodes = [
  'UNKNOWN_TOOL', 'INVALID_ARGS', 'APPROVAL_REJECTED', 'DENIED', 'TOOL_CANCELLED', 'TOOL_TIMEOUT',
  'TOOL_ERROR', 'ABORTED_BEFORE_DISPATCH', 'RESULT_BLOCKED', 'SANDBOX_UNAVAILABLE', 'COMMAND_TOO_LONG',
] as const;

/** Why a tool call did not succeed, as the tools that work on files say it. */
export const toolFileCodes = [
  'NOT_FOUND', 'NOT_A_FILE', 'NOT_A_DIRECTORY', 'NOT_TEXT', 'TOO_LARGE', 'NOT_READ',
  'CHANGED_SINCE_READ', 'NO_MATCH', 'AMBIGUOUS', 'NO_CHANGE', 'READ_ONLY', 'PROTECTED',
  'INVALID_PATH', 'INVALID_PATTERN', 'CHECK_FAILED', 'MOVED', 'SEARCH_FAILED', 'SEARCH_TIMEOUT',
] as const;

export const toolCodes = [...toolPipelineCodes, ...toolFileCodes] as const;

/** How a turn ended: `reason` of `turn/end`. */
export const turnEndReasons = [
  'completed', 'aborted', 'blocked', 'error', 'max-tokens', 'interrupted', 'forked',
] as const;

/**
 * Who did not let a turn go on: `code` of a `turn/end` whose reason is `blocked`.
 * These are the codes of the program itself.
 * A plugin may give a code of its own.
 */
export const turnBlockedCodes = ['STEP_BUDGET', 'TOKEN_BUDGET', 'TIME_BUDGET', 'HOOK', 'NO_DECISION'] as const;

/** What a session may do, as `session.status` says it. */
export const sandboxModes = ['read-only', 'workspace-write', 'danger-full-access'] as const;
export const sandboxStrengths = ['none', 'partial', 'full'] as const;
export const approvalPolicies = ['ask', 'never'] as const;
