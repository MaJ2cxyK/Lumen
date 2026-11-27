import { isTauri } from './env';

type Handler<T> = (payload: T) => void;

const bus = new EventTarget();

export function emitLocal(name: string, payload: unknown) {
  bus.dispatchEvent(new CustomEvent(name, { detail: payload }));
}

export async function onEvent<T>(
  name: string,
  handler: Handler<T>,
): Promise<() => void> {
  if (isTauri) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<T>(name, (event) => handler(event.payload));
  }
  const listener = (event: Event) => {
    handler((event as CustomEvent<T>).detail);
  };
  bus.addEventListener(name, listener);
  return () => bus.removeEventListener(name, listener);
}
