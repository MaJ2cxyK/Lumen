import { isTauri } from './env';
import { tr } from './i18n';
import { mockInvoke } from './mock';
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
  CliVersionInfo,
  EventItem,
  ExtensionHealth,
  ExtensionItem,
  GitState,
  Message,
  Project,
  Provider,
  ProviderTestResult,
  PrOutcome,
  ReviewBranchInfo,
  ReviewDiff,
  ReviewSnapshot,
  Run,
  SandboxProbeReport,
  ScopedUsage,
  Seat,
  SnapshotItem,
  Task,
  TaskDependency,
  TaskPage,
  Team,
  TeamTemplate,
  ToolCall,
  UsagePoint,
  UsageSummary,
} from './types';

function clean(args: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) {
    if (value !== undefined) out[key] = value;
  }
  return out;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const payload = clean(args ?? {});
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, payload);
  }
  return mockInvoke<T>(cmd, payload);
}

export interface BudgetInput {
  id?: string;
  scopeType: string;
  scopeId?: string | null;
  unit: string;
  period: string;
  limitValue: number;
  warnPct?: number;
  hardStop: boolean;
  enabled?: boolean;
}

export interface RuleInput {
  id?: string;
  matcher: Record<string, unknown>;
  action: string;
  priority?: number;
}

export const api = {
  appInfo: () => call<AppInfo>('app_info'),
  setBackendLang: (lang: 'zh' | 'en') => call<void>('set_backend_lang', { lang }),
  settingsGet: () => call<AppSettings>('app_settings_get'),
  settingsSave: (settings: AppSettings) => call<void>('app_settings_save', { settings }),
  openDataDir: () => call<void>('open_data_dir'),
  dataExport: () => call<string>('data_export'),
  dataBackup: () => call<string>('data_backup'),
  backupsList: () => call<BackupFile[]>('backups_list'),
  dataRestore: (name: string) => call<void>('data_restore', { name }),
  openBackupDir: () => call<void>('open_backup_dir'),
  cliDetect: (withVersion = true) => call<DetectedCli[]>('cli_detect', { withVersion }),
  cliInstall: (name: string) => call<string>('cli_install', { name }),
  cliLatestVersions: (names: string[]) => call<CliVersionInfo[]>('cli_latest_versions', { names }),
  sandboxProbe: () => call<SandboxProbeReport>('sandbox_probe'),

  agentsList: () => call<Agent[]>('agents_list'),
  agentCreate: (input: {
    name: string;
    adapter: string;
    command: string;
    model?: string;
    args?: string[];
    providerId?: string;
  }) => call<string>('agent_create', { ...input }),
  agentUpdate: (input: {
    id: string;
    name: string;
    command: string;
    model?: string;
    args?: string[];
    providerId?: string;
  }) => call<void>('agent_update', { ...input }),
  agentDelete: (id: string) => call<void>('agent_delete', { id }),

  providersList: () => call<Provider[]>('providers_list'),
  providerUpsert: (input: {
    id?: string;
    name: string;
    protocol: string;
    baseUrl: string;
    model?: string;
    models?: string[];
    apiKey?: string;
    enabled?: boolean;
  }) => call<string>('provider_upsert', { ...input, enabled: input.enabled ?? true }),
  providerDelete: (id: string) => call<void>('provider_delete', { id }),
  providerTest: (id: string) => call<ProviderTestResult>('provider_test', { id }),
  providerModels: (input: {
    id?: string;
    protocol: string;
    baseUrl: string;
    apiKey?: string;
  }) => call<string[]>('provider_models', input),

  extensionsList: () => call<ExtensionItem[]>('extensions_list'),
  extensionUpsert: (input: {
    id?: string;
    kind: string;
    name: string;
    description?: string;
    config: Record<string, unknown>;
    enabled?: boolean;
  }) => call<string>('extension_upsert', { ...input, enabled: input.enabled ?? true }),
  extensionToggle: (id: string, enabled: boolean) =>
    call<void>('extension_toggle', { id, enabled }),
  extensionDelete: (id: string) => call<void>('extension_delete', { id }),
  extensionsHealth: () => call<ExtensionHealth[]>('extensions_health'),

  projectsList: () => call<Project[]>('projects_list'),
  projectGitCheck: (path: string) => call<GitState>('project_git_check', { path }),
  projectCreate: (input: {
    name: string;
    path: string;
    kind: string;
    description?: string;
  }) => call<string>('project_create', { ...input }),
  projectUpdate: (input: {
    id: string;
    name: string;
    path: string;
    kind: string;
    description?: string;
  }) => call<void>('project_update', { ...input }),
  projectDelete: (id: string) => call<void>('project_delete', { id }),

  teamTemplates: () => call<TeamTemplate[]>('team_templates'),
  teamsList: (projectId?: string) => call<Team[]>('teams_list', { projectId }),
  teamCreate: (input: {
    projectId: string;
    name: string;
    templateKey: string;
    description?: string;
  }) => call<string>('team_create', { ...input }),
  teamDelete: (id: string) => call<void>('team_delete', { id }),

  seatsList: (teamId: string) => call<Seat[]>('seats_list', { teamId }),
  seatAssign: (seatId: string, agentId?: string | null) =>
    call<void>('seat_assign', { seatId, agentId: agentId ?? null }),
  seatUpdate: (input: {
    seatId: string;
    roleName: string;
    responsibility?: string;
    gate: boolean;
  }) => call<void>('seat_update', { ...input }),
  seatReorder: (teamId: string, seatIds: string[]) =>
    call<void>('seat_reorder', { teamId, seatIds }),
  seatAutoAssign: (teamId: string) => call<number>('seat_auto_assign', { teamId }),

  tasksList: (filter?: { projectId?: string; status?: string }) =>
    call<Task[]>('tasks_list', { ...filter }),
  tasksPage: (filter?: {
    projectId?: string;
    status?: string;
    query?: string;
    attentionOnly?: boolean;
    cursor?: string;
    limit?: number;
  }) => call<TaskPage>('tasks_page', { ...filter }),
  taskGet: (taskId: string) => call<Task>('task_get', { id: taskId }),
  taskCreate: (input: {
    projectId: string;
    teamId?: string | null;
    title: string;
    description?: string;
    acceptance?: string;
    model?: string;
    reasoning?: string;
    mandate?: string;
    isolated?: boolean;
    priority?: number;
    parentId?: string | null;
  }) => call<string>('task_create', { ...input }),
  taskUpdate: (
    id: string,
    patch: {
      title?: string;
      description?: string;
      acceptance?: string;
      priority?: number;
      status?: string;
      teamId?: string;
    },
  ) => call<void>('task_update', { id, patch }),
  taskDelete: (id: string) => call<void>('task_delete', { id }),
  taskDispatch: (taskId: string) => call<string | null>('task_dispatch', { taskId }),
  taskCancel: (taskId: string) => call<void>('task_cancel', { taskId }),
  taskReset: (taskId: string) => call<void>('task_reset', { taskId }),
  taskPause: (taskId: string) => call<void>('task_pause', { taskId }),
  taskResume: (taskId: string) => call<string | null>('task_resume', { taskId }),
  taskReassign: (taskId: string, agentId: string) =>
    call<string | null>('task_reassign', { taskId, agentId }),
  taskRetryStage: (taskId: string) => call<string | null>('task_retry_stage', { taskId }),
  taskChildren: (taskId: string) => call<Task[]>('task_children', { taskId }),
  taskDependencies: (taskId: string) =>
    call<TaskDependency[]>('task_dependencies', { taskId }),
  taskDependencyAdd: (taskId: string, dependsOnId: string) =>
    call<void>('task_dependency_add', { taskId, dependsOnId }),
  taskDependencyRemove: (taskId: string, dependsOnId: string) =>
    call<void>('task_dependency_remove', { taskId, dependsOnId }),
  taskChecklistGet: (taskId: string) =>
    call<ChecklistItem[]>('task_checklist_get', { taskId }),
  taskChecklistSave: (taskId: string, items: ChecklistItem[]) =>
    call<ChecklistItem[]>('task_checklist_save', { taskId, items }),
  taskAttachments: (taskId: string) => call<Attachment[]>('task_attachments', { taskId }),
  taskAttachmentAdd: (taskId: string, sourcePath: string) =>
    call<Attachment>('task_attachment_add', { taskId, sourcePath }),
  taskAttachmentDelete: (id: string) => call<void>('task_attachment_delete', { id }),
  taskAttachmentReveal: (id: string) => call<void>('task_attachment_reveal', { id }),
  taskReview: (taskId: string) => call<ReviewSnapshot>('task_review', { taskId }),
  taskReviewDiff: (taskId: string, path: string) =>
    call<ReviewDiff>('task_review_diff', { taskId, path }),
  taskReviewRevert: (taskId: string, path: string) =>
    call<void>('task_review_revert', { taskId, path }),
  taskReviewRevertAll: (taskId: string) =>
    call<void>('task_review_revert_all', { taskId }),
  taskReviewCommit: (taskId: string, message: string) =>
    call<string>('task_review_commit', { taskId, message }),
  taskReviewApply: (taskId: string) => call<string>('task_review_apply', { taskId }),
  taskReviewCleanup: (taskId: string) => call<void>('task_review_cleanup', { taskId }),
  taskReviewBranchInfo: (taskId: string) =>
    call<ReviewBranchInfo>('task_review_branch_info', { taskId }),
  taskReviewBranchCreate: (taskId: string, name: string) =>
    call<string>('task_review_branch_create', { taskId, name }),
  taskReviewBranchSwitch: (taskId: string, name: string) =>
    call<string>('task_review_branch_switch', { taskId, name }),
  taskSnapshots: (taskId: string) => call<SnapshotItem[]>('task_snapshots', { taskId }),
  taskSnapshotRollback: (taskId: string, path: string) =>
    call<string>('task_snapshot_rollback', { taskId, path }),
  taskReviewPr: (taskId: string, title: string, body: string) =>
    call<PrOutcome>('task_review_pr', { taskId, title, body }),
  taskAddNote: (taskId: string, content: string) =>
    call<void>('task_add_note', { taskId, content }),
  taskMessages: (taskId: string, before?: number) =>
    call<Message[]>('task_messages', { taskId, before, limit: 200 }),
  toolCallsList: (taskId: string, limit = 200) =>
    call<ToolCall[]>('tool_calls_list', { taskId, limit }),

  runsList: (filter?: { taskId?: string; limit?: number }) =>
    call<Run[]>('runs_list', { ...filter }),
  runPrompt: (runId: string) => call<string | null>('run_prompt', { runId }),
  runCancel: (runId: string) => call<void>('run_cancel', { runId }),
  runTranscript: (runId: string, limit = 200) =>
    call<string[]>('run_transcript', { runId, limit }),

  usageSummary: () => call<UsageSummary>('usage_summary'),
  usageSeries: (days = 7) => call<UsagePoint[]>('usage_series', { days }),
  usageByAgent: () => call<ScopedUsage[]>('usage_by_agent'),
  usageByProject: () => call<ScopedUsage[]>('usage_by_project'),
  usageByTask: (projectId?: string) => call<ScopedUsage[]>('usage_by_task', { projectId }),

  budgetsList: () => call<Budget[]>('budgets_list'),
  budgetUpsert: (input: BudgetInput) => call<string>('budget_upsert', { ...input }),
  budgetDelete: (id: string) => call<void>('budget_delete', { id }),

  approvalsList: (status?: string) => call<Approval[]>('approvals_list', { status }),
  approvalsPending: () => call<number>('approvals_pending'),
  approvalDecide: (input: {
    approvalId: string;
    approve: boolean;
    note?: string;
    remember?: boolean;
  }) => call<void>('approval_decide', { ...input }),
  approvalRulesList: () => call<ApprovalRule[]>('approval_rules_list'),
  approvalRuleUpsert: (input: RuleInput) => call<string>('approval_rule_upsert', { ...input }),
  approvalRuleToggle: (id: string, enabled: boolean) =>
    call<void>('approval_rule_toggle', { id, enabled }),
  approvalRuleDelete: (id: string) => call<void>('approval_rule_delete', { id }),

  eventsList: (filter?: { projectId?: string; taskId?: string; limit?: number }) =>
    call<EventItem[]>('events_list', { ...filter }),
  searchHistory: (query: string, limit = 80) =>
    call<Message[]>('search_history', { query, limit }),

  dashboardSummary: () => call<DashboardSummary>('dashboard_summary'),
};

export type { AgentStat };

export async function pickDirectory(): Promise<string | null> {
  if (!isTauri) {
    return window.prompt(
      tr('输入项目目录路径', 'Enter the project directory path'),
      'D:\\Projects\\demo',
    );
  }
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === 'string' ? selected : null;
}

export async function pickFiles(): Promise<string[]> {
  if (!isTauri) {
    const value = window.prompt(
      tr(
        '输入附件文件路径（浏览器预览模式）',
        'Enter the attachment file path (browser preview)',
      ),
      'D:\\demo\\spec.md',
    );
    return value ? [value] : [];
  }
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({ directory: false, multiple: true });
  if (Array.isArray(selected)) {
    return selected.filter((value): value is string => typeof value === 'string');
  }
  return typeof selected === 'string' ? [selected] : [];
}

export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return tr('操作失败，请稍后重试', 'Something went wrong. Please try again.');
}
