/** Protocol v1. Unknown event types and additional fields are intentionally accepted. */
export type RpcId = string | number;
export type JsonRecord = Record<string, unknown>;

export interface RpcRequest {
  jsonrpc: '2.0';
  id: RpcId;
  method: string;
  params?: JsonRecord;
}

export interface RpcNotification {
  jsonrpc: '2.0';
  method: string;
  params?: JsonRecord;
}

export interface RpcSuccess {
  jsonrpc: '2.0';
  id: RpcId;
  result: unknown;
}

export interface RpcFailure {
  jsonrpc: '2.0';
  id: RpcId | null;
  error: { code: number; message: string; data?: unknown };
}

export type RpcMessage = RpcRequest | RpcNotification | RpcSuccess | RpcFailure;

export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
}

export interface ContentBlock {
  type: string;
  text?: string;
  call_id?: string;
  name?: string;
  arguments?: string;
  replay_state?: unknown;
  [field: string]: unknown;
}

export interface Message {
  role: string;
  content: ContentBlock[];
  call_id?: string;
  is_error?: boolean;
  [field: string]: unknown;
}

export interface SessionEvent {
  type: string;
  seq: number;
  time: number;
  data: JsonRecord;
  ignorable?: boolean;
  surface_op?: 'append' | { op: string; start_seq: number; end_seq: number };
  source_event_seqs?: number[];
  [field: string]: unknown;
}

export interface ToolDeclaration {
  name: string;
  description: string;
  parameters: JsonRecord;
  [field: string]: unknown;
}

export interface InboxEntry {
  message: Message;
  source: string;
}

export interface Hello {
  protocol: number;
  server: { name: string; version: string };
  cwd: string;
  commands: { name: string; summary: string }[];
}

/** A skill of the session that is open, as `session/skills` lists it. */
export interface SkillSummary {
  name: string;
  /** What it is for, on one line. */
  description: string;
  /** Whether a person may hand it to the agent, with `/skill NAME`. */
  user_invocable: boolean;
  /** Whether the model loads it by itself when a task calls for it. */
  model_invocable: boolean;
}

export interface SessionSummary {
  id: string;
  modified_ms: number;
  first_input: string | null;
}

export interface SessionStatus {
  session_id: string;
  status: string;
  detail: string | null;
  model: { provider: string; model: string; context_window: number | null };
  sandbox: { mode: string; strength: string } | null;
  approval: string | null;
}

export interface ApprovalRequest {
  session_id: string;
  call_id: string;
  tool: string;
  summary: string;
  reason: string;
  target: string | null;
  standing: string | null;
}

export interface StreamChunk {
  type: string;
  index?: number;
  kind?: string;
  call_id?: string | null;
  name?: string | null;
  text?: string;
  arguments?: string;
  usage?: Usage;
  reason?: string | null;
  failure?: JsonRecord | null;
  replay_state?: unknown;
  [field: string]: unknown;
}

export interface StreamNotification {
  session_id: string;
  turn: number;
  step: number;
  chunk: StreamChunk;
}
