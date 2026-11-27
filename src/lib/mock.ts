import type {
  Agent,
  AgentStat,
  AppInfo,
  AppSettings,
  Approval,
  ApprovalRule,
  Attachment,
  BackupFile,
  Budget,
  ChecklistItem,
  DashboardSummary,
  DetectedCli,
  EventItem,
  ExtensionItem,
  Message,
  Project,
  Provider,
  ReviewDiff,
  ReviewFile,
  ReviewSnapshot,
  Run,
  ScopedUsage,
  Seat,
  Task,
  TaskDependency,
  Team,
  TeamTemplate,
  ToolCall,
  UsagePoint,
  UsageSummary,
} from './types';
import { needsYouTask } from './taskStage';
import { canReassignStatus, isActiveStatus } from './taskStatus';
import { emitLocal } from './events';

const demoLang = (() => {
  try {
    return localStorage.getItem('lumen.lang') === 'en' ? 'en' : 'zh';
  } catch {
    return 'zh';
  }
})();

const DEMO_EN: Record<string, string> = {
  '软件研发组': 'Software Team',
  '从需求澄清、架构设计、编码实现到测试与交付的完整研发流水线':
    'A complete pipeline from discovery and architecture through implementation, testing, and delivery',
  '产品经理': 'Product Manager',
  '目标拆解、用户故事、功能边界与验收标准':
    'Goals, user stories, scope, and acceptance criteria',
  '系统架构师': 'System Architect',
  '技术方案、模块划分、接口契约与风险评估':
    'Technical design, module boundaries, interface contracts, and risk assessment',
  '研发工程师': 'Software Engineer',
  '按架构落地代码、补充测试、修复问题':
    'Implement code against the architecture, add tests, and fix issues',
  '测试工程师': 'QA Engineer',
  '验证验收标准、回归风险、缺陷复现与报告':
    'Verify acceptance criteria, regression risk, defect reproduction, and reporting',
  '交付总监': 'Delivery Lead',
  '汇总产物、检查交付完整性、输出最终说明':
    'Collect artifacts, check delivery completeness, and write the final summary',
  '办公协作组': 'Office Collaboration Team',
  '面向文档、资料与日常办公产出的轻量协作团队':
    'A lightweight team for documents, research, and everyday office deliverables',
  '需求整理': 'Requirements Editor',
  '澄清目标、受众、交付格式与截止时间':
    'Clarify goals, audience, delivery format, and deadlines',
  '资料研究员': 'Research Analyst',
  '收集与核对资料、提炼要点、标注来源':
    'Collect and verify sources, extract key points, and cite references',
  '文档工程师': 'Documentation Engineer',
  '按目标撰写、整理文档、表格或演示材料':
    'Write and organize documents, spreadsheets, or presentations against the goals',
  '审核交付': 'Delivery Reviewer',
  '检查准确性、完整性与格式，输出交付说明':
    'Check accuracy, completeness, and formatting; write the delivery notes',
  '量化研究组': 'Quant Research Team',
  '围绕研究假设组织数据、验证过程与最终结论的协作团队':
    'A collaborative team that organizes data, validation, and conclusions around research hypotheses',
  '投研需求': 'Research Requirements',
  '明确研究标的、时间窗口、数据来源与输出格式':
    'Define the target, time window, data sources, and output format',
  '基本面分析师': 'Fundamentals Analyst',
  '财报与行业数据、关键指标与估值假设':
    'Financial reports, industry data, key metrics, and valuation assumptions',
  '量价分析师': 'Market Structure Analyst',
  '价格、成交量与市场结构，识别趋势和关键区间':
    'Price, volume, and market structure; trends and key ranges',
  '风控专员': 'Risk Controller',
  '敞口测算、情景分析、仓位与止损约束':
    'Exposure measurement, scenario analysis, position sizing, and stop-loss limits',
  '研究主管': 'Research Lead',
  '汇总分歧、校验证据、形成研究结论':
    'Consolidates disagreements, checks evidence, and concludes',
  'Lumen 工作台': 'Lumen Workspace',
  '多 Agent 协同工作台本体，包含编排、审批与用量统计。':
    'The multi-agent workbench itself, including orchestration, approvals, and usage tracking.',
  '实现登录与权限体系': 'Implement sign-in and permissions',
  '为桌面端补充账号登录、会话保持与基于角色的权限控制，要求与现有 Rust 后端打通。':
    'Add account sign-in, session persistence, and role-based access control to the desktop app, integrated with the existing Rust backend.',
  '1. 支持账号密码登录与手机号验证码登录\n2. 会话过期自动续期\n3. 无权限操作返回明确错误':
    '1. Support password and SMS-code sign-in\n2. Renew sessions automatically when they expire\n3. Return clear errors for unauthorized actions',
  '整理 Q4 运营复盘报告': 'Prepare the Q4 operations review',
  '汇总第四季度关键指标，输出结构化复盘文档与改进项。':
    'Summarize Q4 key metrics and produce a structured review document with improvement items.',
  '包含目标达成率、问题清单与下季度行动项':
    'Cover goal attainment, the issue list, and next-quarter action items',
  '读取 128 行，识别出 4 个角色校验分支。':
    'Read 128 lines; identified 4 role-check branches.',
  '构建前端产物': 'Build the frontend bundle',
  '任务已派发至「软件研发组」，共 5 个席位。':
    'Task dispatched to Software Team with 5 seats.',
  '## 需求拆解\n\n已完成登录与权限体系的需求拆解：\n\n- **登录方式**：账号密码 + 手机号验证码\n- **会话策略**：JWT 短期票据 + 刷新令牌\n- **权限模型**：角色 + 权限点，按项目隔离\n\n验收标准已同步到任务详情，建议进入架构设计阶段。':
    '## Requirement breakdown\n\nSign-in and permissions have been broken down:\n\n- **Sign-in methods**: password plus SMS code\n- **Session strategy**: short-lived JWT plus refresh token\n- **Permission model**: roles plus permission points, isolated per project\n\nAcceptance criteria are synced to the task; proceed to architecture design.',
  '补充：登录需要支持手机号验证码，验证码 5 分钟过期。':
    'Note: sign-in must support SMS codes; codes expire after 5 minutes.',
  '### 架构结论\n\n采用「认证服务 + 资源服务」分层结构，认证服务负责签发与续期，资源服务只校验权限点。\n\n```ts\ntype Session = {\n  token: string;\n  refreshToken: string;\n  expiresAt: number;\n};\n```\n\n接口契约已整理为 4 个端点，接下来由研发工程师落地。':
    '### Architecture decision\n\nUse a layered "auth service + resource service" design: the auth service issues and refreshes sessions, while the resource service only checks permission points.\n\n```ts\ntype Session = {\n  token: string;\n  refreshToken: string;\n  expiresAt: number;\n};\n```\n\nThe interface contract is organized into 4 endpoints; implementation is next.',
  '任务执行完成，等待阶段门禁审批。':
    'Run complete; waiting for the stage gate approval.',
  'Q4 运营复盘提纲：\n\n1. 目标与达成率\n2. 关键动作复盘\n3. 问题与风险\n4. 下季度行动项\n\n完整报告已生成，请审批后进入下一席位。':
    'Q4 operations review outline:\n\n1. Goals and attainment\n2. Key actions\n3. Issues and risks\n4. Next-quarter action items\n\nThe full report is ready; approve to move to the next seat.',
  '全局': 'Global',
  '阶段门禁：产品经理，整理 Q4 运营复盘报告':
    'Stage gate: Product Manager, Prepare the Q4 operations review',
  '已完成复盘报告提纲与初稿，覆盖目标达成率、关键动作、问题清单与下季度行动项。建议进入架构评审阶段。':
    'The outline and first draft are complete, covering goal attainment, key actions, issues, and next-quarter action items. Proceed to architecture review.',
  '工具授权：Bash': 'Tool approval: Bash',
  '命令执行需要人工确认': 'Command execution requires manual confirmation',
  '阶段门禁：产品经理，实现登录与权限体系':
    'Stage gate: Product Manager, Implement sign-in and permissions',
  '需求清晰，继续。': 'Requirements are clear; continue.',
  '全局，每月美元': 'Global, monthly USD',
};

function localizeDemo<T>(value: T): T {
  if (demoLang !== 'en') return value;
  return localizeValue(value) as T;
}

function localizeValue(value: unknown): unknown {
  if (typeof value === 'string') {
    const exact = DEMO_EN[value];
    if (exact) return exact;
    let text = value;
    for (const [zh, en] of Object.entries(DEMO_EN)) {
      if (zh.length >= 4 && text.includes(zh)) text = text.split(zh).join(en);
    }
    return text;
  }
  if (Array.isArray(value)) return value.map(localizeValue);
  if (value && typeof value === 'object') {
    const copy: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
      copy[key] = localizeValue(item);
    }
    return copy;
  }
  return value;
}

interface MockUsageEvent {
  id: string;
  agentId: string;
  projectId: string;
  teamId: string;
  taskId: string;
  tokens: number;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
  rounds: number;
  occurredAt: number;
}

interface MockReview {
  taskId: string;
  files: ReviewFile[];
  pendingCommits: number;
  diffs: Record<string, string>;
}

interface MockDB {
  projects: Project[];
  agents: Agent[];
  teams: Team[];
  seats: Seat[];
  tasks: Task[];
  runs: Run[];
  toolCalls: ToolCall[];
  reviews: MockReview[];
  messages: Message[];
  usageEvents: MockUsageEvent[];
  budgets: Budget[];
  approvals: Approval[];
  rules: ApprovalRule[];
  events: EventItem[];
  providers: Provider[];
  extensions: ExtensionItem[];
  attachments: Attachment[];
  taskDeps: { taskId: string; dependsOnId: string }[];
  checklists: Record<string, ChecklistItem[]>;
  settings: AppSettings;
  seq: number;
}

const KEY = 'agenthub.mock.v1';
const BACKUP_KEY = 'agenthub.mock.backups.v1';
const DAY = 86_400_000;
let data: MockDB | null = null;

function mockBackups(): BackupFile[] {
  try {
    return JSON.parse(localStorage.getItem(BACKUP_KEY) ?? '[]') as BackupFile[];
  } catch {
    return [];
  }
}

function saveMockBackups(items: BackupFile[]) {
  localStorage.setItem(BACKUP_KEY, JSON.stringify(items.slice(0, 7)));
}

function uid(prefix: string): string {
  const rand = Math.random().toString(36).slice(2, 8);
  return `${prefix}_${Date.now().toString(36)}${rand}`;
}

function save() {
  if (data) localStorage.setItem(KEY, JSON.stringify(data));
}

function load(): MockDB {
  if (!data) {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      try {
        data = JSON.parse(raw) as MockDB;
      } catch {
        data = null;
      }
    }
    if (!data) {
      data = seed();
      save();
    } else {
      data.providers = data.providers ?? [];
      data.extensions = data.extensions ?? [];
      data.toolCalls = data.toolCalls ?? [];
      data.reviews = data.reviews ?? [];
      data.attachments = data.attachments ?? [];
      data.taskDeps = data.taskDeps ?? [];
      data.checklists = data.checklists ?? {};
      data.agents = data.agents.map((agent) => ({
        ...agent,
        providerId: agent.providerId ?? null,
      }));
    }
  }
  return data;
}

const TEMPLATES: TeamTemplate[] = localizeDemo([
  {
    key: 'software-dev',
    name: '软件研发组',
    description: '从需求澄清、架构设计、编码实现到测试与交付的完整研发流水线',
    icon: 'code',
    roles: [
      {
        roleKey: 'pm',
        roleName: '产品经理',
        responsibility: '目标拆解、用户故事、功能边界与验收标准',
        capability: 'office',
        gate: false,
      },
      {
        roleKey: 'architect',
        roleName: '系统架构师',
        responsibility: '技术方案、模块划分、接口契约与风险评估',
        capability: 'code',
        gate: true,
      },
      {
        roleKey: 'developer',
        roleName: '研发工程师',
        responsibility: '按架构落地代码、补充测试、修复问题',
        capability: 'code',
        gate: false,
      },
      {
        roleKey: 'qa',
        roleName: '测试工程师',
        responsibility: '验证验收标准、回归风险、缺陷复现与报告',
        capability: 'code',
        gate: true,
      },
      {
        roleKey: 'delivery',
        roleName: '交付总监',
        responsibility: '汇总产物、检查交付完整性、输出最终说明',
        capability: 'review',
        gate: false,
      },
    ],
  },
  {
    key: 'office-ops',
    name: '办公协作组',
    description: '面向文档、资料与日常办公产出的轻量协作团队',
    icon: 'briefcase',
    roles: [
      {
        roleKey: 'intake',
        roleName: '需求整理',
        responsibility: '澄清目标、受众、交付格式与截止时间',
        capability: 'office',
        gate: false,
      },
      {
        roleKey: 'researcher',
        roleName: '资料研究员',
        responsibility: '收集与核对资料、提炼要点、标注来源',
        capability: 'research',
        gate: false,
      },
      {
        roleKey: 'writer',
        roleName: '文档工程师',
        responsibility: '按目标撰写、整理文档、表格或演示材料',
        capability: 'office',
        gate: false,
      },
      {
        roleKey: 'reviewer',
        roleName: '审核交付',
        responsibility: '检查准确性、完整性与格式，输出交付说明',
        capability: 'review',
        gate: true,
      },
    ],
  },
  {
    key: 'quant-research',
    name: '量化研究组',
    description: '围绕研究假设组织数据、验证过程与最终结论的协作团队',
    icon: 'trending',
    roles: [
      {
        roleKey: 'scope',
        roleName: '投研需求',
        responsibility: '明确研究标的、时间窗口、数据来源与输出格式',
        capability: 'office',
        gate: false,
      },
      {
        roleKey: 'fundamental',
        roleName: '基本面分析师',
        responsibility: '财报与行业数据、关键指标与估值假设',
        capability: 'research',
        gate: false,
      },
      {
        roleKey: 'technical',
        roleName: '量价分析师',
        responsibility: '价格、成交量与市场结构，识别趋势和关键区间',
        capability: 'research',
        gate: false,
      },
      {
        roleKey: 'risk',
        roleName: '风控专员',
        responsibility: '敞口测算、情景分析、仓位与止损约束',
        capability: 'review',
        gate: true,
      },
      {
        roleKey: 'director',
        roleName: '研究主管',
        responsibility: '汇总分歧、校验证据、形成研究结论',
        capability: 'review',
        gate: true,
      },
    ],
  },
]);

function seed(): MockDB {
  const now = Date.now();
  const base: MockDB = {
    projects: [],
    agents: [],
    teams: [],
    seats: [],
    tasks: [],
    runs: [],
    toolCalls: [],
    reviews: [],
    messages: [],
    usageEvents: [],
    budgets: [],
    approvals: [],
    rules: [],
    events: [],
    providers: [],
    extensions: [],
    attachments: [],
    taskDeps: [],
    checklists: {},
    settings: {
      maxConcurrentRuns: 3,
      approvalTimeoutSecs: 600,
      transcriptRetentionDays: 90,
      notify: true,
      proxyEnabled: true,
      runMaxMinutes: 180,
      hookStrict: false,
      nonClaudeRunApproval: true,
      sandboxEnabled: true,
    },
    seq: 0,
  };

  const projectId = 'p_demo';
  base.projects.push({
    id: projectId,
    name: 'Lumen 工作台',
    path: 'D:\\Projects\\lumen',
    kind: 'app',
    description: '多 Agent 协同工作台本体，包含编排、审批与用量统计。',
    isGit: true,
    gitReason: null,
    createdAt: now - 12 * DAY,
    updatedAt: now - 2 * 3600_000,
  });

  base.agents.push({
    id: 'a_claude',
    name: 'Claude Code',
    kind: 'cli',
    adapter: 'claude-code',
    command: 'claude',
    model: 'claude-sonnet-4-5',
    status: 'running',
    version: '2.0.14',
    capabilities: { streamUsage: true, toolApproval: true },
    lastSeenAt: now - 120_000,
    createdAt: now - 20 * DAY,
  });
  base.agents.push({
    id: 'a_opencode',
    name: 'OpenCode',
    kind: 'cli',
    adapter: 'opencode',
    command: 'opencode',
    model: 'gpt-5-codex',
    status: 'idle',
    version: '0.9.31',
    capabilities: { streamUsage: false, toolApproval: false },
    lastSeenAt: now - 3600_000,
    createdAt: now - 15 * DAY,
  });

  const template = TEMPLATES[0];
  const teamId = 't_demo';
  base.teams.push({
    id: teamId,
    projectId,
    name: '软件研发组',
    templateKey: template.key,
    description: template.description,
    seatsCount: template.roles.length,
    assignedCount: 3,
    createdAt: now - 10 * DAY,
  });
  template.roles.forEach((role, index) => {
    base.seats.push({
      id: `s_${role.roleKey}`,
      teamId,
      roleKey: role.roleKey,
      roleName: role.roleName,
      responsibility: role.responsibility,
      agentId: index < 3 ? (index % 2 === 0 ? 'a_claude' : 'a_opencode') : null,
      agentName: index < 3 ? (index % 2 === 0 ? 'Claude Code' : 'OpenCode') : null,
      orderIndex: index,
      gate: role.gate,
    });
  });

  const task1: Task = {
    id: 'task_demo_1',
    projectId,
    projectName: 'Lumen 工作台',
    teamId,
    teamName: '软件研发组',
    title: '实现登录与权限体系',
    description:
      '为桌面端补充账号登录、会话保持与基于角色的权限控制，要求与现有 Rust 后端打通。',
    acceptance:
      '1. 支持账号密码登录与手机号验证码登录\n2. 会话过期自动续期\n3. 无权限操作返回明确错误',
    mandate: 'ask',
    workDir: 'C:\\Lumen\\worktrees\\task_demo_1',
    status: 'running',
    priority: 1,
    assigneeSeatId: 's_architect',
    stageIndex: 1,
    stageKey: 'architect',
    actualTokens: 186_420,
    actualCostMicros: 912_000,
    createdAt: now - 3 * 3600_000,
    startedAt: now - 2 * 3600_000,
  };
  base.tasks.push(task1);

  base.reviews.push({
    taskId: task1.id,
    pendingCommits: 0,
    files: [
      {
        path: 'src/auth/login.ts',
        code: '??',
        status: 'untracked',
        additions: 48,
        deletions: 0,
        binary: false,
      },
      {
        path: 'src/router/index.ts',
        code: 'M',
        status: 'modified',
        additions: 6,
        deletions: 2,
        binary: false,
      },
    ],
    diffs: {
      'src/auth/login.ts': [
        'diff --git a/src/auth/login.ts b/src/auth/login.ts',
        'new file mode 100644',
        '--- /dev/null',
        '+++ b/src/auth/login.ts',
        '@@ -0,0 +1,7 @@',
        '+export async function login(input: LoginInput) {',
        '+  const session = await api.login(input);',
        '+  if (!session) throw new Error("登录失败");',
        '+  await store.saveSession(session);',
        '+  return session;',
        '+}',
        '+',
      ].join('\n'),
      'src/router/index.ts': [
        'diff --git a/src/router/index.ts b/src/router/index.ts',
        '--- a/src/router/index.ts',
        '+++ b/src/router/index.ts',
        '@@ -12,5 +12,9 @@',
        '   { path: "/tasks", element: <Tasks /> },',
        '+  {',
        '+    path: "/tasks/:id",',
        '+    element: <TaskDetail />,',
        '+  },',
        '   { path: "/teams", element: <Teams /> },',
      ].join('\n'),
    },
  });
  base.tasks.push({
    id: 'task_demo_2',
    projectId,
    projectName: 'Lumen 工作台',
    teamId,
    teamName: '软件研发组',
    title: '整理 Q4 运营复盘报告',
    description: '汇总第四季度关键指标，输出结构化复盘文档与改进项。',
    acceptance: '包含目标达成率、问题清单与下季度行动项',
    mandate: 'auto',
    workDir: null,
    status: 'waiting_approval',
    priority: 2,
    assigneeSeatId: 's_pm',
    stageIndex: 0,
    stageKey: 'pm',
    actualTokens: 48_200,
    actualCostMicros: 231_000,
    createdAt: now - 26 * 3600_000,
    startedAt: now - 25 * 3600_000,
  });

  const run1: Run = {
    id: 'run_demo_1',
    taskId: task1.id,
    seatId: 's_pm',
    agentId: 'a_claude',
    agentName: 'Claude Code',
    seatName: '产品经理',
    stageKey: 'pm',
    status: 'completed',
    externalSessionId: 'claude_demo_seed_01',
    isolation: 'appcontainer',
    inputTokens: 21_400,
    outputTokens: 6_800,
    costMicros: 148_000,
    error: null,
    startedAt: now - 2 * 3600_000,
    endedAt: now - 2 * 3600_000 + 420_000,
  };
  const run2: Run = {
    id: 'run_demo_2',
    taskId: task1.id,
    seatId: 's_architect',
    agentId: 'a_opencode',
    agentName: 'OpenCode',
      seatName: '系统架构师',
    stageKey: 'architect',
    status: 'running',
    externalSessionId: 'oc_demo_seed_02',
    isolation: 'process',
    inputTokens: 46_200,
    outputTokens: 12_400,
    costMicros: 392_000,
    error: null,
    startedAt: now - 900_000,
  };
  base.runs.push(run1, run2);

  base.toolCalls.push(
    {
      id: `${run1.id}:tool_read`,
      runId: run1.id,
      toolName: 'Read',
      input: { file_path: 'src/routes/auth.ts' },
      output: '读取 128 行，识别出 4 个角色校验分支。',
      status: 'completed',
      riskLevel: 'low',
      createdAt: now - 2 * 3600_000 + 90_000,
      finishedAt: now - 2 * 3600_000 + 92_400,
    },
    {
      id: `${run1.id}:tool_grep`,
      runId: run1.id,
      toolName: 'Grep',
      input: { pattern: 'requireRole\\(', path: 'src' },
      output: 'src/middleware/guard.ts:18\nsrc/routes/orders.ts:42',
      status: 'completed',
      riskLevel: 'low',
      createdAt: now - 2 * 3600_000 + 150_000,
      finishedAt: now - 2 * 3600_000 + 151_200,
    },
    {
      id: `${run2.id}:tool_bash`,
      runId: run2.id,
      toolName: 'Bash',
      input: { command: 'npm run build', description: '构建前端产物' },
      output: null,
      status: 'running',
      riskLevel: 'high',
      createdAt: now - 240_000,
      finishedAt: null,
    },
  );

  base.messages.push(
    {
      id: 'm_seed_1',
      taskId: task1.id,
      runId: null,
      seatId: null,
      agentId: null,
      seatName: null,
      agentName: null,
      kind: 'system',
      role: 'system',
      content: '任务已派发至「软件研发组」，共 5 个席位。',
      metadata: {},
      createdAt: now - 2 * 3600_000,
    },
    {
      id: 'm_seed_2',
      taskId: task1.id,
      runId: run1.id,
      seatId: 's_pm',
      agentId: 'a_claude',
      seatName: '产品经理',
      agentName: 'Claude Code',
      kind: 'chat',
      role: 'assistant',
      content:
        '## 需求拆解\n\n已完成登录与权限体系的需求拆解：\n\n- **登录方式**：账号密码 + 手机号验证码\n- **会话策略**：JWT 短期票据 + 刷新令牌\n- **权限模型**：角色 + 权限点，按项目隔离\n\n验收标准已同步到任务详情，建议进入架构设计阶段。',
      metadata: { source: 'agent' },
      createdAt: now - 2 * 3600_000 + 420_000,
    },
    {
      id: 'm_seed_3',
      taskId: task1.id,
      runId: null,
      seatId: null,
      agentId: null,
      seatName: null,
      agentName: null,
      kind: 'chat',
      role: 'user',
      content: '补充：登录需要支持手机号验证码，验证码 5 分钟过期。',
      metadata: { source: 'user' },
      createdAt: now - 1100_000,
    },
    {
      id: 'm_seed_4',
      taskId: task1.id,
      runId: run2.id,
      seatId: 's_architect',
      agentId: 'a_opencode',
      seatName: '系统架构师',
      agentName: 'OpenCode',
      kind: 'chat',
      role: 'assistant',
      content:
        '### 架构结论\n\n采用「认证服务 + 资源服务」分层结构，认证服务负责签发与续期，资源服务只校验权限点。\n\n```ts\ntype Session = {\n  token: string;\n  refreshToken: string;\n  expiresAt: number;\n};\n```\n\n接口契约已整理为 4 个端点，接下来由研发工程师落地。',
      metadata: { source: 'agent' },
      createdAt: now - 300_000,
    },
    {
      id: 'm_seed_5',
      taskId: 'task_demo_2',
      runId: null,
      seatId: null,
      agentId: null,
      seatName: null,
      agentName: null,
      kind: 'system',
      role: 'system',
      content: '任务执行完成，等待阶段门禁审批。',
      metadata: {},
      createdAt: now - 24 * 3600_000,
    },
    {
      id: 'm_seed_6',
      taskId: 'task_demo_2',
      runId: null,
      seatId: 's_pm',
      agentId: 'a_claude',
      seatName: '产品经理',
      agentName: 'Claude Code',
      kind: 'chat',
      role: 'assistant',
      content:
        'Q4 运营复盘提纲：\n\n1. 目标与达成率\n2. 关键动作复盘\n3. 问题与风险\n4. 下季度行动项\n\n完整报告已生成，请审批后进入下一席位。',
      metadata: { source: 'agent' },
      createdAt: now - 24 * 3600_000 + 300_000,
    },
  );

  let rnd = 20260101;
  const rand = () => {
    rnd = (rnd * 1103515245 + 12345) % 2147483648;
    return rnd / 2147483648;
  };
  for (let d = 29; d >= 0; d--) {
    const count = d === 0 ? 4 : 2 + Math.floor(rand() * 4);
    for (let i = 0; i < count; i++) {
      const agent = i % 2 === 0 ? base.agents[0] : base.agents[1];
      const task = i % 3 === 0 ? base.tasks[1] : base.tasks[0];
      const tokens = 3_000 + Math.floor(rand() * 24_000);
      const inputTokens = Math.round(tokens * 0.78);
      base.usageEvents.push({
        id: `u_${d}_${i}`,
        agentId: agent.id,
        projectId,
        teamId,
        taskId: task.id,
        tokens,
        inputTokens,
        outputTokens: tokens - inputTokens,
        costMicros: Math.round(tokens * (agent.id === 'a_claude' ? 5.4 : 2.4)),
        rounds: 1 + Math.floor(rand() * 3),
        occurredAt:
          d === 0 ? now - (i + 1) * 7 * 60_000 : now - d * DAY - (i + 1) * 1_900_000,
      });
    }
  }

  base.budgets.push(
    {
      id: 'b_global',
      scopeType: 'global',
      scopeId: null,
      scopeName: '全局',
      unit: 'usd',
      period: 'month',
      limitValue: 2_500_000,
      warnPct: 80,
      hardStop: true,
      enabled: true,
      spent: 0,
      reserved: 0,
      pct: 0,
      blocked: false,
      createdAt: now - 8 * DAY,
    },
    {
      id: 'b_project',
      scopeType: 'project',
      scopeId: projectId,
      scopeName: 'Lumen 工作台',
      unit: 'tokens',
      period: 'day',
      limitValue: 200_000,
      warnPct: 80,
      hardStop: false,
      enabled: true,
      spent: 0,
      reserved: 0,
      pct: 0,
      blocked: false,
      createdAt: now - 7 * DAY,
    },
    {
      id: 'b_agent',
      scopeType: 'agent',
      scopeId: 'a_claude',
      scopeName: 'Claude Code',
      unit: 'usd',
      period: 'month',
      limitValue: 12_000_000,
      warnPct: 80,
      hardStop: false,
      enabled: true,
      spent: 0,
      reserved: 0,
      pct: 0,
      blocked: false,
      createdAt: now - 6 * DAY,
    },
  );

  base.approvals.push(
    {
      id: 'ap_stage',
      taskId: 'task_demo_2',
      taskTitle: '整理 Q4 运营复盘报告',
      runId: null,
      type: 'stage_gate',
      riskLevel: 'medium',
      title: '阶段门禁：产品经理，整理 Q4 运营复盘报告',
      detail: {
        stageIndex: 0,
        nextStageIndex: 1,
        roleName: '产品经理',
        summary:
          '已完成复盘报告提纲与初稿，覆盖目标达成率、关键动作、问题清单与下季度行动项。建议进入架构评审阶段。',
      },
      status: 'pending',
      requestedBy: 'Claude Code',
      decidedBy: null,
      decisionNote: null,
      createdAt: now - 23 * 3600_000,
      decidedAt: null,
    },
    {
      id: 'ap_tool',
      taskId: 'task_demo_1',
      taskTitle: '实现登录与权限体系',
      runId: 'run_demo_2',
      type: 'tool_permission',
      riskLevel: 'high',
      title: '工具授权：Bash',
      detail: {
        toolName: 'Bash',
        toolInput: { command: 'npm run build', description: '构建前端产物' },
        cwd: 'D:\\Projects\\agenthub',
        reason: '命令执行需要人工确认',
        seatName: '系统架构师',
        agentName: 'OpenCode',
      },
      status: 'pending',
      requestedBy: 'OpenCode',
      decidedBy: null,
      decisionNote: null,
      createdAt: now - 420_000,
      decidedAt: null,
    },
    {
      id: 'ap_decided',
      taskId: 'task_demo_1',
      taskTitle: '实现登录与权限体系',
      runId: 'run_demo_1',
      type: 'stage_gate',
      riskLevel: 'low',
      title: '阶段门禁：产品经理，实现登录与权限体系',
      detail: { stageIndex: 0, nextStageIndex: 1, roleName: '产品经理' },
      status: 'approved',
      requestedBy: 'Claude Code',
      decidedBy: 'user',
      decisionNote: '需求清晰，继续。',
      createdAt: now - 2 * 3600_000 + 500_000,
      decidedAt: now - 2 * 3600_000 + 620_000,
    },
  );

  base.rules.push(
    {
      id: 'r_1',
      scopeType: 'global',
      scopeId: null,
      match: { tool: 'Read' },
      action: 'allow',
      priority: 10,
      enabled: true,
      createdAt: now - 8 * DAY,
    },
    {
      id: 'r_2',
      scopeType: 'global',
      scopeId: null,
      match: { tool: 'Bash', commandContains: 'rm -rf' },
      action: 'deny',
      priority: 5,
      enabled: true,
      createdAt: now - 8 * DAY,
    },
    {
      id: 'r_3',
      scopeType: 'global',
      scopeId: null,
      match: { tool: 'Write', pathGlob: 'src/**' },
      action: 'ask',
      priority: 100,
      enabled: true,
      createdAt: now - 5 * DAY,
    },
  );

  const eventSeeds: Array<[string, string | null, unknown, number]> = [
    ['project.created', 'user', { name: 'Lumen 工作台' }, now - 12 * DAY],
    ['task.created', 'user', { title: '实现登录与权限体系' }, now - 3 * 3600_000],
    ['task.dispatched', 'user', { stageIndex: 0 }, now - 2 * 3600_000],
    ['run.started', 'system', { seat: '产品经理' }, now - 2 * 3600_000],
    ['run.completed', 'system', { tokens: 28_200 }, now - 2 * 3600_000 + 420_000],
    ['approval.requested', 'gate', { type: 'stage_gate' }, now - 2 * 3600_000 + 440_000],
    ['approval.decided', 'user', { approved: true }, now - 2 * 3600_000 + 620_000],
    ['run.started', 'system', { seat: '系统架构师' }, now - 900_000],
    ['approval.requested', 'gate', { type: 'tool_permission', tool: 'Bash' }, now - 420_000],
    ['budget.alert', 'budget-engine', { level: 'warning', name: '全局，每月美元' }, now - 300_000],
    ['usage.updated', 'system', { tokens: 58_600 }, now - 200_000],
  ];
  eventSeeds.forEach(([type, actor, payload, at], index) => {
    base.seq = index + 1;
    base.events.push({
      id: `e_seed_${index}`,
      seq: base.seq,
      projectId,
      taskId: index > 0 ? 'task_demo_1' : null,
      runId: null,
      type,
      actor,
      payload,
      createdAt: at,
    });
  });

  return localizeDemo(base);
}

function trackEvent(type: string, payload: unknown, taskId?: string | null) {
  const db = load();
  db.seq += 1;
  const event: EventItem = {
    id: uid('e'),
    seq: db.seq,
    projectId: null,
    taskId: taskId ?? null,
    runId: null,
    type,
    actor: 'user',
    payload,
    createdAt: Date.now(),
  };
  db.events.push(event);
}

function dateKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(
    d.getDate(),
  ).padStart(2, '0')}`;
}

function startOfToday(): number {
  const d = new Date();
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

function startOfMonth(): number {
  const d = new Date();
  return new Date(d.getFullYear(), d.getMonth(), 1).getTime();
}

function summaryOf(db: MockDB): UsageSummary {
  const todayStart = startOfToday();
  let todayTokens = 0;
  let totalTokens = 0;
  let todayCost = 0;
  let totalCost = 0;
  let rounds = 0;
  let todayRounds = 0;
  for (const e of db.usageEvents) {
    totalTokens += e.tokens;
    totalCost += e.costMicros;
    rounds += e.rounds;
    if (e.occurredAt >= todayStart) {
      todayTokens += e.tokens;
      todayCost += e.costMicros;
      todayRounds += e.rounds;
    }
  }
  return {
    todayTokens,
    totalTokens,
    todayCostMicros: todayCost,
    totalCostMicros: totalCost,
    cacheHitPct: 74.6,
    rounds,
    todayRounds,
  };
}

function seriesOf(db: MockDB, days: number): UsagePoint[] {
  const map = new Map<string, UsagePoint>();
  const today = startOfToday();
  for (let i = days - 1; i >= 0; i--) {
    const key = dateKey(today - i * DAY);
    map.set(key, { date: key, tokens: 0, costMicros: 0 });
  }
  for (const e of db.usageEvents) {
    const key = dateKey(e.occurredAt);
    const point = map.get(key);
    if (point) {
      point.tokens += e.tokens;
      point.costMicros += e.costMicros;
    }
  }
  return Array.from(map.values());
}

function scopeUsage(db: MockDB, field: 'agentId' | 'projectId' | 'taskId', nameOf: (id: string) => string): ScopedUsage[] {
  const map = new Map<string, ScopedUsage>();
  for (const e of db.usageEvents) {
    const key = e[field];
    const item = map.get(key) ?? { key, name: nameOf(key), tokens: 0, costMicros: 0, rounds: 0 };
    item.tokens += e.tokens;
    item.costMicros += e.costMicros;
    item.rounds += e.rounds;
    map.set(key, item);
  }
  return Array.from(map.values()).sort((a, b) => b.tokens - a.tokens);
}

function budgetSpent(db: MockDB, b: Budget): number {
  const todayStart = startOfToday();
  const monthStart = startOfMonth();
  const scopeEvents = db.usageEvents.filter((e) => {
    if (b.scopeType === 'project') return e.projectId === b.scopeId;
    if (b.scopeType === 'team') return e.teamId === b.scopeId;
    if (b.scopeType === 'agent') return e.agentId === b.scopeId;
    if (b.scopeType === 'task') return e.taskId === b.scopeId;
    return true;
  });
  const periodEvents = scopeEvents.filter((e) => {
    if (b.period === 'day') return e.occurredAt >= todayStart;
    if (b.period === 'month') return e.occurredAt >= monthStart;
    return true;
  });
  return periodEvents.reduce(
    (sum, e) => sum + (b.unit === 'usd' ? e.costMicros : e.tokens),
    0,
  );
}

function computeBudget(db: MockDB, b: Budget): Budget {
  const spent = budgetSpent(db, b);
  const pct = b.limitValue > 0 ? Math.round((spent / b.limitValue) * 1000) / 10 : 0;
  return {
    ...b,
    spent,
    reserved: 0,
    pct,
    blocked: b.enabled && b.hardStop && spent >= b.limitValue,
  };
}

function syncTeam(db: MockDB, teamId: string) {
  const team = db.teams.find((t) => t.id === teamId);
  if (!team) return;
  const seats = db.seats.filter((s) => s.teamId === teamId);
  team.seatsCount = seats.length;
  team.assignedCount = seats.filter((s) => s.agentId).length;
}

function agentStatusById(db: MockDB, id: string | null | undefined): string {
  if (!id) return 'idle';
  return db.agents.find((a) => a.id === id)?.status ?? 'idle';
}

function agentNameById(db: MockDB, id: string | null | undefined): string {
  if (!id) return '';
  return db.agents.find((a) => a.id === id)?.name ?? '';
}

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function mockInvoke<T>(cmd: string, rawArgs?: Record<string, unknown>): Promise<T> {
  await delay(50 + Math.random() * 90);
  const db = load();
  const args = (rawArgs ?? {}) as Record<string, any>;
  const now = Date.now();

  switch (cmd) {
    case 'app_info': {
      const info: AppInfo = {
        version: '0.1.0',
        dataDir: 'C:\\Users\\User\\AppData\\Roaming\\Lumen',
        dbPath: 'C:\\Users\\User\\AppData\\Roaming\\Lumen\\lumen.db',
        gatePort: 17321,
        platform: 'windows',
      };
      return info as T;
    }
    case 'set_backend_lang':
      return undefined as T;
    case 'app_settings_get':
      return { ...db.settings } as T;
    case 'app_settings_save':
      db.settings = { ...db.settings, ...(args.settings as AppSettings) };
      save();
      return undefined as T;
    case 'open_data_dir':
      return undefined as T;
    case 'open_backup_dir':
      return undefined as T;
    case 'data_backup': {
      const name = `lumen-${now}.db`;
      const path = `D:\\Lumen\\backups\\${name}`;
      const items = mockBackups();
      items.unshift({
        name,
        path,
        size: 1_600_000 + Math.floor(Math.random() * 800_000),
        createdAt: now,
      });
      saveMockBackups(items);
      return path as T;
    }
    case 'data_export':
      return `D:\\Lumen\\exports\\lumen-export-${now}.json` as T;
    case 'backups_list':
      return mockBackups().sort((a, b) => b.createdAt - a.createdAt) as T;
    case 'data_restore':
      return undefined as T;
    case 'sandbox_probe':
      return {
        ok: true,
        profile: 'com.agenthub.desktop.sandbox.probe',
        workRead: true,
        workWrite: true,
        existingRead: true,
        outsideReadBlocked: true,
        userReadBlocked: true,
        outsideWriteBlocked: true,
        homeRedirected: true,
        relayEcho: true,
        stdioForwarded: true,
        hostExitCode: 0,
        detail: '演示数据：浏览器预览不执行真实沙箱自检',
        elapsedMs: 0,
      } as T;
    case 'cli_detect': {
      const list: DetectedCli[] = [
        {
          name: 'Claude Code',
          command: 'claude',
          adapter: 'claude-code',
          version: args.withVersion ? '2.0.14' : null,
          installed: true,
          recommended: true,
          installable: true,
          installHint: 'npm i -g @anthropic-ai/claude-code',
          capabilities: {
            streaming: true,
            streamUsage: true,
            toolApproval: true,
            resume: true,
            budgetEnforcement: true,
            usageSource: 'provider',
          },
        },
        {
          name: 'OpenCode',
          command: 'opencode',
          adapter: 'opencode',
          version: args.withVersion ? '0.9.31' : null,
          installed: true,
          recommended: true,
          installable: true,
          installHint: 'npm i -g opencode-ai',
          capabilities: {
            streaming: true,
            streamUsage: false,
            toolApproval: false,
            resume: false,
            budgetEnforcement: false,
            usageSource: 'estimate',
          },
        },
        {
          name: 'Codex',
          command: 'codex',
          adapter: 'pty',
          version: null,
          installed: false,
          recommended: false,
          installable: true,
          installHint: 'npm i -g @openai/codex',
        },
        {
          name: 'Gemini CLI',
          command: 'gemini',
          adapter: 'pty',
          version: args.withVersion ? '0.6.2' : null,
          installed: true,
          recommended: false,
          installable: true,
          installHint: 'npm i -g @google/gemini-cli',
          capabilities: {
            streaming: true,
            streamUsage: false,
            toolApproval: false,
            resume: false,
            budgetEnforcement: false,
            usageSource: 'estimate',
          },
        },
        {
          name: 'Aider',
          command: 'aider',
          adapter: 'pty',
          version: null,
          installed: false,
          recommended: false,
          installable: true,
          installHint: 'pip install aider-chat',
        },
      ];
      return list as T;
    }
    case 'cli_install':
      return '演示模式：已模拟完成安装' as T;
    case 'cli_latest_versions': {
      const latest: Record<string, string> = {
        'Claude Code': '2.0.15',
        OpenCode: '0.9.31',
        'Gemini CLI': '0.6.2',
      };
      const names = Array.isArray(args.names) ? (args.names as string[]) : [];
      return names.map((name) => ({ name, latestVersion: latest[name] ?? null })) as T;
    }
    case 'agents_list':
      return [...db.agents] as T;
    case 'agent_create': {
      const name = String(args.name ?? '').trim();
      if (!name) throw new Error('Agent 名称不能为空');
      const adapter = String(args.adapter ?? 'claude-code');
      let command = String(args.command ?? '').trim();
      if (!command) {
        command = adapter === 'claude-code' ? 'claude' : adapter === 'opencode' ? 'opencode' : '';
        if (!command) throw new Error('该适配器需要填写启动命令');
      }
      const id = uid('a');
      db.agents.push({
        id,
        name,
        kind: 'cli',
        adapter,
        command,
        model: args.model ? String(args.model) : null,
        providerId: args.providerId ? String(args.providerId) : null,
        status: 'idle',
        version: adapter === 'claude-code' ? '2.0.14' : adapter === 'opencode' ? '0.9.31' : null,
        capabilities: {
          streamUsage: adapter === 'claude-code',
          toolApproval: adapter === 'claude-code',
        },
        lastSeenAt: now,
        createdAt: now,
      });
      save();
      return id as T;
    }
    case 'agent_update': {
      const agent = db.agents.find((a) => a.id === args.id);
      if (!agent) throw new Error('Agent 不存在');
      agent.name = String(args.name ?? agent.name).trim();
      agent.command = String(args.command ?? agent.command ?? '').trim();
      agent.model = args.model ? String(args.model) : null;
      agent.providerId = args.providerId ? String(args.providerId) : null;
      save();
      return undefined as T;
    }
    case 'agent_delete': {
      db.agents = db.agents.filter((a) => a.id !== args.id);
      db.seats.forEach((s) => {
        if (s.agentId === args.id) {
          s.agentId = null;
          s.agentName = null;
        }
      });
      db.teams.forEach((t) => syncTeam(db, t.id));
      save();
      return undefined as T;
    }
    case 'providers_list':
      return [...db.providers] as T;
    case 'provider_upsert': {
      const name = String(args.name ?? '').trim();
      if (!name) throw new Error('供应商名称不能为空');
      const baseUrl = String(args.baseUrl ?? '').trim().replace(/\/+$/, '');
      if (!/^https?:\/\//.test(baseUrl)) {
        throw new Error('Base URL 必须以 http:// 或 https:// 开头');
      }
      const protocol = args.protocol === 'openai' ? 'openai' : 'anthropic';
      const key = args.apiKey ? String(args.apiKey).trim() : '';
      const existing = db.providers.find((p) => p.id === args.id);
      if (existing) {
        existing.name = name;
        existing.protocol = protocol;
        existing.baseUrl = baseUrl;
        existing.model = args.model ? String(args.model) : null;
        if (Array.isArray(args.models)) existing.models = args.models.map(String);
        existing.enabled = args.enabled !== false;
        existing.keyHint = key ? `••••${key.slice(-4)}` : existing.keyHint ?? null;
        existing.updatedAt = now;
        save();
        return existing.id as T;
      }
      const provider: Provider = {
        id: uid('prov'),
        name,
        protocol,
        baseUrl,
        model: args.model ? String(args.model) : null,
        models: Array.isArray(args.models) ? args.models.map(String) : [],
        keyHint: key ? `••••${key.slice(-4)}` : null,
        enabled: args.enabled !== false,
        createdAt: now,
        updatedAt: now,
      };
      db.providers.push(provider);
      save();
      return provider.id as T;
    }
    case 'provider_delete': {
      db.providers = db.providers.filter((p) => p.id !== args.id);
      db.agents.forEach((agent) => {
        if (agent.providerId === args.id) agent.providerId = null;
      });
      save();
      return undefined as T;
    }
    case 'provider_test': {
      const provider = db.providers.find((p) => p.id === args.id);
      if (!provider) throw new Error('供应商不存在');
      if (!provider.model) throw new Error('请先填写模型名称再测试连接');
      return {
        ok: true,
        status: 200,
        latencyMs: 120,
        message: '浏览器预览模式：模拟连接成功（不会发起真实请求）',
      } as T;
    }
    case 'provider_models': {
      const baseUrl = String(args.baseUrl ?? '').trim();
      if (!/^https?:\/\//.test(baseUrl)) {
        throw new Error('Base URL 必须以 http:// 或 https:// 开头');
      }
      return (args.protocol === 'openai'
        ? ['gpt-4.1', 'gpt-4.1-mini', 'o4-mini']
        : ['claude-sonnet-4-5', 'claude-opus-4-1', 'claude-haiku-4-5']) as T;
    }
    case 'extensions_list':
      return [...db.extensions] as T;
    case 'extension_upsert': {
      const kind = String(args.kind ?? '').trim();
      if (!['mcp', 'hook', 'skill', 'plugin'].includes(kind)) {
        throw new Error('未知的扩展类型');
      }
      const name = String(args.name ?? '').trim();
      if (!name) throw new Error('扩展名称不能为空');
      const config = { ...(args.config ?? {}) } as Record<string, unknown>;
      if (kind === 'mcp') {
        const transport = String(config.transport ?? 'stdio');
        if (transport === 'stdio' && !String(config.command ?? '').trim()) {
          throw new Error('stdio 类型的 MCP 需要填写启动命令');
        }
        if (transport !== 'stdio' && !/^https?:\/\//.test(String(config.url ?? ''))) {
          throw new Error('远程 MCP 需要填写 http/https 地址');
        }
      }
      if (kind === 'hook' && !String(config.command ?? '').trim()) {
        throw new Error('Hook 需要填写执行命令');
      }
      if (kind === 'skill' && !String(config.content ?? '').trim()) {
        throw new Error('Skill 内容不能为空');
      }
      if (kind === 'plugin' && !String(config.path ?? '').trim()) {
        throw new Error('请填写插件目录或 .zip 路径');
      }
      const existing = db.extensions.find((e) => e.id === args.id);
      if (existing) {
        existing.kind = kind as ExtensionItem['kind'];
        existing.name = name;
        existing.description = args.description ? String(args.description) : null;
        existing.config = config;
        existing.enabled = args.enabled !== false;
        existing.updatedAt = now;
        save();
        return existing.id as T;
      }
      const extension: ExtensionItem = {
        id: uid('ext'),
        kind: kind as ExtensionItem['kind'],
        name,
        description: args.description ? String(args.description) : null,
        config,
        enabled: args.enabled !== false,
        createdAt: now,
        updatedAt: now,
      };
      db.extensions.push(extension);
      save();
      return extension.id as T;
    }
    case 'extension_toggle': {
      const extension = db.extensions.find((e) => e.id === args.id);
      if (extension) {
        extension.enabled = Boolean(args.enabled);
        extension.updatedAt = now;
        save();
      }
      return undefined as T;
    }
    case 'extensions_health':
      return db.extensions.map((item) => ({
        id: item.id,
        name: item.name,
        kind: item.kind,
        enabled: item.enabled,
        status: 'ok',
        message: '预览模式：配置结构有效',
      })) as T;
    case 'extension_delete':
      db.extensions = db.extensions.filter((e) => e.id !== args.id);
      save();
      return undefined as T;
    case 'projects_list':
      return [...db.projects].sort((a, b) => b.updatedAt - a.updatedAt) as T;
    case 'project_git_check':
      return {
        isGit: true,
        reason: null,
      } as T;
    case 'project_create': {
      const name = String(args.name ?? '').trim();
      const path = String(args.path ?? '').trim();
      if (!name) throw new Error('项目名称不能为空');
      if (!path) throw new Error('请选择项目目录');
      const project: Project = {
        id: uid('p'),
        name,
        path,
        kind: String(args.kind ?? 'app'),
        description: args.description ? String(args.description) : null,
        isGit: true,
        gitReason: null,
        createdAt: now,
        updatedAt: now,
      };
      db.projects.push(project);
      trackEvent('project.created', { name }, null);
      save();
      return project.id as T;
    }
    case 'project_update': {
      const project = db.projects.find((p) => p.id === args.id);
      if (!project) throw new Error('项目不存在');
      project.name = String(args.name ?? project.name).trim();
      project.path = String(args.path ?? project.path).trim();
      project.kind = String(args.kind ?? project.kind);
      project.description = args.description ? String(args.description) : null;
      project.updatedAt = now;
      save();
      return undefined as T;
    }
    case 'project_delete': {
      const projectId = String(args.id);
      if (
        db.tasks.some(
          (t) =>
            t.projectId === projectId &&
            isActiveStatus(t.status),
        )
      ) {
        throw new Error('项目下有任务正在运行，请先停止后再删除项目');
      }
      const teamIds = db.teams.filter((t) => t.projectId === projectId).map((t) => t.id);
      db.projects = db.projects.filter((p) => p.id !== projectId);
      db.teams = db.teams.filter((t) => t.projectId !== projectId);
      db.seats = db.seats.filter((s) => !teamIds.includes(s.teamId));
      const taskIds = db.tasks.filter((t) => t.projectId === projectId).map((t) => t.id);
      db.tasks = db.tasks.filter((t) => t.projectId !== projectId);
      db.messages = db.messages.filter((m) => !m.taskId || !taskIds.includes(m.taskId));
      const runIds = new Set(db.runs.filter((r) => taskIds.includes(r.taskId)).map((r) => r.id));
      db.toolCalls = db.toolCalls.filter((c) => !runIds.has(c.runId));
      db.runs = db.runs.filter((r) => !taskIds.includes(r.taskId));
      db.approvals = db.approvals.filter((a) => !a.taskId || !taskIds.includes(a.taskId));
      db.usageEvents = db.usageEvents.filter((u) => u.projectId !== projectId);
      db.budgets = db.budgets.filter((b) => !(b.scopeType === 'project' && b.scopeId === projectId));
      save();
      return undefined as T;
    }
    case 'team_templates':
      return JSON.parse(JSON.stringify(TEMPLATES)) as T;
    case 'teams_list': {
      db.teams.forEach((t) => syncTeam(db, t.id));
      save();
      const list = args.projectId
        ? db.teams.filter((t) => t.projectId === args.projectId)
        : db.teams;
      return [...list]
        .sort((a, b) => b.createdAt - a.createdAt)
        .map((t) => ({
          ...t,
          projectName: db.projects.find((p) => p.id === t.projectId)?.name ?? null,
        })) as T;
    }
    case 'team_create': {
      const template = TEMPLATES.find((t) => t.key === args.templateKey);
      if (!template) throw new Error('团队模板不存在');
      const projectId = String(args.projectId ?? '');
      if (!db.projects.some((p) => p.id === projectId)) throw new Error('请选择项目');
      const team: Team = {
        id: uid('t'),
        projectId,
        projectName: db.projects.find((p) => p.id === projectId)?.name ?? null,
        name: String(args.name ?? template.name).trim() || template.name,
        templateKey: template.key,
        description: args.description ? String(args.description) : template.description,
        seatsCount: template.roles.length,
        assignedCount: 0,
        createdAt: now,
      };
      db.teams.push(team);
      template.roles.forEach((role, index) => {
        db.seats.push({
          id: uid('s'),
          teamId: team.id,
          roleKey: role.roleKey,
          roleName: role.roleName,
          responsibility: role.responsibility,
          agentId: null,
          agentName: null,
          orderIndex: index,
          gate: role.gate,
        });
      });
      save();
      return team.id as T;
    }
    case 'team_delete': {
      db.teams = db.teams.filter((t) => t.id !== args.id);
      db.seats = db.seats.filter((s) => s.teamId !== args.id);
      db.tasks.forEach((t) => {
        if (t.teamId === args.id) {
          t.teamId = null;
          t.teamName = null;
        }
      });
      save();
      return undefined as T;
    }
    case 'seats_list':
      return db.seats
        .filter((s) => s.teamId === args.teamId)
        .sort((a, b) => a.orderIndex - b.orderIndex) as T;
    case 'seat_assign': {
      const seat = db.seats.find((s) => s.id === args.seatId);
      if (!seat) throw new Error('席位不存在');
      seat.agentId = args.agentId ?? null;
      seat.agentName = agentNameById(db, seat.agentId) || null;
      syncTeam(db, seat.teamId);
      save();
      return undefined as T;
    }
    case 'seat_update': {
      const seat = db.seats.find((s) => s.id === args.seatId);
      if (!seat) throw new Error('席位不存在');
      seat.roleName = String(args.roleName ?? seat.roleName);
      seat.responsibility = args.responsibility ?? seat.responsibility;
      seat.gate = Boolean(args.gate);
      save();
      return undefined as T;
    }
    case 'seat_reorder': {
      const seatIds = (args.seatIds as string[]) ?? [];
      seatIds.forEach((id, index) => {
        const seat = db.seats.find((s) => s.id === id);
        if (seat) seat.orderIndex = index;
      });
      save();
      return undefined as T;
    }
    case 'seat_auto_assign': {
      const teamId = String(args.teamId);
      if (db.agents.length === 0) throw new Error('还没有可用的 Agent，请先添加');
      const seats = db.seats.filter((s) => s.teamId === teamId).sort((a, b) => a.orderIndex - b.orderIndex);
      let count = 0;
      seats.forEach((seat, index) => {
        if (!seat.agentId) {
          const agent = db.agents[index % db.agents.length];
          seat.agentId = agent.id;
          seat.agentName = agent.name;
          count += 1;
        }
      });
      syncTeam(db, teamId);
      save();
      return count as T;
    }
    case 'task_get': {
      const task = db.tasks.find((t) => t.id === args.id);
      if (!task) throw new Error('任务不存在');
      return task as T;
    }
    case 'tasks_list': {
      let list = [...db.tasks];
      if (args.projectId) list = list.filter((t) => t.projectId === args.projectId);
      if (args.status) list = list.filter((t) => t.status === args.status);
      list.sort((a, b) => a.priority - b.priority || b.createdAt - a.createdAt);
      return list as T;
    }
    case 'tasks_page': {
      let list = [...db.tasks];
      if (args.projectId) list = list.filter((t) => t.projectId === args.projectId);
      if (args.status) list = list.filter((t) => t.status === args.status);
      const query = String(args.query ?? '').trim().toLowerCase();
      if (query) {
        list = list.filter(
          (t) =>
            t.title.toLowerCase().includes(query) ||
            (t.description ?? '').toLowerCase().includes(query),
        );
      }
      if (args.attentionOnly) list = list.filter((t) => needsYouTask(t));
      list.sort(
        (a, b) =>
          a.priority - b.priority || b.createdAt - a.createdAt || (a.id < b.id ? 1 : -1),
      );
      const statusCounts: Record<string, number> = {};
      for (const task of list) {
        statusCounts[task.status] = (statusCounts[task.status] ?? 0) + 1;
      }
      const total = list.length;
      const reviewCount = list.filter(
        (t) =>
          t.reviewPending &&
          !isActiveStatus(t.status),
      ).length;
      const limit = Math.min(200, Math.max(1, Number(args.limit ?? 60)));
      let start = 0;
      const cursor = String(args.cursor ?? '');
      if (cursor) {
        const [priority, createdAt, id] = cursor.split(':');
        const index = list.findIndex(
          (t) =>
            t.priority === Number(priority) &&
            t.createdAt === Number(createdAt) &&
            t.id === id,
        );
        if (index >= 0) start = index + 1;
      }
      const items = list.slice(start, start + limit);
      const hasMore = start + items.length < total;
      const last = items[items.length - 1];
      return {
        items,
        total,
        statusCounts,
        reviewCount,
        hasMore,
        nextCursor: hasMore && last ? `${last.priority}:${last.createdAt}:${last.id}` : null,
      } as T;
    }
    case 'task_create': {
      const projectId = String(args.projectId ?? '');
      if (!projectId) throw new Error('请选择项目');
      const title = String(args.title ?? '').trim();
      if (!title) throw new Error('任务标题不能为空');
      const project = db.projects.find((p) => p.id === projectId);
      const team = args.teamId ? db.teams.find((t) => t.id === args.teamId) : null;
      const task: Task = {
        id: uid('task'),
        projectId,
        projectName: project?.name ?? null,
        teamId: team?.id ?? null,
        teamName: team?.name ?? null,
        parentId: args.parentId ? String(args.parentId) : null,
        title,
        description: args.description ? String(args.description) : null,
        acceptance: args.acceptance ? String(args.acceptance) : null,
        model: args.model ? String(args.model) : null,
        reasoning: args.reasoning ? String(args.reasoning) : null,
        mandate: args.mandate === 'auto' ? 'auto' : 'ask',
        workDir: null,
        reviewPending: false,
        status: 'inbox',
        priority: Number(args.priority ?? 2),
        assigneeSeatId: null,
        stageIndex: 0,
        stageKey: null,
        actualTokens: 0,
        actualCostMicros: 0,
        createdAt: now,
      };
      db.tasks.push(task);
      trackEvent('task.created', { title }, task.id);
      save();
      return task.id as T;
    }
    case 'task_update': {
      const task = db.tasks.find((t) => t.id === args.id);
      if (!task) throw new Error('任务不存在');
      const patch = (args.patch ?? {}) as Record<string, any>;
      if (patch.title != null) task.title = String(patch.title);
      if (patch.description != null) task.description = String(patch.description);
      if (patch.acceptance != null) task.acceptance = String(patch.acceptance);
      if (patch.priority != null) task.priority = Number(patch.priority);
      if (patch.status != null) task.status = String(patch.status);
      if (patch.teamId != null) {
        const team = db.teams.find((t) => t.id === patch.teamId);
        task.teamId = team?.id ?? null;
        task.teamName = team?.name ?? null;
      }
      if (task.status === 'completed') task.finishedAt = now;
      save();
      return undefined as T;
    }
    case 'task_delete': {
      const task = db.tasks.find((t) => t.id === args.id);
      if (task && isActiveStatus(task.status)) {
        throw new Error('任务正在运行，请先停止后再删除');
      }
      db.tasks = db.tasks.filter((t) => t.id !== args.id);
      db.messages = db.messages.filter((m) => m.taskId !== args.id);
      const runIds = new Set(db.runs.filter((r) => r.taskId === args.id).map((r) => r.id));
      db.toolCalls = db.toolCalls.filter((c) => !runIds.has(c.runId));
      db.runs = db.runs.filter((r) => r.taskId !== args.id);
      db.approvals = db.approvals.filter((a) => a.taskId !== args.id);
      db.reviews = db.reviews.filter((r) => r.taskId !== args.id);
      save();
      return undefined as T;
    }
    case 'task_dispatch': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (!task.teamId) throw new Error('请先为任务选择团队');
      if (task.status === 'running' || task.status === 'waiting_approval') {
        throw new Error('任务正在运行或等待审批，不能重复派发');
      }
      const restart = task.status === 'completed';
      task.status = 'queued';
      task.finishedAt = null;
      if (restart) task.stageIndex = 0;
      save();
      trackEvent('task.dispatched', { stageIndex: task.stageIndex }, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'queued' });
      simulateTask(task.id);
      return null as T;
    }
    case 'task_cancel': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      db.runs.forEach((r) => {
        if (r.taskId === task.id && (r.status === 'running' || r.status === 'starting')) {
          r.status = 'cancelled';
          r.endedAt = now;
        }
      });
      task.status = 'blocked';
      save();
      trackEvent('task.cancelled', {}, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'blocked' });
      return undefined as T;
    }
    case 'task_reset': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      task.status = 'inbox';
      task.stageIndex = 0;
      task.stageKey = null;
      task.startedAt = null;
      task.finishedAt = null;
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'inbox' });
      return undefined as T;
    }
    case 'task_pause': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (!isActiveStatus(task.status)) {
        throw new Error('只有运行中或等待审批的任务可以暂停');
      }
      db.runs.forEach((r) => {
        if (r.taskId === task.id && (r.status === 'running' || r.status === 'starting')) {
          r.status = 'cancelled';
          r.error = '用户暂停';
          r.endedAt = now;
        }
      });
      task.status = 'paused';
      save();
      trackEvent('task.paused', {}, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'paused' });
      return undefined as T;
    }
    case 'task_resume': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (task.status !== 'paused') throw new Error('只有已暂停的任务可以继续');
      if (!task.teamId) throw new Error('请先为任务选择团队');
      task.status = 'queued';
      task.finishedAt = null;
      save();
      trackEvent('task.resumed', { stageIndex: task.stageIndex }, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'queued' });
      simulateTask(task.id);
      return null as T;
    }
    case 'task_reassign': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (!canReassignStatus(task.status)) {
        throw new Error('只有暂停或阻塞的任务可以改派后重跑');
      }
      if (!task.teamId) throw new Error('请先为任务选择团队');
      const seats = db.seats
        .filter((s) => s.teamId === task.teamId)
        .sort((a, b) => a.orderIndex - b.orderIndex);
      const seat = seats[task.stageIndex];
      if (!seat) throw new Error('当前阶段没有对应席位');
      const name = agentNameById(db, String(args.agentId));
      if (!name) throw new Error('Agent 不存在');
      seat.agentId = String(args.agentId);
      seat.agentName = name;
      task.status = 'queued';
      task.finishedAt = null;
      save();
      trackEvent('task.reassigned', { stageIndex: task.stageIndex, agentName: name }, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'queued' });
      simulateTask(task.id);
      return null as T;
    }
    case 'task_children': {
      const taskId = String(args.taskId ?? '');
      return db.tasks.filter((t) => t.parentId === taskId) as T;
    }
    case 'task_dependencies': {
      const taskId = String(args.taskId ?? '');
      const items: TaskDependency[] = db.taskDeps
        .filter((d) => d.taskId === taskId)
        .map((d) => {
          const task = db.tasks.find((t) => t.id === d.dependsOnId);
          return {
            taskId,
            dependsOnId: d.dependsOnId,
            title: task?.title ?? '已删除任务',
            status: task?.status ?? 'inbox',
          };
        });
      return items as T;
    }
    case 'task_dependency_add': {
      const taskId = String(args.taskId ?? '');
      const dependsOnId = String(args.dependsOnId ?? '');
      if (taskId === dependsOnId) throw new Error('任务不能依赖自身');
      const task = db.tasks.find((t) => t.id === taskId);
      const dep = db.tasks.find((t) => t.id === dependsOnId);
      if (!task || !dep) throw new Error('任务不存在');
      if (task.projectId !== dep.projectId) throw new Error('只能依赖同一项目下的任务');
      const reaches = (from: string, target: string, seen = new Set<string>()): boolean => {
        if (from === target) return true;
        if (seen.has(from)) return false;
        seen.add(from);
        return db.taskDeps
          .filter((d) => d.taskId === from)
          .some((d) => reaches(d.dependsOnId, target, seen));
      };
      if (reaches(dependsOnId, taskId)) throw new Error('该依赖会形成循环');
      if (!db.taskDeps.some((d) => d.taskId === taskId && d.dependsOnId === dependsOnId)) {
        db.taskDeps.push({ taskId, dependsOnId });
        trackEvent('task.dependency_added', { dependsOnId }, taskId);
        save();
      }
      return undefined as T;
    }
    case 'task_dependency_remove': {
      const taskId = String(args.taskId ?? '');
      const dependsOnId = String(args.dependsOnId ?? '');
      db.taskDeps = db.taskDeps.filter(
        (d) => !(d.taskId === taskId && d.dependsOnId === dependsOnId),
      );
      trackEvent('task.dependency_removed', { dependsOnId }, taskId);
      save();
      return undefined as T;
    }
    case 'task_checklist_get': {
      const taskId = String(args.taskId ?? '');
      return (db.checklists[taskId] ?? []) as T;
    }
    case 'task_checklist_save': {
      const taskId = String(args.taskId ?? '');
      const raw = (args.items ?? []) as ChecklistItem[];
      const items = raw
        .map((item) => ({
          id: item.id || uid('item'),
          text: String(item.text ?? '').trim(),
          done: Boolean(item.done),
        }))
        .filter((item) => item.text);
      db.checklists[taskId] = items;
      trackEvent('task.checklist_updated', { total: items.length }, taskId);
      save();
      return items as T;
    }
    case 'task_attachments': {
      const taskId = String(args.taskId ?? '');
      return db.attachments.filter((a) => a.taskId === taskId) as T;
    }
    case 'task_attachment_add': {
      const taskId = String(args.taskId ?? '');
      const source = String(args.sourcePath ?? '');
      if (!source) throw new Error('请选择文件');
      const name = source.split(/[\\/]/).pop() || 'attachment';
      const attachment: Attachment = {
        id: uid('att'),
        taskId,
        name,
        path: source,
        size: 0,
        createdAt: Date.now(),
      };
      db.attachments.push(attachment);
      trackEvent('task.attachment_added', { name }, taskId);
      save();
      return attachment as T;
    }
    case 'task_attachment_delete': {
      const id = String(args.id ?? '');
      db.attachments = db.attachments.filter((a) => a.id !== id);
      save();
      return undefined as T;
    }
    case 'task_attachment_reveal':
      return undefined as T;
    case 'task_retry_stage': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (task.status !== 'blocked') throw new Error('只有已阻塞的任务可以从当前阶段重试');
      if (!task.teamId) throw new Error('请先为任务选择团队');
      task.status = 'queued';
      task.finishedAt = null;
      save();
      trackEvent('task.retried', { stageIndex: task.stageIndex }, task.id);
      save();
      emitLocal('task.updated', { taskId: task.id, status: 'queued' });
      simulateTask(task.id);
      return null as T;
    }
    case 'task_review': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (!task.workDir) {
        return {
          available: false,
          reason: '未启用隔离工作区，改动直接写入项目目录',
          workDir: null,
          repoRoot: null,
          files: [],
          additions: 0,
          deletions: 0,
          pendingCommits: 0,
          canApply: false,
          truncated: false,
        } as T;
      }
      let review = db.reviews.find((r) => r.taskId === task.id);
      if (!review) {
        review = { taskId: task.id, files: [], pendingCommits: 0, diffs: {} };
        db.reviews.push(review);
        save();
      }
      const files = review.files.map((file) => ({ ...file }));
      const snapshot: ReviewSnapshot = {
        available: true,
        reason: null,
        workDir: task.workDir,
        repoRoot: db.projects.find((p) => p.id === task.projectId)?.path ?? null,
        files,
        additions: files.reduce((sum, file) => sum + file.additions, 0),
        deletions: files.reduce((sum, file) => sum + file.deletions, 0),
        pendingCommits: review.pendingCommits,
        canApply: review.pendingCommits > 0,
        truncated: false,
      };
      return snapshot as T;
    }
    case 'task_review_diff': {
      const review = db.reviews.find((r) => r.taskId === args.taskId);
      const path = String(args.path ?? '');
      if (!review) throw new Error('隔离工作区已不存在');
      const diff: ReviewDiff = {
        path,
        patch: review.diffs[path] ?? '（无差异）',
        truncated: false,
        binary: false,
      };
      return diff as T;
    }
    case 'task_review_revert': {
      const review = db.reviews.find((r) => r.taskId === args.taskId);
      if (!review) throw new Error('隔离工作区已不存在');
      review.files = review.files.filter((file) => file.path !== args.path);
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (task) {
        task.reviewPending = review.files.length > 0 || review.pendingCommits > 0;
      }
      save();
      return undefined as T;
    }
    case 'task_review_revert_all': {
      const review = db.reviews.find((r) => r.taskId === args.taskId);
      if (!review) throw new Error('隔离工作区已不存在');
      review.files = [];
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (task) task.reviewPending = review.pendingCommits > 0;
      save();
      return undefined as T;
    }
    case 'task_review_commit': {
      const review = db.reviews.find((r) => r.taskId === args.taskId);
      if (!review) throw new Error('隔离工作区已不存在');
      if (review.files.length === 0) throw new Error('没有可提交的改动');
      review.files = [];
      review.pendingCommits += 1;
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (task) task.reviewPending = true;
      save();
      trackEvent('task.reviewed', { pendingCommits: review.pendingCommits }, String(args.taskId));
      save();
      return 'c0mm17a' as T;
    }
    case 'task_review_apply': {
      const review = db.reviews.find((r) => r.taskId === args.taskId);
      if (!review) throw new Error('隔离工作区已不存在');
      if (review.pendingCommits <= 0) throw new Error('没有待应用的提交');
      review.pendingCommits = 0;
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (task) task.reviewPending = review.files.length > 0;
      save();
      return 'a991c3d' as T;
    }
    case 'task_review_cleanup': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      if (isActiveStatus(task.status)) {
        throw new Error('任务正在运行，请先停止后再操作隔离工作区');
      }
      task.workDir = null;
      task.reviewPending = false;
      db.reviews = db.reviews.filter((r) => r.taskId !== task.id);
      save();
      return undefined as T;
    }
    case 'task_review_branch_info': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      const available = Boolean(task.workDir);
      const current = available ? `lumen/${task.id.slice(0, 6)}` : null;
      return {
        available,
        reason: available ? null : '分支操作仅用于隔离工作区',
        current,
        detached: false,
        branches: current ? ['main', current] : [],
      } as T;
    }
    case 'task_review_branch_create':
    case 'task_review_branch_switch':
      return String(args.name ?? '') as T;
    case 'task_snapshots':
      return [] as T;
    case 'task_snapshot_rollback':
      return '已回滚到快照（预览模式）' as T;
    case 'task_review_pr':
      return {
        mode: 'manual',
        url: null,
        command:
          'git push -u origin lumen/task\ngh pr create --head lumen/task --title "Lumen 任务交付" --body ""',
        message: '预览模式未调用 gh；请手动执行命令。',
      } as T;
    case 'task_add_note': {
      const task = db.tasks.find((t) => t.id === args.taskId);
      if (!task) throw new Error('任务不存在');
      const content = String(args.content ?? '').trim();
      if (!content) throw new Error('内容不能为空');
      const message: Message = {
        id: uid('m'),
        taskId: task.id,
        runId: null,
        seatId: null,
        agentId: null,
        seatName: null,
        agentName: null,
        kind: 'chat',
        role: 'user',
        content,
        metadata: {
          source: 'user',
          duringRun: isActiveStatus(task.status),
        },
        createdAt: now,
      };
      db.messages.push(message);
      save();
      emitLocal('run.delta', {
        runId: '',
        taskId: task.id,
        kind: 'message',
        message,
      });
      return undefined as T;
    }
    case 'task_messages':
      return db.messages
        .filter((m) => m.taskId === args.taskId)
        .sort((a, b) => a.createdAt - b.createdAt)
        .slice(-500) as T;
    case 'tool_calls_list': {
      const runIds = new Set(
        db.runs.filter((r) => r.taskId === args.taskId).map((r) => r.id),
      );
      const limit = Number(args.limit ?? 200);
      return db.toolCalls
        .filter((c) => runIds.has(c.runId))
        .sort((a, b) => a.createdAt - b.createdAt)
        .slice(-limit) as T;
    }
    case 'runs_list': {
      const limit = Number(args.limit ?? 100);
      let list = [...db.runs];
      if (args.taskId) list = list.filter((r) => r.taskId === args.taskId);
      return list.sort((a, b) => b.startedAt - a.startedAt).slice(0, limit) as T;
    }
    case 'run_prompt': {
      const run = db.runs.find((r) => r.id === args.runId);
      if (!run) return null as T;
      const task = db.tasks.find((t) => t.id === run.taskId);
      return `【Mock 提示词快照】\n项目：${task?.projectName ?? '未关联'}\n任务：${task?.title ?? '未知任务'}\n席位：${run.seatName ?? '未知'}` as T;
    }
    case 'run_cancel': {
      const run = db.runs.find((r) => r.id === args.runId);
      if (!run) throw new Error('运行记录不存在');
      run.status = 'cancelled';
      run.endedAt = now;
      const task = db.tasks.find((t) => t.id === run.taskId);
      if (task) task.status = 'blocked';
      save();
      emitLocal('run.status', { runId: run.id, status: 'cancelled', taskId: run.taskId });
      return undefined as T;
    }
    case 'run_transcript': {
      const run = db.runs.find((r) => r.id === args.runId);
      const messages = db.messages.filter((m) => m.runId === args.runId);
      const lines: string[] = [];
      if (run) {
        lines.push(
          JSON.stringify({
            type: 'run',
            runId: run.id,
            status: run.status,
            seat: run.seatName,
            agent: run.agentName,
            startedAt: run.startedAt,
          }),
        );
      }
      messages.forEach((m) => {
        lines.push(
          JSON.stringify({
            type: m.kind,
            role: m.role,
            seat: m.seatName,
            agent: m.agentName,
            content: m.content,
            at: m.createdAt,
          }),
        );
      });
      if (run) {
        lines.push(
          JSON.stringify({
            type: 'usage',
            inputTokens: run.inputTokens,
            outputTokens: run.outputTokens,
            costMicros: run.costMicros,
          }),
        );
      }
      const limit = Number(args.limit ?? 200);
      return lines.slice(-limit) as T;
    }
    case 'usage_summary':
      return summaryOf(db) as T;
    case 'usage_series':
      return seriesOf(db, Number(args.days ?? 7)) as T;
    case 'usage_by_agent':
      return scopeUsage(db, 'agentId', (id) => agentNameById(db, id) || id) as T;
    case 'usage_by_project':
      return scopeUsage(
        db,
        'projectId',
        (id) => db.projects.find((p) => p.id === id)?.name ?? id,
      ) as T;
    case 'usage_by_task': {
      let list = scopeUsage(
        db,
        'taskId',
        (id) => db.tasks.find((t) => t.id === id)?.title ?? `任务 ${id.slice(-4)}`,
      );
      if (args.projectId) {
        const ids = new Set(db.tasks.filter((t) => t.projectId === args.projectId).map((t) => t.id));
        list = list.filter((u) => ids.has(u.key));
      }
      return list as T;
    }
    case 'budgets_list':
      return db.budgets.map((b) => computeBudget(db, b)) as T;
    case 'budget_upsert': {
      const limitValue = Number(args.limitValue ?? 0);
      if (limitValue <= 0) throw new Error('预算上限必须大于 0');
      if (args.scopeType !== 'global' && !args.scopeId) throw new Error('请选择预算作用对象');
      const scopeName = (() => {
        switch (args.scopeType) {
          case 'project':
            return db.projects.find((p) => p.id === args.scopeId)?.name ?? '已删除项目';
          case 'team':
            return db.teams.find((t) => t.id === args.scopeId)?.name ?? '已删除团队';
          case 'agent':
            return agentNameById(db, args.scopeId) || '已删除 Agent';
          case 'task':
            return db.tasks.find((t) => t.id === args.scopeId)?.title ?? '已删除任务';
          default:
            return '全局';
        }
      })();
      const existing = args.id ? db.budgets.find((b) => b.id === args.id) : undefined;
      const payload: Budget = {
        id: existing?.id ?? uid('b'),
        scopeType: String(args.scopeType ?? 'global'),
        scopeId: args.scopeType === 'global' ? null : (args.scopeId ?? null),
        scopeName,
        unit: String(args.unit ?? 'tokens'),
        period: String(args.period ?? 'month'),
        limitValue,
        warnPct: Number(args.warnPct ?? 80),
        hardStop: Boolean(args.hardStop),
        enabled: existing?.enabled ?? true,
        spent: 0,
        reserved: 0,
        pct: 0,
        blocked: false,
        createdAt: existing?.createdAt ?? now,
      };
      if (existing) db.budgets = db.budgets.map((b) => (b.id === existing.id ? payload : b));
      else db.budgets.unshift(payload);
      save();
      return payload.id as T;
    }
    case 'budget_delete': {
      db.budgets = db.budgets.filter((b) => b.id !== args.id);
      save();
      return undefined as T;
    }
    case 'approvals_list': {
      let list = [...db.approvals];
      if (args.status) list = list.filter((a) => a.status === args.status);
      return list
        .sort((a, b) => b.createdAt - a.createdAt)
        .map((a) => ({
          ...a,
          taskTitle: db.tasks.find((t) => t.id === a.taskId)?.title ?? a.taskTitle ?? null,
        })) as T;
    }
    case 'approvals_pending':
      return db.approvals.filter((a) => a.status === 'pending').length as T;
    case 'approval_decide': {
      const approval = db.approvals.find((a) => a.id === args.approvalId);
      if (!approval) throw new Error('审批不存在');
      if (approval.status !== 'pending') throw new Error('该审批已处理');
      approval.status = args.approve ? 'approved' : 'denied';
      approval.decidedBy = 'user';
      approval.decisionNote = args.note ? String(args.note) : '';
      approval.decidedAt = now;

      if (args.remember && approval.type === 'tool_permission') {
        const detail = approval.detail as Record<string, any>;
        const tool = detail?.toolName;
        if (tool) {
          const matcher: Record<string, unknown> = { tool };
          const command = detail?.toolInput?.command;
          if (tool === 'Bash' && typeof command === 'string') {
            matcher.commandContains = command.split(/\s+/).slice(0, 2).join(' ');
          }
          db.rules.unshift({
            id: uid('r'),
            scopeType: 'global',
            scopeId: null,
            match: matcher,
            action: args.approve ? 'allow' : 'deny',
            priority: 10,
            enabled: true,
            createdAt: now,
          });
        }
      }

      if (approval.type === 'stage_gate' && approval.taskId) {
        const task = db.tasks.find((t) => t.id === approval.taskId);
        const detail = approval.detail as Record<string, any>;
        if (task) {
          if (args.approve) {
            task.status = 'running';
            task.stageIndex = Number(detail?.nextStageIndex ?? task.stageIndex + 1);
          } else {
            task.status = 'blocked';
          }
          emitLocal('task.updated', {
            taskId: task.id,
            status: task.status,
            stageIndex: task.stageIndex,
          });
        }
      }
      save();
      trackEvent(
        'approval.decided',
        { approvalId: approval.id, approved: Boolean(args.approve), remember: Boolean(args.remember) },
        approval.taskId,
      );
      save();
      emitLocal('approval.decided', { approvalId: approval.id, approved: Boolean(args.approve) });
      emitLocal('approvals.changed', {});
      return undefined as T;
    }
    case 'approval_rules_list':
      return [...db.rules].sort((a, b) => a.priority - b.priority) as T;
    case 'approval_rule_upsert': {
      const action = String(args.action ?? 'ask');
      if (!['allow', 'deny', 'ask'].includes(action)) {
        throw new Error('规则动作必须是 allow/deny/ask');
      }
      const matcher = (args.matcher ?? {}) as Record<string, unknown>;
      if (!matcher.tool && !matcher.commandContains && !matcher.pathGlob) {
        throw new Error('请至少填写工具、命令或路径条件');
      }
      const existing = args.id ? db.rules.find((r) => r.id === args.id) : undefined;
      const rule: ApprovalRule = {
        id: existing?.id ?? uid('r'),
        scopeType: 'global',
        scopeId: null,
        match: matcher,
        action,
        priority: Number(args.priority ?? 100),
        enabled: existing?.enabled ?? true,
        createdAt: existing?.createdAt ?? now,
      };
      if (existing) db.rules = db.rules.map((r) => (r.id === existing.id ? rule : r));
      else db.rules.unshift(rule);
      save();
      return rule.id as T;
    }
    case 'approval_rule_toggle': {
      const rule = db.rules.find((r) => r.id === args.id);
      if (!rule) throw new Error('规则不存在');
      rule.enabled = Boolean(args.enabled);
      save();
      return undefined as T;
    }
    case 'approval_rule_delete': {
      db.rules = db.rules.filter((r) => r.id !== args.id);
      save();
      return undefined as T;
    }
    case 'events_list': {
      let list = [...db.events];
      if (args.projectId) list = list.filter((e) => e.projectId === args.projectId);
      if (args.taskId) list = list.filter((e) => e.taskId === args.taskId);
      return list
        .sort((a, b) => b.seq - a.seq)
        .slice(0, Number(args.limit ?? 100)) as T;
    }
    case 'search_history': {
      const query = String(args.query ?? '').trim().toLowerCase();
      if (!query) return [] as T;
      return db.messages
        .filter((m) => m.content.toLowerCase().includes(query))
        .sort((a, b) => b.createdAt - a.createdAt)
        .slice(0, Number(args.limit ?? 80)) as T;
    }
    case 'dashboard_summary': {
      const agentStats: AgentStat[] = db.agents.map((agent) => {
        const events = db.usageEvents.filter((e) => e.agentId === agent.id);
        const todayStart = startOfToday();
        return {
          id: agent.id,
          name: agent.name,
          adapter: agent.adapter,
          status: agent.status,
          tokens: events.reduce((sum, e) => sum + e.tokens, 0),
          todayTokens: events
            .filter((e) => e.occurredAt >= todayStart)
            .reduce((sum, e) => sum + e.tokens, 0),
          costMicros: events.reduce((sum, e) => sum + e.costMicros, 0),
          rounds: events.reduce((sum, e) => sum + e.rounds, 0),
        };
      });
      const summary: DashboardSummary = {
        usage: summaryOf(db),
        pendingApprovals: db.approvals.filter((a) => a.status === 'pending').length,
        pendingReviews: db.tasks.filter(
          (t) =>
            t.reviewPending &&
            !isActiveStatus(t.status),
        ).length,
        agents: agentStats,
        teams: db.teams.slice(0, 5).map((t) => ({
          ...t,
          projectName: db.projects.find((p) => p.id === t.projectId)?.name ?? null,
        })),
        series: seriesOf(db, 7),
      };
      return summary as T;
    }
    default:
      throw new Error(`未实现的命令：${cmd}`);
  }
}

function simulateTask(taskId: string) {
  const step = (fn: () => void, ms: number) => window.setTimeout(fn, ms);

  step(() => {
    const db = load();
    const task = db.tasks.find((t) => t.id === taskId);
    if (!task || task.status !== 'queued') return;
    task.status = 'running';
    task.startedAt = Date.now();
    const seats = db.seats
      .filter((s) => s.teamId === task.teamId)
      .sort((a, b) => a.orderIndex - b.orderIndex);
    const seat = seats[task.stageIndex] ?? seats[0];
    if (!seat) return;
    task.stageKey = seat.roleKey;
    task.assigneeSeatId = seat.id;
    const run: Run = {
      id: uid('run'),
      taskId,
      seatId: seat.id,
      agentId: seat.agentId,
      agentName: seat.agentName,
      seatName: seat.roleName,
      stageKey: seat.roleKey,
      status: 'running',
      externalSessionId: uid('session'),
      isolation: 'process',
      inputTokens: 0,
      outputTokens: 0,
      costMicros: 0,
      error: null,
      startedAt: Date.now(),
    };
    db.runs.push(run);
    save();

    const message: Message = {
      id: uid('m'),
      taskId,
      runId: run.id,
      seatId: seat.id,
      agentId: seat.agentId,
      seatName: seat.roleName,
      agentName: seat.agentName,
      kind: 'chat',
      role: 'assistant',
      content: `## ${seat.roleName}已接手\n\n正在按团队流程处理「${task.title}」，将围绕职责范围输出结论。`,
      metadata: { source: 'agent' },
      createdAt: Date.now(),
    };
    db.messages.push(message);
    save();
    emitLocal('run.status', { runId: run.id, status: 'running', taskId });
    emitLocal('run.delta', { runId: run.id, taskId, kind: 'message', message });
    emitLocal('task.updated', { taskId, status: 'running', stageIndex: task.stageIndex });
  }, 500);

  step(() => {
    const db = load();
    const task = db.tasks.find((t) => t.id === taskId);
    if (!task || task.status !== 'running') return;
    const run = db.runs.filter((r) => r.taskId === taskId).sort((a, b) => b.startedAt - a.startedAt)[0];
    const seat = db.seats.find((s) => s.roleKey === task.stageKey && s.teamId === task.teamId);
    const inputTokens = 12_000 + Math.floor(Math.random() * 8_000);
    const outputTokens = 3_000 + Math.floor(Math.random() * 4_000);
    const costMicros = Math.round((inputTokens + outputTokens) * 4.2);
    const message: Message = {
      id: uid('m'),
      taskId,
      runId: run?.id ?? null,
      seatId: seat?.id ?? null,
      agentId: seat?.agentId ?? null,
      seatName: seat?.roleName ?? null,
      agentName: seat?.agentName ?? null,
      kind: 'chat',
      role: 'assistant',
      content:
        '### 阶段产出\n\n- 已完成关键路径梳理\n- 输出了可执行的检查清单\n- 标注了 2 个待确认风险点\n\n本阶段工作已完成，等待下一步指令或门禁审批。',
      metadata: { source: 'agent' },
      createdAt: Date.now(),
    };
    db.messages.push(message);
    if (run) {
      run.inputTokens = inputTokens;
      run.outputTokens = outputTokens;
      run.costMicros = costMicros;
      run.status = 'completed';
      run.endedAt = Date.now();
    }
    task.actualTokens += inputTokens + outputTokens;
    task.actualCostMicros += costMicros;
    task.status = 'completed';
    task.finishedAt = Date.now();
    const review = db.reviews.find((r) => r.taskId === task.id);
    task.reviewPending = Boolean(
      task.workDir && review && (review.files.length > 0 || review.pendingCommits > 0),
    );
    db.usageEvents.push({
      id: uid('u'),
      agentId: seat?.agentId ?? db.agents[0]?.id ?? '',
      projectId: task.projectId,
      teamId: task.teamId ?? '',
      taskId: task.id,
      tokens: inputTokens + outputTokens,
      inputTokens,
      outputTokens,
      costMicros,
      rounds: 1,
      occurredAt: Date.now(),
    });
    save();
    if (run) emitLocal('run.delta', { runId: run.id, taskId, kind: 'message', message });
    emitLocal('run.status', { runId: run?.id ?? '', status: 'completed', taskId });
    emitLocal('task.updated', { taskId, status: 'completed', stageIndex: task.stageIndex });
    emitLocal('usage.updated', { taskId });
  }, 2600);
}

export function resetMockData() {
  data = null;
  localStorage.removeItem(KEY);
  load();
}
