import { useCallback, useEffect, useRef } from 'react';

const IME_WINDOW_MS = 500;
let compositionEndedAt = 0;

/**
 * 输入框组合键守卫：Enter 发送、Shift + Enter 换行。
 * 中文输入法选词时也会触发 Enter，用户在 composition 期间按下的 Enter 只用于选词；
 * 部分输入法还会在 compositionend 之后补发一次 keydown，因此额外保留 500ms 时间窗。
 */
export function useComposer(onSubmit: () => void): {
  onCompositionStart: () => void;
  onCompositionEnd: () => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLTextAreaElement>) => void;
} {
  const composing = useRef(false);

  const onCompositionStart = useCallback(() => {
    composing.current = true;
  }, []);

  const onCompositionEnd = useCallback(() => {
    composing.current = false;
    compositionEndedAt = Date.now();
  }, []);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key !== 'Enter' || event.shiftKey) return;
      if (
        composing.current ||
        event.nativeEvent.isComposing ||
        event.nativeEvent.keyCode === 229 ||
        Date.now() - compositionEndedAt < IME_WINDOW_MS
      ) {
        return;
      }
      event.preventDefault();
      onSubmit();
    },
    [onSubmit],
  );

  return { onCompositionStart, onCompositionEnd, onKeyDown };
}

/**
 * 自适应高度：先归零再按内容撑开，超过上限后转为内部滚动。
 */
export function useAutoGrow(
  ref: React.RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxHeight: number,
) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, maxHeight)}px`;
    el.style.overflowY = el.scrollHeight > maxHeight ? 'auto' : 'hidden';
  }, [ref, value, maxHeight]);
}
