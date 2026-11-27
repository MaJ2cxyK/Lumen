import { useEffect, useId, useRef, type ReactNode } from 'react';
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import {
  AlertTriangle,
  ChevronLeft,
  Loader2,
  SearchX,
  ShieldCheck,
  X,
  type LucideIcon,
} from 'lucide-react';
import { AGENT_STATUS, RUN_STATUS } from '../lib/labels';
import { statusDescriptor } from '../lib/taskLifecycle';
import { tr } from '../lib/i18n';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function Card({
  className,
  children,
  onClick,
}: {
  className?: string;
  children: ReactNode;
  onClick?: () => void;
}) {
  return (
    <div
      onClick={onClick}
      className={cn(
        'rounded-xl border border-line bg-card p-5',
        className,
      )}
    >
      {children}
    </div>
  );
}

type ButtonVariant = 'primary' | 'outline' | 'ghost' | 'danger' | 'subtle';
type ButtonSize = 'sm' | 'md' | 'xs';

const VARIANTS: Record<ButtonVariant, string> = {
  primary: 'bg-primary text-white font-medium hover:bg-primary/90',
  outline: 'border border-line bg-transparent text-ink hover:bg-hover',
  ghost: 'text-muted hover:text-ink hover:bg-hover',
  danger: 'border border-danger/40 bg-transparent text-danger hover:bg-danger/10',
  subtle: 'bg-soft2 text-ink hover:bg-soft2',
};

const SIZES: Record<ButtonSize, string> = {
  xs: 'h-7 px-2.5 text-xs gap-1 rounded-lg',
  sm: 'h-8 px-3 text-xs gap-1.5 rounded-lg',
  md: 'h-9 px-3.5 text-sm gap-2 rounded-lg',
};

export function Button({
  variant = 'primary',
  size = 'md',
  loading,
  className,
  children,
  disabled,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}) {
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      className={cn(
        'inline-flex items-center justify-center font-medium transition-colors active:translate-y-px disabled:cursor-not-allowed disabled:opacity-50 disabled:active:translate-y-0',
        VARIANTS[variant],
        SIZES[size],
        className,
      )}
    >
      {loading && <Loader2 size={14} className="animate-spin" />}
      {children}
    </button>
  );
}

export function Badge({
  children,
  className,
  tone,
}: {
  children: ReactNode;
  className?: string;
  tone?: 'primary' | 'success' | 'warn' | 'danger' | 'muted';
}) {
  const tones = {
    primary: 'bg-primary/10 text-primary',
    success: 'bg-success/10 text-success',
    warn: 'bg-warn/10 text-warn',
    danger: 'bg-danger/10 text-danger',
    muted: 'bg-soft text-muted',
  };
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium leading-4',
        tone ? tones[tone] : 'bg-soft text-muted',
        className,
      )}
    >
      {children}
    </span>
  );
}

export function TaskStatusBadge({ status }: { status: string }) {
  const meta = statusDescriptor(status);
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-md bg-soft px-1.5 py-0.5 text-xs font-medium text-muted"
    >
      <span className={cn('h-1.5 w-1.5 rounded-full', meta.dot)} />
      {meta.label}
    </span>
  );
}

export function RunStatusBadge({ status }: { status: string }) {
  const meta = RUN_STATUS[status] ?? RUN_STATUS.running;
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-md bg-soft px-1.5 py-0.5 text-xs font-medium text-muted"
    >
      <span className={cn('h-1.5 w-1.5 rounded-full', meta.dot)} />
      {meta.label}
    </span>
  );
}

export function isolationReasonText(reason?: string | null): string {
  switch (reason) {
    case 'git_nul':
    case 'opencode_git_nul':
      return tr(
        'Git 工作区：Windows AppContainer 禁止访问 NUL，Git for Windows 无法可靠运行',
        'Git workspace: Windows AppContainer blocks the NUL device required by Git for Windows',
      );
    case 'proxy_disabled':
      return tr(
        '模型代理已关闭，真实 Key 不应进入文件隔离容器',
        'The model proxy is disabled, so the real key must not enter the file-isolated container',
      );
    case 'provider_unavailable':
      return tr('未找到可用的模型供应商', 'No available model provider');
    case 'prepare_failed':
      return tr('沙箱准备失败，详见运行事件', 'Sandbox preparation failed; see run events');
    default:
      return reason ?? '';
  }
}

// AppContainer 隔离运行时显示沙箱标记；发生回退时显示可追溯的进程隔离标记。
export function IsolationBadge({
  isolation,
  reason,
}: {
  isolation?: string;
  reason?: string | null;
}) {
  if (isolation === 'appcontainer') {
    return (
      <span
        title={tr('AppContainer 文件隔离运行', 'Running with AppContainer file isolation')}
        className="inline-flex items-center gap-0.5 rounded-md bg-soft px-1.5 py-0.5 text-xs font-medium text-muted"
      >
        <ShieldCheck size={10} />
        {tr('沙箱', 'Sandbox')}
      </span>
    );
  }
  if (!reason) {
    return null;
  }
  const detail = isolationReasonText(reason);
  return (
    <span
      title={detail}
      className="inline-flex items-center gap-0.5 rounded-md bg-warn/10 px-1.5 py-0.5 text-xs font-medium text-warn"
    >
      <AlertTriangle size={10} />
      {tr('进程隔离', 'Process')}
    </span>
  );
}

export function AgentStatusBadge({ status }: { status: string }) {
  const meta = AGENT_STATUS[status] ?? AGENT_STATUS.idle;
  return (
    <span className="inline-flex items-center gap-1.5 text-xs font-medium text-muted">
      <span className={cn('h-1.5 w-1.5 rounded-full', meta.dot)} />
      {meta.label}
    </span>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative h-5 w-9 shrink-0 rounded-full transition-colors disabled:opacity-50',
        checked ? 'bg-primary' : 'bg-soft2',
      )}
    >
      <span
        className={cn(
          'absolute top-[3px] h-3.5 w-3.5 rounded-full bg-white transition-all',
          checked ? 'left-[19px]' : 'left-[3px]',
        )}
      />
    </button>
  );
}

export function Field({
  label,
  hint,
  children,
  className,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={cn('block', className)}>
      <span className="mb-1.5 block text-xs font-medium text-muted">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-muted/70">{hint}</span>}
    </label>
  );
}

const inputClass =
  'w-full rounded-lg border border-line bg-base px-2.5 py-1.5 text-sm text-ink outline-none transition-colors placeholder:text-muted/70 focus:border-primary/70 focus:ring-2 focus:ring-primary/15 disabled:opacity-50';

export function Input({
  className,
  ...rest
}: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...rest} className={cn(inputClass, className)} />;
}

export function Select({
  className,
  children,
  ...rest
}: React.SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...rest}
      className={cn(inputClass, 'select-caret appearance-none pr-8', className)}
    >
      {children}
    </select>
  );
}

export function Textarea({
  className,
  ...rest
}: React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...rest} className={cn(inputClass, 'resize-none', className)} />;
}

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  width = 'max-w-lg',
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const lastActive = useRef<HTMLElement | null>(null);
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    lastActive.current = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const focusables = () =>
      Array.from(
        panel?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      );
    const raf = requestAnimationFrame(() => {
      const first = focusables()[0];
      (first ?? panel)?.focus();
    });
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== 'Tab') return;
      const list = focusables();
      if (list.length === 0) return;
      const first = list[0];
      const last = list[list.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = previousOverflow;
      lastActive.current?.focus?.();
    };
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-[2px]"
        onClick={onClose}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cn(
          'relative flex max-h-[86vh] w-full flex-col overflow-hidden rounded-xl border border-line bg-modal shadow-[0_10px_28px_-10px_rgba(0,0,0,0.5)] outline-none',
          width,
        )}
      >
        <div className="flex items-start justify-between border-b border-line px-5 py-4">
          <div>
            <h3 id={titleId} className="text-sm font-semibold text-ink">
              {title}
            </h3>
            {description && <p className="mt-1 text-xs text-muted">{description}</p>}
          </div>
          <Button variant="ghost" size="xs" onClick={onClose} aria-label={tr('关闭', 'Close')}>
            <X size={15} />
          </Button>
        </div>
        <div className="flex-1 overflow-y-auto px-5 py-4">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-line px-5 py-3">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

export function Empty({
  icon: Icon = SearchX,
  title,
  description,
  action,
  className,
}: {
  icon?: LucideIcon;
  title: string;
  description?: string;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-line px-4 py-10 text-center',
        className,
      )}
    >
      <Icon size={22} className="text-muted/60" />
      <p className="text-sm font-medium text-ink/80">{title}</p>
      {description && <p className="max-w-sm text-xs text-muted">{description}</p>}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}

export function Loading({ label = tr('加载中', 'Loading') }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-2 py-12 text-xs text-muted">
      <Loader2 size={15} className="animate-spin" />
      {label}
    </div>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
  back,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  back?: { to: string; label: string };
}) {
  return (
    <div className="mb-5 flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0 flex-1">
        {back && (
          <a
            href={`#${back.to}`}
            className="mb-1.5 inline-flex items-center gap-1 text-xs text-muted transition-colors hover:text-ink"
          >
            <ChevronLeft size={13} />
            {back.label}
          </a>
        )}
        <h1 className="truncate text-xl font-semibold leading-6 tracking-[-0.02em] text-ink">
          {title}
        </h1>
        {subtitle && <p className="mt-1 text-sm leading-5 text-muted">{subtitle}</p>}
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 pt-0.5">{actions}</div>
      )}
    </div>
  );
}

export function PageScroll({
  children,
  embedded = false,
}: {
  children: ReactNode;
  embedded?: boolean;
}) {
  if (embedded) return <div className="space-y-5">{children}</div>;
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-[1240px] px-7 py-6">{children}</div>
    </div>
  );
}

export function Tabs<T extends string>({
  items,
  value,
  onChange,
}: {
  items: { value: T; label: string; count?: number }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex items-stretch gap-1 border-b border-line">
      {items.map((item) => (
        <button
          key={item.value}
          onClick={() => onChange(item.value)}
          className={cn(
            'relative -mb-px inline-flex items-center gap-1.5 px-2.5 pb-2 pt-0.5 text-sm transition-colors',
            value === item.value
              ? 'border-b-2 border-primary font-semibold text-ink'
              : 'border-b-2 border-transparent text-muted hover:bg-hover hover:text-ink',
          )}
        >
          {item.label}
          {item.count !== undefined && (
            <span
              className={cn(
                'rounded-full px-1.5 text-xs leading-[18px]',
                value === item.value ? 'bg-primary/15 text-primary' : 'bg-soft2',
              )}
            >
              {item.count}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}

export function ProgressBar({
  value,
  warn,
  blocked,
  className,
  markerPct,
}: {
  value: number;
  warn?: boolean;
  blocked?: boolean;
  className?: string;
  markerPct?: number;
}) {
  const pct = Math.min(100, Math.max(0, value));
  return (
    <div className={cn('relative h-1.5 w-full overflow-hidden rounded-full bg-soft2', className)}>
      <div
        className={cn(
          'h-full rounded-full transition-all',
          blocked ? 'bg-danger' : warn ? 'bg-warn' : 'bg-primary',
        )}
        style={{ width: `${pct}%` }}
      />
      {markerPct !== undefined && markerPct > 0 && markerPct < 100 && (
        <span
          className="absolute top-0 h-full w-px bg-ink/30"
          style={{ left: `${markerPct}%` }}
        />
      )}
    </div>
  );
}

export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  description,
  hint = tr('该操作不可撤销，请确认后继续。', 'This cannot be undone. Continue?'),
  confirmText = tr('删除', 'Delete'),
  loading,
}: {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  description?: string;
  hint?: string;
  confirmText?: string;
  loading?: boolean;
}) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={title}
      description={description}
      width="max-w-sm"
      footer={
        <>
          <Button variant="outline" size="sm" onClick={onClose}>
            {tr('取消', 'Cancel')}
          </Button>
          <Button variant="danger" size="sm" loading={loading} onClick={onConfirm}>
            {confirmText}
          </Button>
        </>
      }
    >
      <p className="text-xs leading-5 text-muted">
        {hint}
      </p>
    </Modal>
  );
}

export function SectionTitle({
  icon: Icon,
  title,
  description,
  extra,
}: {
  icon?: LucideIcon;
  title: string;
  description?: string;
  extra?: ReactNode;
}) {
  return (
    <div className="mb-3.5 flex items-start justify-between gap-3">
      <div className="flex min-w-0 items-center gap-2">
        {Icon && <Icon size={14} className="shrink-0 text-muted" />}
        <div className="min-w-0">
          <h2 className="text-sm font-semibold leading-5 tracking-[-0.01em] text-ink">
            {title}
          </h2>
          {description && (
            <p className="mt-0.5 text-xs leading-4 text-muted">{description}</p>
          )}
        </div>
      </div>
      {extra && <div className="shrink-0">{extra}</div>}
    </div>
  );
}
