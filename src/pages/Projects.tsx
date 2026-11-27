import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  AppWindow,
  ArrowUp,
  ArrowUpRight,
  Brain,
  Check,
  ChevronDown,
  ClipboardList,
  Cpu,
  FlaskConical,
  Folder,
  FolderOpen,
  Globe,
  Pencil,
  Plus,
  Server,
  ShieldCheck,
  Trash2,
  type LucideIcon,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage, pickDirectory } from '../lib/api';
import type { GitState, Project } from '../lib/types';
import { PROJECT_KINDS, projectKindLabel } from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import { useAutoGrow, useComposer } from '../lib/composer';
import {
  clearProjectDraft,
  loadProjectDraft,
  saveProjectDraft,
} from '../lib/drafts';
import { useAppStore } from '../store';
import {
  Badge,
  Button,
  ConfirmDialog,
  Field,
  Input,
  Modal,
  PageHeader,
  PageScroll,
  Select,
  Textarea,
  cn,
} from '../components/ui';

const KIND_ICON: Record<string, LucideIcon> = {
  app: AppWindow,
  web: Globe,
  service: Server,
  research: FlaskConical,
  ops: ClipboardList,
  other: Folder,
};

const KIND_TONE: Record<string, string> = {
  app: 'bg-primary/10 text-primary',
  web: 'bg-info/10 text-info',
  service: 'bg-success/10 text-success',
  research: 'bg-warn/10 text-warn',
  ops: 'bg-primary/10 text-primary',
  other: 'bg-soft text-muted',
};

const REASONING_OPTIONS = [
  {
    value: 'low',
    get label() {
      return tr('低', 'Low');
    },
    get hint() {
      return tr('快速作答，直接给结果', 'Fast answers, straight to the point');
    },
  },
  {
    value: 'medium',
    get label() {
      return tr('中', 'Medium');
    },
    get hint() {
      return tr('平衡深度与速度', 'Balanced depth and speed');
    },
  },
  {
    value: 'high',
    get label() {
      return tr('高', 'High');
    },
    get hint() {
      return tr('深入分析后再给结论', 'Analyze deeply before concluding');
    },
  },
];

interface FormState {
  name: string;
  path: string;
  kind: string;
  description: string;
}

const EMPTY_FORM: FormState = { name: '', path: '', kind: 'app', description: '' };

export default function Projects() {
  const navigate = useNavigate();
  const { t } = useI18n();
  const projects = useAppStore((s) => s.projects);
  const loaded = useAppStore((s) => s.loaded);
  const loadCore = useAppStore((s) => s.loadCore);

  const [text, setText] = useState('');
  const [projectId, setProjectId] = useState('');
  const [model, setModel] = useState('');
  const [reasoning, setReasoning] = useState('medium');
  const [mandate, setMandate] = useState('ask');
  const [isolated, setIsolated] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const keepComposer = useRef(false);
  const [models, setModels] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);

  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const [projectModalOpen, setProjectModalOpen] = useState(false);
  const [editing, setEditing] = useState<Project | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [reopenWorkspace, setReopenWorkspace] = useState(false);
  const [deleting, setDeleting] = useState<Project | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);
  const [gitCheck, setGitCheck] = useState<GitState | null>(null);
  const [checkingGit, setCheckingGit] = useState(false);
  const gitRequest = useRef(0);

  const selected = useMemo(
    () => projects.find((p) => p.id === projectId) ?? null,
    [projects, projectId],
  );
  const isolateUnavailable = Boolean(selected && selected.isGit === false);

  // 切到非 Git 项目时清掉隔离选项，避免创建时才被后端拒绝。
  useEffect(() => {
    if (isolateUnavailable && isolated) setIsolated(false);
  }, [isolateUnavailable, isolated]);

  const checkGit = useCallback(async (path: string) => {
    const trimmed = path.trim();
    const request = ++gitRequest.current;
    if (!trimmed) {
      setGitCheck(null);
      setCheckingGit(false);
      return;
    }
    setCheckingGit(true);
    try {
      const result = await api.projectGitCheck(trimmed);
      if (request === gitRequest.current) setGitCheck(result);
    } catch {
      if (request === gitRequest.current) setGitCheck(null);
    } finally {
      if (request === gitRequest.current) setCheckingGit(false);
    }
  }, []);

  useEffect(() => {
    if (projects.length === 0) {
      setProjectId('');
      return;
    }
    setProjectId((prev) => (projects.some((p) => p.id === prev) ? prev : projects[0].id));
  }, [projects]);

  const draftKey = projectId || 'new';

  // 切换项目时恢复对应草稿；刚新建项目时保留当前输入。
  useEffect(() => {
    if (keepComposer.current) {
      keepComposer.current = false;
      return;
    }
    const draft = loadProjectDraft(draftKey);
    setText(draft.text);
    setModel(draft.model);
    setReasoning(draft.reasoning);
    setMandate(draft.mandate);
    setIsolated(draft.isolated);
  }, [draftKey]);

  useEffect(() => {
    saveProjectDraft(draftKey, { text, model, reasoning, mandate, isolated });
  }, [draftKey, text, model, reasoning, mandate, isolated]);

  useAutoGrow(textareaRef, text, 240);
  const composer = useComposer(submitTask);

  const loadModels = useCallback(async () => {
    try {
      const [providers, agents] = await Promise.all([api.providersList(), api.agentsList()]);
      const set = new Set<string>();
      providers
        .filter((p) => p.enabled)
        .forEach((p) => {
          if (p.model) set.add(p.model);
          p.models.forEach((m) => set.add(m));
        });
      agents.forEach((a) => {
        if (a.model) set.add(a.model);
      });
      setModels(Array.from(set));
    } catch {
      // 模型列表加载失败不阻塞任务创建，仍可使用默认模型
    }
  }, []);

  useEffect(() => {
    loadModels();
  }, [loadModels]);

  function openCreate(fromWorkspace = false) {
    setReopenWorkspace(fromWorkspace);
    if (fromWorkspace) setWorkspaceOpen(false);
    setEditing(null);
    setForm(EMPTY_FORM);
    setGitCheck(null);
    setProjectModalOpen(true);
  }

  function openEdit(project: Project) {
    setReopenWorkspace(true);
    setWorkspaceOpen(false);
    setEditing(project);
    setForm({
      name: project.name,
      path: project.path,
      kind: project.kind,
      description: project.description ?? '',
    });
    setGitCheck({ isGit: project.isGit, reason: project.gitReason ?? null });
    setProjectModalOpen(true);
  }

  function closeProjectModal() {
    setProjectModalOpen(false);
    if (reopenWorkspace) setWorkspaceOpen(true);
    setReopenWorkspace(false);
  }

  async function choosePath() {
    try {
      const path = await pickDirectory();
      if (path) {
        setForm((f) => ({ ...f, path }));
        checkGit(path);
      }
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function submitProject() {
    if (!form.name.trim()) {
      toast.error(t('请填写项目名称', 'Enter a project name'));
      return;
    }
    if (!form.path.trim()) {
      toast.error(t('请选择项目目录', 'Choose a project directory'));
      return;
    }
    setSaving(true);
    try {
      if (editing) {
        await api.projectUpdate({
          id: editing.id,
          name: form.name.trim(),
          path: form.path.trim(),
          kind: form.kind,
          description: form.description.trim() || undefined,
        });
        toast.success(t('项目已更新', 'Project updated'));
      } else {
        const id = await api.projectCreate({
          name: form.name.trim(),
          path: form.path.trim(),
          kind: form.kind,
          description: form.description.trim() || undefined,
        });
        keepComposer.current = true;
        setProjectId(id);
        toast.success(t('项目已创建', 'Project created'));
      }
      setProjectModalOpen(false);
      await loadCore();
      if (reopenWorkspace) setWorkspaceOpen(true);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
      setReopenWorkspace(false);
    }
  }

  async function chooseDirectory() {
    setPicking(true);
    try {
      const path = await pickDirectory();
      if (!path) return;
      const normalized = path.replace(/[\\/]+$/, '').toLowerCase();
      const existing = projects.find(
        (p) => p.path.replace(/[\\/]+$/, '').toLowerCase() === normalized,
      );
      if (existing) {
        setProjectId(existing.id);
        setWorkspaceOpen(false);
        toast.success(t(`已选择「${existing.name}」`, `Selected “${existing.name}”`));
        return;
      }
      const name =
        path.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || t('未命名项目', 'Untitled project');
      const id = await api.projectCreate({ name, path, kind: 'app' });
      await loadCore();
      keepComposer.current = true;
      setProjectId(id);
      setWorkspaceOpen(false);
      toast.success(t(`已添加项目「${name}」`, `Added project “${name}”`));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setPicking(false);
    }
  }

  async function confirmDelete() {
    if (!deleting) return;
    setDeleteLoading(true);
    try {
      await api.projectDelete(deleting.id);
      toast.success(t('项目已删除', 'Project deleted'));
      setDeleting(null);
      await loadCore();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleteLoading(false);
    }
  }

  async function submitTask() {
    const content = text.trim();
    if (!content) {
      toast.error(t('请先描述任务', 'Describe the task first'));
      return;
    }
    const pid = projectId || projects[0]?.id || '';
    if (!pid) {
      toast.error(t('请先选择项目目录', 'Choose a project directory first'));
      setWorkspaceOpen(true);
      return;
    }
    const firstLine = content.split('\n').map((line) => line.trim()).find(Boolean) ?? content;
    const title = firstLine.length > 80 ? `${firstLine.slice(0, 79)}…` : firstLine;
    setSubmitting(true);
    try {
      const id = await api.taskCreate({
        projectId: pid,
        title,
        description: content,
        model: model || undefined,
        reasoning,
        mandate,
        isolated,
      });
      clearProjectDraft(pid);
      setText('');
      await loadCore();
      toast.success(
        t('任务已创建，可在任务看板分派团队', 'Task created. Assign a team on the board.'),
      );
      navigate(`/tasks/${id}`);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSubmitting(false);
    }
  }

  const canSend = Boolean(text.trim() && (projectId || projects.length > 0) && !submitting);

  return (
    <PageScroll>
      <PageHeader
        title={t('新建任务', 'New task')}
        subtitle={t(`已连接 ${projects.length} 个目录`, `${projects.length} directories connected`)}
        actions={
          <Button size="sm" variant="outline" onClick={() => openCreate(false)}>
            <Plus size={14} />
            {t('新建项目', 'New project')}
          </Button>
        }
      />

      <div className="mx-auto flex min-h-[calc(100vh-230px)] w-full max-w-[860px] flex-col justify-center pb-16">
        <div className="overflow-hidden rounded-2xl border border-line bg-card focus-within:border-primary/40">
          <textarea
            ref={textareaRef}
            value={text}
            onChange={(e) => setText(e.target.value)}
            {...composer}
            rows={1}
            autoFocus
            placeholder={t(
              '描述任务目标、范围与验收标准…\n例如：登录页支持手机号验证码登录，并补齐错误提示与限流',
              'Describe the goal, scope, and acceptance criteria…\ne.g. Add SMS-code sign-in, plus error hints and rate limiting',
            )}
            className="composer min-h-[88px] w-full resize-none bg-transparent px-4 pt-4 text-sm leading-6 text-ink outline-none placeholder:text-muted/55"
          />

          <div className="flex flex-wrap items-center gap-1.5 px-2.5 pb-2.5 pt-1">
            <button
              type="button"
              onClick={() => setWorkspaceOpen(true)}
              title={selected ? selected.path : t('选择项目目录', 'Choose project directory')}
              className="inline-flex h-7 max-w-[240px] items-center gap-1.5 rounded-lg border border-line/70 px-2 text-xs text-muted transition-colors hover:border-line hover:text-ink"
            >
              <Folder size={13} className="shrink-0" />
              <span className="truncate">
                {!loaded
                  ? t('正在加载项目…', 'Loading projects…')
                  : selected
                    ? selected.name
                    : t('选择项目目录', 'Choose project directory')}
              </span>
              <ChevronDown size={12} className="shrink-0 text-muted/70" />
            </button>

            <label
              className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-line/70 pl-2 pr-1 text-muted transition-colors hover:border-line"
              title={t('切换执行模型', 'Switch model')}
            >
              <Cpu size={13} className="shrink-0" />
              <select
                value={model}
                onChange={(e) => setModel(e.target.value)}
                className="select-caret h-6 max-w-[180px] cursor-pointer appearance-none bg-transparent pl-0 pr-6 text-xs text-ink outline-none"
              >
                <option value="">{t('默认模型', 'Default model')}</option>
                {models.map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </label>

            <label
              className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-line/70 pl-2 pr-1 text-muted transition-colors hover:border-line"
              title={t(
                '任务内自动授权可减少逐次审批打断',
                'Auto-approve within the task to reduce approval prompts',
              )}
            >
              <ShieldCheck size={13} className="shrink-0" />
              <select
                value={mandate}
                onChange={(e) => setMandate(e.target.value)}
                className="select-caret h-6 cursor-pointer appearance-none bg-transparent pl-0 pr-6 text-xs text-ink outline-none"
              >
                <option value="ask">{t('逐次审批', 'Approve each step')}</option>
                <option value="auto">{t('任务内自动', 'Auto within task')}</option>
              </select>
            </label>

            <label
              className={cn(
                'inline-flex h-7 items-center gap-1.5 rounded-lg border border-line/70 px-2 text-xs text-muted transition-colors hover:border-line hover:text-ink',
                isolateUnavailable && 'cursor-not-allowed opacity-60',
              )}
              title={
                isolateUnavailable
                  ? t(
                      '当前项目不是 Git 仓库，无法创建隔离工作区',
                      'This project is not a Git repo, so an isolated workspace is unavailable',
                    )
                  : t(
                      '仅 git 仓库有效：为任务创建独立 worktree',
                      'Git repos only: create a separate worktree for the task',
                    )
              }
            >
              <input
                type="checkbox"
                checked={isolated}
                disabled={isolateUnavailable}
                onChange={(e) => setIsolated(e.target.checked)}
                className="h-3 w-3 accent-primary"
              />
              {t('隔离工作区', 'Isolated workspace')}
            </label>

            <div
              className="flex h-7 items-center gap-1 rounded-lg border border-line/70 px-1 text-muted"
              title={t(
                '调整思考强度，影响 Agent 分析深度',
                'Adjust reasoning effort; affects how deeply the agent analyzes',
              )}
            >
              <Brain size={13} className="ml-1 shrink-0" />
              <div className="flex items-center rounded-md p-0.5">
                {REASONING_OPTIONS.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    title={option.hint}
                    onClick={() => setReasoning(option.value)}
                    className={cn(
                      'h-6 rounded-md px-2 text-xs font-medium transition-colors',
                      reasoning === option.value
                        ? 'bg-primary/12 text-primary'
                        : 'text-muted hover:text-ink',
                    )}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </div>

            <button
              type="button"
              onClick={submitTask}
              disabled={!canSend}
              title={t('创建任务（Enter）', 'Create task (Enter)')}
              className="ml-auto flex h-8 w-8 items-center justify-center rounded-full bg-primary text-white transition-opacity hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {submitting ? (
                <span className="h-3.5 w-3.5 animate-spin rounded-full border-[1.5px] border-white/40 border-t-white" />
              ) : (
                <ArrowUp size={16} />
              )}
            </button>
          </div>
        </div>

        <p className="mt-2.5 px-1 text-xs leading-5 text-muted/70">
          {t(
            'Enter 创建任务，Shift + Enter 换行。任务会先进入「待分派」，可在任务看板指定团队；模型与思考强度会随任务保存。',
            'Enter to create, Shift + Enter for a new line. Tasks start as “Unassigned”; pick a team on the board. Model and reasoning effort are saved with the task.',
          )}
        </p>
        {isolateUnavailable && (
          <p className="mt-1 px-1 text-xs leading-5 text-warn">
            {t(
              '当前项目不是 Git 仓库，无法创建隔离工作区；非隔离运行会直接修改项目目录。',
              'This project is not a Git repo, so an isolated workspace is unavailable. Non-isolated runs modify the project directory directly.',
            )}
          </p>
        )}
      </div>

      <Modal
        open={workspaceOpen}
        onClose={() => setWorkspaceOpen(false)}
        title={t('选择项目目录', 'Choose project directory')}
        description={t(
          '选择已有项目，或直接添加一个本地目录',
          'Pick an existing project, or add a local directory',
        )}
        width="max-w-xl"
        footer={
          <>
            <Button variant="outline" size="sm" loading={picking} onClick={chooseDirectory}>
              <FolderOpen size={14} />
              {t('选择本地目录', 'Choose local directory')}
            </Button>
            <Button size="sm" onClick={() => openCreate(true)}>
              <Plus size={14} />
              {t('新建项目', 'New project')}
            </Button>
          </>
        }
      >
        {projects.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line px-4 py-8 text-center">
            <Folder size={22} className="mx-auto text-muted/60" />
            <p className="mt-2 text-sm font-medium text-ink/80">{t('还没有项目', 'No projects yet')}</p>
            <p className="mt-1 text-xs text-muted">
              {t(
                '点击下方「选择本地目录」添加第一个项目',
                'Click “Choose local directory” below to add your first project',
              )}
            </p>
          </div>
        ) : (
          <div className="space-y-1">
            {projects.map((project) => {
              const Icon = KIND_ICON[project.kind] ?? Folder;
              const active = project.id === selected?.id;
              return (
                <div
                  key={project.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    setProjectId(project.id);
                    setWorkspaceOpen(false);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      setProjectId(project.id);
                      setWorkspaceOpen(false);
                    }
                  }}
                  className={cn(
                    'group flex cursor-pointer items-center gap-3 rounded-xl border px-3 py-2.5 transition-colors',
                    active
                      ? 'border-primary/40 bg-primary/5'
                      : 'border-transparent hover:border-line hover:bg-hover',
                  )}
                >
                  <span
                    className={cn(
                      'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg',
                      KIND_TONE[project.kind] ?? KIND_TONE.other,
                    )}
                  >
                    <Icon size={15} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-ink">{project.name}</p>
                    <p className="mt-0.5 truncate font-mono text-xs text-muted/70">
                      {project.path}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <Badge tone={project.isGit ? 'success' : 'warn'}>
                      {project.isGit ? 'Git' : t('非 Git', 'Not Git')}
                    </Badge>
                    <Badge>{projectKindLabel(project.kind)}</Badge>
                    <Button
                      variant="ghost"
                      size="xs"
                      title={t('查看项目详情', 'View project details')}
                      onClick={(e) => {
                        e.stopPropagation();
                        setWorkspaceOpen(false);
                        navigate(`/projects/${project.id}`);
                      }}
                    >
                      <ArrowUpRight size={13} />
                    </Button>
                    <div className="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                      <Button
                        variant="ghost"
                        size="xs"
                        onClick={(e) => {
                          e.stopPropagation();
                          openEdit(project);
                        }}
                        title={t('编辑', 'Edit')}
                      >
                        <Pencil size={13} />
                      </Button>
                      <Button
                        variant="ghost"
                        size="xs"
                        className="hover:text-danger"
                        onClick={(e) => {
                          e.stopPropagation();
                          setDeleting(project);
                        }}
                        title={t('删除', 'Delete')}
                      >
                        <Trash2 size={13} />
                      </Button>
                    </div>
                    {active && <Check size={15} className="text-primary" />}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Modal>

      <Modal
        open={projectModalOpen}
        onClose={closeProjectModal}
        title={editing ? t('编辑项目', 'Edit project') : t('新建项目', 'New project')}
        description={t(
          '项目目录用于 Agent 执行时的默认工作路径',
          'The directory used as the agent default working path',
        )}
        footer={
          <>
            <Button variant="outline" size="sm" onClick={closeProjectModal}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={submitProject}>
              {editing ? t('保存', 'Save') : t('创建', 'Create')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <Field label={t('项目名称', 'Project name')}>
            <Input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder={t('例如：Lumen 工作台', 'e.g. Lumen workspace')}
            />
          </Field>
          <Field label={t('项目目录', 'Project directory')}>
            <div className="flex gap-2">
              <Input
                value={form.path}
                onChange={(e) => setForm({ ...form, path: e.target.value })}
                onBlur={() => checkGit(form.path)}
                placeholder="D:\\Projects\\example"
                className="font-mono text-xs"
              />
              <Button variant="outline" size="md" onClick={choosePath}>
                <FolderOpen size={14} />
                {t('选择', 'Browse')}
              </Button>
            </div>
            {checkingGit ? (
              <p className="mt-1 text-xs text-muted">
                {t('正在检测 Git 仓库…', 'Checking Git repo…')}
              </p>
            ) : gitCheck ? (
              <p className={cn('mt-1 text-xs', gitCheck.isGit ? 'text-success' : 'text-warn')}>
                {gitCheck.isGit
                  ? t(
                      '已检测到 Git 仓库，可使用隔离工作区',
                      'Git repository detected; isolated workspace available',
                    )
                  : t(
                      `非 Git 仓库：${gitCheck.reason ?? '无法创建隔离工作区'}`,
                      `Not a Git repo: ${gitCheck.reason ?? 'isolated workspace unavailable'}`,
                    )}
              </p>
            ) : null}
          </Field>
          <Field label={t('项目类型', 'Project type')}>
            <Select
              value={form.kind}
              onChange={(e) => setForm({ ...form, kind: e.target.value })}
            >
              {PROJECT_KINDS.map((kind) => (
                <option key={kind.value} value={kind.value}>
                  {kind.label}
                </option>
              ))}
            </Select>
          </Field>
          <Field label={t('描述', 'Description')}>
            <Textarea
              rows={3}
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
              placeholder={t('简要说明项目目标与范围', 'Briefly describe the goal and scope')}
            />
          </Field>
        </div>
      </Modal>

      <ConfirmDialog
        open={Boolean(deleting)}
        onClose={() => setDeleting(null)}
        onConfirm={confirmDelete}
        title={t(`删除项目「${deleting?.name ?? ''}」`, `Delete project “${deleting?.name ?? ''}”`)}
        description={t(
          '项目下的团队、任务与运行记录将一并移除，任务隔离工作区中的改动也会一并删除',
          'Teams, tasks, and run history under this project will be removed, along with changes made in task worktrees',
        )}
        loading={deleteLoading}
      />
    </PageScroll>
  );
}
