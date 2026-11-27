const KEY = 'lumen.cli.hidden';
const EVENT = 'lumen:cli-visibility';

/** 读取被隐藏（不参与 Agent 选择）的 CLI 名称集合。 */
export function hiddenCliNames(): Set<string> {
  try {
    const raw = localStorage.getItem(KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((item): item is string => typeof item === 'string'));
  } catch {
    return new Set();
  }
}

export function setCliHidden(name: string, hidden: boolean) {
  const set = hiddenCliNames();
  if (hidden) set.add(name);
  else set.delete(name);
  localStorage.setItem(KEY, JSON.stringify([...set]));
  window.dispatchEvent(new Event(EVENT));
}

export function subscribeCliVisibility(listener: () => void) {
  window.addEventListener(EVENT, listener);
  return () => window.removeEventListener(EVENT, listener);
}
