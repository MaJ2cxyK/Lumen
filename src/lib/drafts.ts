// 输入草稿按项目 / 任务分开保存，切换上下文时恢复，创建成功后清除。

export interface ProjectDraft {
  text: string;
  model: string;
  reasoning: string;
  mandate: string;
  isolated: boolean;
}

export const DEFAULT_PROJECT_DRAFT: ProjectDraft = {
  text: '',
  model: '',
  reasoning: 'medium',
  mandate: 'ask',
  isolated: false,
};

const PROJECT_PREFIX = 'lumen.draft.project.';
const TASK_PREFIX = 'lumen.draft.task.';

function read<T>(key: string): T | null {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function write(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 存储不可用时放弃草稿，不阻塞输入
  }
}

function remove(key: string) {
  try {
    localStorage.removeItem(key);
  } catch {
    // ignore
  }
}

function normalizeProjectDraft(value: Partial<ProjectDraft> | null): ProjectDraft {
  return {
    text: typeof value?.text === 'string' ? value.text : '',
    model: typeof value?.model === 'string' ? value.model : '',
    reasoning: typeof value?.reasoning === 'string' ? value.reasoning : 'medium',
    mandate: typeof value?.mandate === 'string' ? value.mandate : 'ask',
    isolated: Boolean(value?.isolated),
  };
}

export function loadProjectDraft(id: string): ProjectDraft {
  return normalizeProjectDraft(read<Partial<ProjectDraft>>(PROJECT_PREFIX + (id || 'new')));
}

export function saveProjectDraft(id: string, draft: ProjectDraft) {
  write(PROJECT_PREFIX + (id || 'new'), draft);
}

export function clearProjectDraft(id: string) {
  remove(PROJECT_PREFIX + (id || 'new'));
}

export function loadTaskDraft(id: string): string {
  const value = read<{ text?: unknown }>(TASK_PREFIX + id);
  return typeof value?.text === 'string' ? value.text : '';
}

export function saveTaskDraft(id: string, text: string) {
  write(TASK_PREFIX + id, { text });
}

export function clearTaskDraft(id: string) {
  remove(TASK_PREFIX + id);
}
