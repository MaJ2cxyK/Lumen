import { create } from 'zustand';
import { api, errorMessage } from './lib/api';
import type { Agent, Project } from './lib/types';

export type ThemeMode = 'dark' | 'light';
export type AccentTheme = 'forest' | 'mint' | 'honey' | 'parchment';

function readTheme(): ThemeMode {
  try {
    const saved =
      localStorage.getItem('lumen.theme') || localStorage.getItem('k.theme');
    return saved === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

function readAccent(): AccentTheme {
  try {
    const saved = localStorage.getItem('lumen.accent');
    if (saved === 'mint' || saved === 'honey' || saved === 'parchment') {
      return saved;
    }
    return 'forest';
  } catch {
    return 'forest';
  }
}

export function applyTheme(mode: ThemeMode) {
  document.documentElement.dataset.theme = mode;
  try {
    localStorage.setItem('lumen.theme', mode);
  } catch {
    /* ignore */
  }
}

export function applyAccent(accent: AccentTheme) {
  document.documentElement.dataset.accent = accent;
  try {
    localStorage.setItem('lumen.accent', accent);
  } catch {
    /* ignore */
  }
}

const initialTheme = readTheme();
const initialAccent = readAccent();
applyTheme(initialTheme);
applyAccent(initialAccent);

interface AppState {
  agents: Agent[];
  projects: Project[];
  pendingApprovals: number;
  /** 需要你介入的任务数（待分派 / 待审批 / 阻塞 / 失败）。 */
  needsYou: number;
  loaded: boolean;
  coreError: string | null;
  theme: ThemeMode;
  accent: AccentTheme;
  setTheme: (mode: ThemeMode) => void;
  setAccent: (accent: AccentTheme) => void;
  loadCore: () => Promise<void>;
  refreshPending: () => Promise<void>;
  refreshNeedsYou: () => Promise<void>;
}

export const useAppStore = create<AppState>((set) => ({
  agents: [],
  projects: [],
  pendingApprovals: 0,
  needsYou: 0,
  loaded: false,
  coreError: null,
  theme: initialTheme,
  accent: initialAccent,
  setTheme: (mode) => {
    applyTheme(mode);
    set({ theme: mode });
  },
  setAccent: (accent) => {
    applyAccent(accent);
    set({ accent });
  },
  loadCore: async () => {
    try {
      const [agents, projects, pending, attention] = await Promise.all([
        api.agentsList(),
        api.projectsList(),
        api.approvalsPending(),
        api.tasksPage({ attentionOnly: true, limit: 1 }),
      ]);
      set({
        agents,
        projects,
        pendingApprovals: pending,
        needsYou: attention.total,
        loaded: true,
        coreError: null,
      });
    } catch (err) {
      set({ loaded: true, coreError: errorMessage(err) });
    }
  },
  refreshPending: async () => {
    try {
      const pending = await api.approvalsPending();
      set({ pendingApprovals: pending });
    } catch {
      /* ignore */
    }
  },
  refreshNeedsYou: async () => {
    try {
      const attention = await api.tasksPage({ attentionOnly: true, limit: 1 });
      set({ needsYou: attention.total });
    } catch {
      /* ignore */
    }
  },
}));
