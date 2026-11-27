export interface AppInfo {
  version: string;
  dataDir: string;
  dbPath: string;
  gatePort: number;
  platform: string;
}

export interface AppSettings {
  maxConcurrentRuns: number;
  approvalTimeoutSecs: number;
  transcriptRetentionDays: number;
  notify: boolean;
  proxyEnabled: boolean;
  runMaxMinutes: number;
  hookStrict: boolean;
  nonClaudeRunApproval: boolean;
  sandboxEnabled: boolean;
}

export interface DetectedCli {
  name: string;
  command: string;
  adapter: string;
  version: string | null;
  installed: boolean;
  recommended: boolean;
  installable: boolean;
  installHint: string | null;
  /** 适配器真实能力，用于能力矩阵展示。 */
  capabilities?: DetectedCliCapabilities;
}

export interface DetectedCliCapabilities {
  streaming?: boolean;
  streamUsage?: boolean;
  toolApproval?: boolean;
  resume?: boolean;
  budgetEnforcement?: boolean;
  /** provider：CLI 结构化上报；estimate：按输出文本估算。 */
  usageSource?: string;
}

export interface CliVersionInfo {
  name: string;
  latestVersion: string | null;
}

export interface SandboxProbeReport {
  ok: boolean;
  profile: string;
  workRead: boolean;
  workWrite: boolean;
  existingRead: boolean;
  outsideReadBlocked: boolean;
  userReadBlocked: boolean;
  outsideWriteBlocked: boolean;
  homeRedirected: boolean;
  relayEcho: boolean;
  stdioForwarded: boolean;
  hostExitCode: number;
  detail: string;
  elapsedMs: number;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  kind: string;
  description?: string | null;
  /** 项目目录是否为 Git 仓库（后端探测，不入库）。 */
  isGit: boolean;
  gitReason?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface GitState {
  isGit: boolean;
  reason?: string | null;
}

export interface BackupFile {
  name: string;
  path: string;
  size: number;
  createdAt: number;
}

export interface Agent {
  id: string;
  name: string;
  kind: string;
  adapter: string;
  command?: string | null;
  model?: string | null;
  providerId?: string | null;
  status: string;
  version?: string | null;
  capabilities: unknown;
  lastSeenAt?: number | null;
  createdAt: number;
}

export type ProviderProtocol = 'anthropic' | 'openai';

export interface Provider {
  id: string;
  name: string;
  protocol: ProviderProtocol | string;
  baseUrl: string;
  model?: string | null;
  models: string[];
  keyHint?: string | null;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface ProviderTestResult {
  ok: boolean;
  status: number;
  latencyMs: number;
  message: string;
}

export type ExtensionKind = 'mcp' | 'hook' | 'skill' | 'plugin';

export interface ExtensionItem {
  id: string;
  kind: ExtensionKind | string;
  name: string;
  description?: string | null;
  config: Record<string, unknown>;
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
}

/** 扩展健康检查结果：ok 可用；warn 存疑；error 不可用。 */
export interface ExtensionHealth {
  id: string;
  name: string;
  kind: string;
  enabled: boolean;
  status: 'ok' | 'warn' | 'error';
  message: string;
}

export interface Team {
  id: string;
  projectId: string;
  projectName?: string | null;
  name: string;
  templateKey?: string | null;
  description?: string | null;
  seatsCount: number;
  assignedCount: number;
  createdAt: number;
}

export interface TeamTemplateRole {
  roleKey: string;
  roleName: string;
  responsibility: string;
  capability: string;
  gate: boolean;
}

export interface TeamTemplate {
  key: string;
  name: string;
  description: string;
  icon: string;
  roles: TeamTemplateRole[];
}

export interface Seat {
  id: string;
  teamId: string;
  roleKey: string;
  roleName: string;
  responsibility?: string | null;
  agentId?: string | null;
  agentName?: string | null;
  orderIndex: number;
  gate: boolean;
}

export type TaskStatus =
  | 'inbox'
  | 'queued'
  | 'running'
  | 'waiting_approval'
  | 'paused'
  | 'completed'
  | 'blocked'
  | 'cancelled';

export interface Task {
  id: string;
  projectId: string;
  projectName?: string | null;
  teamId?: string | null;
  teamName?: string | null;
  /** 父任务：子任务共享项目与团队，但独立运行与计费。 */
  parentId?: string | null;
  title: string;
  description?: string | null;
  acceptance?: string | null;
  model?: string | null;
  reasoning?: string | null;
  /** 任务授权：ask 逐次确认，auto 任务内自动。 */
  mandate: string;
  /** 隔离工作区路径；为空时使用项目目录。 */
  workDir?: string | null;
  /** 隔离工作区仍有未提交或未应用的改动，等待审查。 */
  reviewPending?: boolean;
  status: TaskStatus;
  priority: number;
  assigneeSeatId?: string | null;
  stageIndex: number;
  stageKey?: string | null;
  actualTokens: number;
  actualCostMicros: number;
  createdAt: number;
  startedAt?: number | null;
  finishedAt?: number | null;
}

/** 任务分页结果：items 为当前页，total/statusCounts 为过滤后的全量统计。 */
export interface TaskPage {
  items: Task[];
  total: number;
  statusCounts: Record<string, number>;
  reviewCount: number;
  hasMore: boolean;
  nextCursor?: string | null;
}

/** 任务附件：文件已复制到应用数据目录。 */
export interface Attachment {
  id: string;
  taskId: string;
  name: string;
  path: string;
  size: number;
  createdAt: number;
}

/** 任务依赖：被依赖任务完成后才允许运行。 */
export interface TaskDependency {
  taskId: string;
  dependsOnId: string;
  title: string;
  status: string;
}

/** 验收清单条目。 */
export interface ChecklistItem {
  id: string;
  text: string;
  done: boolean;
}

export interface Run {
  id: string;
  taskId: string;
  seatId?: string | null;
  agentId?: string | null;
  agentName?: string | null;
  seatName?: string | null;
  stageKey?: string | null;
  status: string;
  externalSessionId?: string | null;
  isolation: string;
  isolationReason?: string | null;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
  error?: string | null;
  startedAt: number;
  endedAt?: number | null;
}

export interface ToolCall {
  id: string;
  runId: string;
  toolName: string;
  input: unknown;
  output?: unknown;
  status: string;
  riskLevel?: string | null;
  createdAt: number;
  finishedAt?: number | null;
}

export interface ReviewFile {
  path: string;
  code: string;
  status: string;
  additions: number;
  deletions: number;
  binary: boolean;
}

export interface ReviewSnapshot {
  available: boolean;
  reason?: string | null;
  workDir?: string | null;
  repoRoot?: string | null;
  files: ReviewFile[];
  additions: number;
  deletions: number;
  pendingCommits: number;
  canApply: boolean;
  truncated: boolean;
}

/** 隔离工作区的分支信息。 */
export interface ReviewBranchInfo {
  available: boolean;
  reason?: string | null;
  current?: string | null;
  detached: boolean;
  branches: string[];
}

/** 运行前快照文件。 */
export interface SnapshotItem {
  path: string;
  name: string;
  createdAt: number;
  size: number;
}

/** PR 创建结果：created 已创建；manual 需手动执行；blocked 缺少前置条件。 */
export interface PrOutcome {
  mode: 'created' | 'manual' | 'blocked';
  url?: string | null;
  command: string;
  message?: string | null;
}

export interface ReviewDiff {
  path: string;
  patch: string;
  truncated: boolean;
  binary: boolean;
}

export interface Message {
  id: string;
  taskId?: string | null;
  runId?: string | null;
  seatId?: string | null;
  agentId?: string | null;
  seatName?: string | null;
  agentName?: string | null;
  kind: string;
  role?: string | null;
  content: string;
  metadata?: unknown;
  createdAt: number;
}

export interface UsageSummary {
  todayTokens: number;
  totalTokens: number;
  todayCostMicros: number;
  totalCostMicros: number;
  cacheHitPct: number;
  rounds: number;
  todayRounds: number;
}

export interface UsagePoint {
  date: string;
  tokens: number;
  costMicros: number;
}

export interface ScopedUsage {
  key: string;
  name: string;
  tokens: number;
  costMicros: number;
  rounds: number;
}

export interface AgentStat {
  id: string;
  name: string;
  adapter: string;
  status: string;
  tokens: number;
  todayTokens: number;
  costMicros: number;
  rounds: number;
}

export interface DashboardSummary {
  usage: UsageSummary;
  pendingApprovals: number;
  /** 待审查的隔离工作区任务数。 */
  pendingReviews: number;
  agents: AgentStat[];
  teams: Team[];
  series: UsagePoint[];
}

export interface Budget {
  id: string;
  scopeType: string;
  scopeId?: string | null;
  scopeName: string;
  unit: string;
  period: string;
  limitValue: number;
  warnPct: number;
  hardStop: boolean;
  enabled: boolean;
  spent: number;
  reserved: number;
  pct: number;
  blocked: boolean;
  createdAt: number;
}

export interface Approval {
  id: string;
  taskId?: string | null;
  taskTitle?: string | null;
  runId?: string | null;
  type: string;
  riskLevel?: string | null;
  title: string;
  detail: unknown;
  status: string;
  requestedBy?: string | null;
  decidedBy?: string | null;
  decisionNote?: string | null;
  createdAt: number;
  decidedAt?: number | null;
}

export interface ApprovalRule {
  id: string;
  scopeType: string;
  scopeId?: string | null;
  match: unknown;
  action: string;
  priority: number;
  enabled: boolean;
  createdAt: number;
}

export interface EventItem {
  id: string;
  seq: number;
  projectId?: string | null;
  taskId?: string | null;
  runId?: string | null;
  type: string;
  actor?: string | null;
  payload: unknown;
  createdAt: number;
}

export interface RunDeltaPayload {
  runId: string;
  taskId?: string;
  kind: 'message' | 'tool_call' | 'tool_result' | 'usage' | 'warning' | 'raw';
  warning?: string;
  message?: Message;
  usage?: {
    inputTokens: number;
    outputTokens: number;
    cacheReadTokens?: number;
    cacheWriteTokens?: number;
    costMicros: number;
  };
  toolCall?: { id: string; toolName: string; input?: unknown; status?: string };
  toolCallId?: string;
  line?: string;
}
