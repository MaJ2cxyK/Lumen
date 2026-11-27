import { tr } from './i18n';

export interface ToneMeta {
  label: string;
  text: string;
  bg: string;
  dot: string;
}

function tone(zh: string, en: string, text: string, bg: string, dot: string): ToneMeta {
  return {
    get label() {
      return tr(zh, en);
    },
    text,
    bg,
    dot,
  };
}

function pairMap(entries: Record<string, [string, string]>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, [zh, en]] of Object.entries(entries)) {
    Object.defineProperty(out, key, {
      get: () => tr(zh, en),
      enumerable: true,
    });
  }
  return out;
}

function templateMeta(zhName: string, enName: string, zhDesc: string, enDesc: string) {
  return {
    get name() {
      return tr(zhName, enName);
    },
    get description() {
      return tr(zhDesc, enDesc);
    },
  };
}

export const RUN_STATUS: Record<string, ToneMeta> = {
  starting: tone('启动中', 'Starting', 'text-info', 'bg-info/10', 'bg-info'),
  running: tone('运行中', 'Running', 'text-primary', 'bg-primary/10', 'bg-primary'),
  completed: tone('已完成', 'Completed', 'text-success', 'bg-success/10', 'bg-success'),
  paused_budget: tone('预算暂停', 'Paused by budget', 'text-warn', 'bg-warn/10', 'bg-warn'),
  failed: tone('失败', 'Failed', 'text-danger', 'bg-danger/10', 'bg-danger'),
  cancelled: tone('已取消', 'Cancelled', 'text-muted', 'bg-soft', 'bg-muted'),
};

export const AGENT_STATUS: Record<string, ToneMeta> = {
  idle: tone('空闲', 'Idle', 'text-muted', 'bg-soft', 'bg-muted'),
  running: tone('运行中', 'Running', 'text-primary', 'bg-primary/10', 'bg-primary'),
  offline: tone('离线', 'Offline', 'text-muted', 'bg-soft', 'bg-muted/70'),
  error: tone('异常', 'Error', 'text-danger', 'bg-danger/10', 'bg-danger'),
};

export const PRIORITY: Record<number, { label: string; text: string; bg: string; dot: string }> = {
  1: tone('紧急', 'Urgent', 'text-danger', 'bg-danger/10', 'bg-danger'),
  2: tone('高', 'High', 'text-warn', 'bg-warn/10', 'bg-warn'),
  3: tone('普通', 'Normal', 'text-muted', 'bg-soft', 'bg-muted'),
  4: tone('低', 'Low', 'text-muted', 'bg-soft', 'bg-muted/70'),
};

export function priorityMeta(p: number) {
  return PRIORITY[p] ?? PRIORITY[3];
}

export const PROJECT_KINDS: { value: string; label: string }[] = [
  {
    value: 'app',
    get label() {
      return tr('桌面应用', 'Desktop app');
    },
  },
  {
    value: 'web',
    get label() {
      return tr('Web 应用', 'Web app');
    },
  },
  {
    value: 'service',
    get label() {
      return tr('服务端', 'Service');
    },
  },
  {
    value: 'research',
    get label() {
      return tr('数据 / 研究', 'Data / research');
    },
  },
  {
    value: 'ops',
    get label() {
      return tr('办公 / 运营', 'Office / ops');
    },
  },
  {
    value: 'other',
    get label() {
      return tr('其他', 'Other');
    },
  },
];

export function projectKindLabel(kind: string): string {
  return PROJECT_KINDS.find((k) => k.value === kind)?.label ?? kind;
}

export const ADAPTERS: { value: string; label: string }[] = [
  { value: 'claude-code', label: 'Claude Code' },
  { value: 'opencode', label: 'OpenCode' },
  {
    value: 'pty',
    get label() {
      return tr('PTY 终端', 'PTY terminal');
    },
  },
];

export function adapterLabel(adapter: string): string {
  return ADAPTERS.find((a) => a.value === adapter)?.label ?? adapter;
}

export const CAPABILITY_LABEL: Record<string, string> = pairMap({
  office: ['办公', 'Office'],
  code: ['代码', 'Code'],
  research: ['研究', 'Research'],
  review: ['审阅', 'Review'],
});

export function capabilityLabel(key: string): string {
  return CAPABILITY_LABEL[key] ?? key;
}

export const TEMPLATE_META: Record<string, { name: string; description: string }> = {
  'software-dev': templateMeta(
    '软件研发组',
    'Software team',
    '需求澄清、架构设计、编码实现、测试与交付',
    'Discovery, architecture, implementation, testing and delivery',
  ),
  'office-ops': templateMeta(
    '办公协作组',
    'Office team',
    '文档、资料整理与日常办公产出',
    'Documents, research notes and daily office deliverables',
  ),
  'quant-research': templateMeta(
    '量化研究组',
    'Quant research team',
    '研究假设、数据验证与最终结论',
    'Research hypotheses, data validation and conclusions',
  ),
};

const ROLE_META: Record<string, { name: string; description: string }> = {
  pm: templateMeta(
    '产品经理',
    'Product manager',
    '目标拆解、用户故事、功能边界与验收标准',
    'Goals, user stories, scope and acceptance criteria',
  ),
  architect: templateMeta(
    '系统架构师',
    'Solution architect',
    '技术方案、模块划分、接口契约与风险评估',
    'Technical design, modules, interface contracts and risk assessment',
  ),
  developer: templateMeta(
    '研发工程师',
    'Software engineer',
    '按架构落地代码、补充测试、修复问题',
    'Implements the design, adds tests and fixes issues',
  ),
  qa: templateMeta(
    '测试工程师',
    'QA engineer',
    '验证验收标准、回归风险、缺陷复现与报告',
    'Verifies acceptance criteria, regression risk and defect reports',
  ),
  delivery: templateMeta(
    '交付总监',
    'Delivery lead',
    '汇总产物、检查交付完整性、输出最终说明',
    'Consolidates outputs, checks completeness and writes the final brief',
  ),
  intake: templateMeta(
    '需求整理',
    'Intake',
    '澄清目标、受众、交付格式与截止时间',
    'Clarifies goals, audience, format and deadlines',
  ),
  researcher: templateMeta(
    '资料研究员',
    'Researcher',
    '收集与核对资料、提炼要点、标注来源',
    'Collects and verifies sources, extracts key points',
  ),
  writer: templateMeta(
    '文档工程师',
    'Technical writer',
    '按目标撰写、整理文档、表格或演示材料',
    'Writes docs, sheets or slides to the agreed brief',
  ),
  reviewer: templateMeta(
    '审核交付',
    'Reviewer',
    '检查准确性、完整性与格式，输出交付说明',
    'Checks accuracy, completeness and format, then writes the handoff',
  ),
  scope: templateMeta(
    '投研需求',
    'Research scope',
    '明确研究标的、时间窗口、数据来源与输出格式',
    'Defines targets, horizons, data sources and output format',
  ),
  fundamental: templateMeta(
    '基本面分析师',
    'Fundamentals analyst',
    '财报与行业数据、关键指标与估值假设',
    'Financial reports, industry data, key metrics and valuation assumptions',
  ),
  technical: templateMeta(
    '量价分析师',
    'Market structure analyst',
    '价格、成交量与市场结构，识别趋势和关键区间',
    'Price, volume and market structure, trends and key ranges',
  ),
  risk: templateMeta(
    '风控专员',
    'Risk controller',
    '敞口测算、情景分析、仓位与止损约束',
    'Exposure, scenarios, position sizing and stop-loss limits',
  ),
  director: templateMeta(
    '研究主管',
    'Research lead',
    '汇总分歧、校验证据、形成研究结论',
    'Consolidates disagreements, checks evidence and concludes',
  ),
};

export function roleName(key: string | null | undefined, fallback: string): string {
  return (key && ROLE_META[key]?.name) || fallback;
}

export function roleDescription(key: string | null | undefined, fallback: string): string {
  return (key && ROLE_META[key]?.description) || fallback;
}

export const BUDGET_SCOPE: Record<string, string> = pairMap({
  global: ['全局', 'Global'],
  project: ['项目', 'Project'],
  team: ['团队', 'Team'],
  agent: ['Agent', 'Agent'],
  task: ['任务', 'Task'],
});

export const BUDGET_PERIOD: Record<string, string> = pairMap({
  day: ['每日', 'Daily'],
  month: ['每月', 'Monthly'],
  task: ['按任务', 'Per task'],
});

export const BUDGET_UNIT: Record<string, string> = pairMap({
  tokens: ['Tokens', 'Tokens'],
  usd: ['美元', 'USD'],
});

function approvalType(zh: string, en: string, text: string, bg: string, dot: string) {
  return {
    get label() {
      return tr(zh, en);
    },
    tone: tone(zh, en, text, bg, dot),
  };
}

export const APPROVAL_TYPE: Record<string, { label: string; tone: ToneMeta }> = {
  stage_gate: approvalType('阶段门禁', 'Stage gate', 'text-primary', 'bg-primary/10', 'bg-primary'),
  tool_permission: approvalType(
    '工具授权',
    'Tool permission',
    'text-warn',
    'bg-warn/10',
    'bg-warn',
  ),
  budget_exceeded: approvalType(
    '预算越限',
    'Budget exceeded',
    'text-danger',
    'bg-danger/10',
    'bg-danger',
  ),
  run_permission: approvalType(
    '运行授权',
    'Run permission',
    'text-warn',
    'bg-warn/10',
    'bg-warn',
  ),
};

export const RISK_META: Record<
  string,
  { label: string; text: string; bg: string; dot: string }
> = {
  low: tone('低风险', 'Low risk', 'text-success', 'bg-success/10', 'bg-success'),
  medium: tone('中风险', 'Medium risk', 'text-warn', 'bg-warn/10', 'bg-warn'),
  high: tone('高风险', 'High risk', 'text-danger', 'bg-danger/10', 'bg-danger'),
};

export const APPROVAL_STATUS: Record<string, ToneMeta> = {
  pending: tone('待处理', 'Pending', 'text-warn', 'bg-warn/10', 'bg-warn'),
  approved: tone('已批准', 'Approved', 'text-success', 'bg-success/10', 'bg-success'),
  denied: tone('已拒绝', 'Denied', 'text-danger', 'bg-danger/10', 'bg-danger'),
  expired: tone('已过期', 'Expired', 'text-muted', 'bg-soft', 'bg-muted/70'),
};

export function approvalTypeMeta(type: string) {
  return APPROVAL_TYPE[type]?.tone ?? {
    label: type,
    text: 'text-muted',
    bg: 'bg-soft',
    dot: 'bg-muted',
  };
}

export const EVENT_LABEL: Record<string, string> = pairMap({
  'project.created': ['项目已创建', 'Project created'],
  'task.created': ['任务已创建', 'Task created'],
  'task.dispatched': ['任务已派发', 'Task dispatched'],
  'task.cancelled': ['任务已取消', 'Task cancelled'],
  'task.paused': ['任务已暂停', 'Task paused'],
  'task.resumed': ['任务已继续', 'Task resumed'],
  'task.reassigned': ['任务已改派当前阶段', 'Task stage reassigned'],
  'task.dependency_added': ['已添加任务依赖', 'Dependency added'],
  'task.dependency_removed': ['已移除任务依赖', 'Dependency removed'],
  'task.checklist_updated': ['验收清单已更新', 'Checklist updated'],
  'task.attachment_added': ['已添加附件', 'Attachment added'],
  'task.attachment_removed': ['已删除附件', 'Attachment removed'],
  'task.retried': ['任务从当前阶段重试', 'Task retried from current stage'],
  'task.reviewed': ['隔离工作区改动已提交', 'Isolated changes committed'],
  'task.updated': ['任务状态更新', 'Task updated'],
  'run.started': ['运行开始', 'Run started'],
  'run.approved': ['运行已放行', 'Run approved'],
  'run.completed': ['运行完成', 'Run completed'],
  'run.failed': ['运行失败', 'Run failed'],
  'run.message': ['席位产出消息', 'Seat message'],
  'approval.requested': ['发起审批', 'Approval requested'],
  'approval.decided': ['审批已处理', 'Approval decided'],
  'budget.alert': ['预算预警', 'Budget alert'],
  'usage.updated': ['用量更新', 'Usage updated'],
  'seat.assigned': ['席位已指派', 'Seat assigned'],
});

export function eventLabel(type: string): string {
  return EVENT_LABEL[type] ?? type;
}

export function eventColor(type: string): string {
  if (type.startsWith('approval')) return 'bg-warn';
  if (type.startsWith('budget')) return 'bg-danger';
  if (type.startsWith('run')) return 'bg-primary';
  if (type.startsWith('task')) return 'bg-info';
  return 'bg-muted';
}
