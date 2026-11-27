import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

export type Lang = 'zh' | 'en';

const LANG_KEY = 'lumen.lang';

export function readLang(): Lang {
  try {
    return localStorage.getItem(LANG_KEY) === 'en' ? 'en' : 'zh';
  } catch {
    return 'zh';
  }
}

let current: Lang = readLang();
if (typeof document !== 'undefined') {
  document.documentElement.lang = current === 'en' ? 'en' : 'zh-CN';
}

/** 供非 React 模块（常量表、工具函数）读取当前语言。 */
export function currentLang(): Lang {
  return current;
}

/** 非组件场景的双语取值；组件内优先使用 useI18n().t。 */
export function tr(zh: string, en: string): string {
  return current === 'en' ? en : zh;
}

interface I18nContextValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (zh: string, en: string) => string;
}

const I18nContext = createContext<I18nContextValue>({
  lang: current,
  setLang: () => undefined,
  t: tr,
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(current);

  const setLang = useCallback((next: Lang) => {
    current = next;
    try {
      localStorage.setItem(LANG_KEY, next);
    } catch {
      /* ignore */
    }
    document.documentElement.lang = next === 'en' ? 'en' : 'zh-CN';
    setLangState(next);
  }, []);

  // 动态导入避免 api.ts 与本模块形成循环依赖；失败时保持后端原语言即可。
  useEffect(() => {
    void import('./api')
      .then(({ api }) => api.setBackendLang(lang))
      .catch(() => {});
  }, [lang]);

  const t = useCallback((zh: string, en: string) => (lang === 'en' ? en : zh), [lang]);
  const value = useMemo(() => ({ lang, setLang, t }), [lang, setLang, t]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
