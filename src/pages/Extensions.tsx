import { useCallback, useEffect, useState } from 'react';
import {
  Activity,
  Package,
  Pencil,
  Plug,
  Plus,
  ShieldCheck,
  Sparkles,
  Trash2,
  Webhook,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { ExtensionHealth, ExtensionItem, ExtensionKind } from '../lib/types';
import { tr, useI18n } from '../lib/i18n';
import {
  Badge,
  Button,
  Card,
  ConfirmDialog,
  Empty,
  Field,
  Input,
  Loading,
  Modal,
  PageHeader,
  PageScroll,
  Select,
  Tabs,
  Textarea,
  Toggle,
} from '../components/ui';

const KIND_META: Record<
  ExtensionKind,
  { label: string; singular: string; icon: typeof Plug; blurb: string }
> = {
  mcp: {
    label: 'MCP',
    singular: 'MCP',
    icon: Plug,
    get blurb() {
      return tr(
        '模型上下文协议服务，运行时通过临时 mcp 配置注入。',
        'Model Context Protocol servers, injected at runtime through a temporary mcp config.',
      );
    },
  },
  hook: {
    label: 'Hooks',
    singular: 'Hook',
    icon: Webhook,
    get blurb() {
      return tr(
        '在 Claude Code 生命周期事件上执行命令，与 Lumen 的审批钩子合并。',
        'Run commands on Claude Code lifecycle events and merge with Lumen approval hooks.',
      );
    },
  },
  skill: {
    label: 'Skills',
    singular: 'Skill',
    icon: Sparkles,
    get blurb() {
      return tr(
        '以 SKILL.md 形式保存，运行时会生成 Lumen 私有插件目录再加载。',
        'Stored as SKILL.md; at runtime Lumen generates a private plugin directory and loads it.',
      );
    },
  },
  plugin: {
    get label() {
      return tr('插件', 'Plugins');
    },
    get singular() {
      return tr('插件', 'plugin');
    },
    icon: Package,
    get blurb() {
      return tr(
        '引用本地插件目录或 .zip，通过 --plugin-dir 加载。',
        'Reference a local plugin directory or .zip, loaded via --plugin-dir.',
      );
    },
  },
};

const HOOK_EVENTS = [
  'PreToolUse',
  'PostToolUse',
  'UserPromptSubmit',
  'Stop',
  'SubagentStop',
  'Notification',
  'PreCompact',
  'SessionStart',
  'SessionEnd',
];

interface ExtForm {
  id?: string;
  kind: ExtensionKind;
  name: string;
  description: string;
  enabled: boolean;
  transport: 'stdio' | 'http' | 'sse';
  command: string;
  argsText: string;
  envText: string;
  url: string;
  headersText: string;
  event: string;
  matcher: string;
  timeout: string;
  slug: string;
  content: string;
  path: string;
}

function emptyForm(kind: ExtensionKind): ExtForm {
  return {
    kind,
    name: '',
    description: '',
    enabled: true,
    transport: 'stdio',
    command: '',
    argsText: '',
    envText: '',
    url: '',
    headersText: '',
    event: 'PreToolUse',
    matcher: '',
    timeout: '60',
    slug: '',
    content: '',
    path: '',
  };
}

function kvToText(value: unknown): string {
  if (!value || typeof value !== 'object') return '';
  return Object.entries(value as Record<string, unknown>)
    .map(([key, val]) => `${key}=${String(val)}`)
    .join('\n');
}

function textToKv(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split('\n')) {
    const index = line.indexOf('=');
    if (index <= 0) continue;
    const key = line.slice(0, index).trim();
    const value = line.slice(index + 1).trim();
    if (key && value) out[key] = value;
  }
  return out;
}

export default function Extensions({
  embedded = false,
  fixedKind,
}: {
  embedded?: boolean;
  fixedKind?: ExtensionKind;
}) {
  const [items, setItems] = useState<ExtensionItem[]>([]);
  const [kind, setKind] = useState<ExtensionKind>(fixedKind ?? 'mcp');
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<ExtForm | null>(null);
  const [saving, setSaving] = useState(false);
  const [removing, setRemoving] = useState<ExtensionItem | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [health, setHealth] = useState<Record<string, ExtensionHealth>>({});
  const [healthBusy, setHealthBusy] = useState(false);
  const [healthAt, setHealthAt] = useState<number | null>(null);
  const { lang, t } = useI18n();

  const load = useCallback(async () => {
    try {
      setItems(await api.extensionsList());
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const visible = items.filter((item) => item.kind === kind);
  const healthRows = Object.values(health);
  const healthOk = healthRows.filter((row) => row.status === 'ok').length;
  const healthWarn = healthRows.filter((row) => row.status === 'warn').length;
  const healthError = healthRows.filter((row) => row.status === 'error').length;

  async function runHealth() {
    if (healthBusy) return;
    setHealthBusy(true);
    try {
      const list = await api.extensionsHealth();
      const map: Record<string, ExtensionHealth> = {};
      for (const row of list) map[row.id] = row;
      setHealth(map);
      setHealthAt(Date.now());
      const warn = list.filter((row) => row.status === 'warn').length;
      const error = list.filter((row) => row.status === 'error').length;
      if (warn === 0 && error === 0) {
        toast.success(
          t(
            `检查完成：${list.length} 项扩展均正常`,
            `Check complete: all ${list.length} extensions are healthy`,
          ),
        );
      } else {
        toast.warning(
          t(
            `检查完成：${error} 项不可用、${warn} 项待确认（共 ${list.length} 项）`,
            `Check complete: ${error} unavailable, ${warn} need review (${list.length} total)`,
          ),
        );
      }
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setHealthBusy(false);
    }
  }

  function clearHealth(id: string) {
    setHealth((prev) => {
      if (!prev[id]) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
  }

  function openCreate() {
    setEditing(emptyForm(kind));
  }

  function openEdit(item: ExtensionItem) {
    const config = item.config ?? {};
    setEditing({
      ...emptyForm(item.kind as ExtensionKind),
      id: item.id,
      kind: item.kind as ExtensionKind,
      name: item.name,
      description: item.description ?? '',
      enabled: item.enabled,
      transport: (config.transport as ExtForm['transport']) ?? 'stdio',
      command: String(config.command ?? ''),
      argsText: Array.isArray(config.args) ? config.args.join('\n') : '',
      envText: kvToText(config.env),
      url: String(config.url ?? ''),
      headersText: kvToText(config.headers),
      event: String(config.event ?? 'PreToolUse'),
      matcher: String(config.matcher ?? ''),
      timeout: String(config.timeout ?? 60),
      slug: String(config.slug ?? ''),
      content: String(config.content ?? ''),
      path: String(config.path ?? ''),
    });
  }

  function buildConfig(form: ExtForm): Record<string, unknown> {
    if (form.kind === 'mcp') {
      if (form.transport === 'stdio') {
        return {
          transport: 'stdio',
          command: form.command.trim(),
          args: form.argsText
            .split('\n')
            .map((line) => line.trim())
            .filter(Boolean),
          env: textToKv(form.envText),
        };
      }
      return {
        transport: form.transport,
        url: form.url.trim(),
        headers: textToKv(form.headersText),
      };
    }
    if (form.kind === 'hook') {
      return {
        event: form.event,
        matcher: form.matcher.trim(),
        command: form.command.trim(),
        timeout: Number(form.timeout) || 60,
      };
    }
    if (form.kind === 'skill') {
      return {
        slug: form.slug.trim(),
        description: form.description.trim() || form.name.trim(),
        content: form.content,
      };
    }
    return { path: form.path.trim() };
  }

  async function save() {
    if (!editing) return;
    setSaving(true);
    try {
      await api.extensionUpsert({
        id: editing.id,
        kind: editing.kind,
        name: editing.name,
        description: editing.description || undefined,
        config: buildConfig(editing),
        enabled: editing.enabled,
      });
      toast.success(
        editing.id
          ? t('扩展已更新', 'Extension updated')
          : t('扩展已添加', 'Extension added'),
      );
      setEditing(null);
      setHealth({});
      setHealthAt(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function toggle(item: ExtensionItem, enabled: boolean) {
    try {
      await api.extensionToggle(item.id, enabled);
      setItems((prev) =>
        prev.map((x) => (x.id === item.id ? { ...x, enabled } : x)),
      );
      clearHealth(item.id);
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function remove() {
    if (!removing) return;
    setDeleting(true);
    try {
      await api.extensionDelete(removing.id);
      toast.success(t('扩展已删除', 'Extension deleted'));
      setRemoving(null);
      setHealth({});
      setHealthAt(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <PageScroll embedded={embedded}>
      <PageHeader
        back={embedded ? undefined : { to: '/settings', label: t('返回设置', 'Back to settings') }}
        title={fixedKind ? KIND_META[fixedKind].label : t('扩展', 'Extensions')}
        subtitle={fixedKind ? KIND_META[fixedKind].blurb : undefined}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              loading={healthBusy}
              disabled={loading || items.length === 0 || Boolean(editing)}
              onClick={runHealth}
            >
              {!healthBusy && <Activity size={13} />}
              {t('健康检查', 'Health check')}
            </Button>
            <Button size="sm" onClick={openCreate}>
              <Plus size={13} />
              {t(
                `添加${KIND_META[kind].singular}`,
                `Add ${KIND_META[kind].singular}`,
              )}
            </Button>
          </>
        }
      />

      <div className="space-y-5">
        <Card className="border-success/20 bg-success/5">
          <div className="flex items-start gap-2">
            <ShieldCheck size={15} className="mt-0.5 shrink-0 text-success" />
            <div>
              <p className="text-xs font-semibold text-ink">
                {t(
                  'Lumen 私有扩展，不写入任何全局目录',
                  'Lumen-only extensions; nothing is written to global directories',
                )}
              </p>
              <p className="mt-1 text-xs leading-5 text-muted">
                {t(
                  '扩展只保存在 Lumen 的数据目录与本地数据库。Claude Code 运行时，Lumen 会在本次运行目录生成临时 settings / mcp 配置，并通过 --plugin-dir 加载 Lumen 管理的插件目录，进程结束立即删除；Skills 生成的插件包装也只位于 %APPDATA%\\Lumen\\extensions 下。不会改写 ~/.claude、~/.codex 等目录。',
                  'Extensions are stored only in Lumen’s data directory and local database. When Claude Code runs, Lumen writes temporary settings / mcp configs into the run directory and loads the Lumen-managed plugin directory via --plugin-dir, removing them as soon as the process exits. Skill wrapper plugins also live only under %APPDATA%\\Lumen\\extensions. Directories such as ~/.claude and ~/.codex are never modified.',
                )}
              </p>
            </div>
          </div>
        </Card>

        {!fixedKind && (
          <div>
            <Tabs
              items={(Object.keys(KIND_META) as ExtensionKind[]).map((key) => ({
                value: key,
                label: KIND_META[key].label,
                count: items.filter((item) => item.kind === key).length,
              }))}
              value={kind}
              onChange={setKind}
            />
            <p className="mt-2 text-xs text-muted">{KIND_META[kind].blurb}</p>
          </div>
        )}

        {healthAt && (
          <p className="px-1 text-xs text-muted">
            {t('最近检查', 'Last checked')}{' '}
            {new Date(healthAt).toLocaleTimeString(lang === 'en' ? 'en-US' : 'zh-CN', {
              hour: '2-digit',
              minute: '2-digit',
            })}
            {t('：', ': ')}
            <span className="text-success">
              {t(`${healthOk} 正常`, `${healthOk} OK`)}
            </span>
            {healthWarn > 0 && (
              <span className="text-warn">
                {' · '}
                {t(`${healthWarn} 待确认`, `${healthWarn} need review`)}
              </span>
            )}
            {healthError > 0 && (
              <span className="text-danger">
                {' · '}
                {t(`${healthError} 不可用`, `${healthError} unavailable`)}
              </span>
            )}
          </p>
        )}

        {loading ? (
          <Loading />
        ) : visible.length === 0 ? (
          <Empty
            icon={KIND_META[kind].icon}
            title={t(
              `还没有 ${KIND_META[kind].label} 扩展`,
              `No ${KIND_META[kind].label} yet`,
            )}
            description={t(
              '添加后默认启用，仅对 Claude Code 的运行生效。',
              'New entries are enabled by default and only apply to Claude Code runs.',
            )}
            action={
              <Button size="sm" onClick={openCreate}>
                <Plus size={13} />
                {t(
                  `添加${KIND_META[kind].singular}`,
                  `Add ${KIND_META[kind].singular}`,
                )}
              </Button>
            }
          />
        ) : (
          <div className="space-y-2">
            {visible.map((item) => (
              <Card key={item.id} className="px-3 py-2.5">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-xs font-semibold text-ink">
                        {item.name}
                      </span>
                      {!item.enabled && <Badge>{t('已停用', 'Disabled')}</Badge>}
                      <Badge tone="muted">{KIND_META[item.kind as ExtensionKind]?.label ?? item.kind}</Badge>
                      {health[item.id] && (
                        <Badge
                          tone={
                            health[item.id].status === 'ok'
                              ? 'success'
                              : health[item.id].status === 'warn'
                                ? 'warn'
                                : 'danger'
                          }
                        >
                          {health[item.id].status === 'ok'
                            ? t('健康', 'Healthy')
                            : health[item.id].status === 'warn'
                              ? t('待确认', 'Needs review')
                              : t('不可用', 'Unavailable')}
                        </Badge>
                      )}
                    </div>
                    {item.description && (
                      <p className="mt-1 text-xs text-muted">
                        {item.description}
                      </p>
                    )}
                    <p className="mt-1 break-all font-mono text-xs text-muted/80">
                      {summaryOf(item)}
                    </p>
                    {health[item.id] && (
                      <p
                        className={
                          health[item.id].status === 'ok'
                            ? 'mt-1 text-xs text-success/90'
                            : health[item.id].status === 'warn'
                              ? 'mt-1 text-xs text-warn'
                              : 'mt-1 text-xs text-danger'
                        }
                      >
                        {health[item.id].message}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <Toggle
                      checked={item.enabled}
                      onChange={(value) => toggle(item, value)}
                      label={t(`启用 ${item.name}`, `Enable ${item.name}`)}
                    />
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => openEdit(item)}
                    >
                      <Pencil size={12} />
                    </Button>
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => setRemoving(item)}
                    >
                      <Trash2 size={12} className="text-danger" />
                    </Button>
                  </div>
                </div>
              </Card>
            ))}
          </div>
        )}
      </div>

      <Modal
        open={!!editing}
        onClose={() => setEditing(null)}
        title={
          editing
            ? t(
                `${editing.id ? '编辑' : '添加'} ${KIND_META[editing.kind].singular}`,
                `${editing.id ? 'Edit' : 'Add'} ${KIND_META[editing.kind].singular}`,
              )
            : ''
        }
        description={t(
          '保存后仅在 Claude Code 运行时注入，不修改全局配置。',
          'Injected only while Claude Code runs; global config is never modified.',
        )}
        width="max-w-xl"
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setEditing(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={save}>
              {t('保存', 'Save')}
            </Button>
          </>
        }
      >
        {editing && (
          <div className="space-y-3">
            {!fixedKind && (
              <Field label={t('类型', 'Type')}>
                <Select
                  value={editing.kind}
                  disabled={!!editing.id}
                  onChange={(e) =>
                    setEditing({
                      ...emptyForm(e.target.value as ExtensionKind),
                      name: editing.name,
                      description: editing.description,
                      enabled: editing.enabled,
                      id: editing.id,
                    })
                  }
                >
                  {(Object.keys(KIND_META) as ExtensionKind[]).map((key) => (
                    <option key={key} value={key}>
                      {KIND_META[key].label}
                    </option>
                  ))}
                </Select>
              </Field>
            )}
            <Field label={t('名称', 'Name')}>
              <Input
                value={editing.name}
                placeholder={
                  editing.kind === 'mcp'
                    ? t('例如 github', 'e.g. github')
                    : editing.kind === 'hook'
                      ? t('例如 提交前格式化', 'e.g. format before commit')
                      : editing.kind === 'skill'
                        ? t('例如 发布检查清单', 'e.g. release checklist')
                        : t('例如 团队插件包', 'e.g. team plugin bundle')
                }
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </Field>

            {editing.kind === 'mcp' && (
              <>
                <Field label={t('传输方式', 'Transport')}>
                  <Select
                    value={editing.transport}
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        transport: e.target.value as ExtForm['transport'],
                      })
                    }
                  >
                    <option value="stdio">{t('stdio（本地命令）', 'stdio (local command)')}</option>
                    <option value="http">HTTP</option>
                    <option value="sse">SSE</option>
                  </Select>
                </Field>
                {editing.transport === 'stdio' ? (
                  <>
                    <Field label={t('启动命令', 'Command')}>
                      <Input
                        value={editing.command}
                        placeholder="npx"
                        spellCheck={false}
                        onChange={(e) =>
                          setEditing({ ...editing, command: e.target.value })
                        }
                      />
                    </Field>
                    <Field
                      label={t('参数', 'Arguments')}
                      hint={t('每行一个参数', 'One argument per line')}
                    >
                      <Textarea
                        rows={3}
                        value={editing.argsText}
                        placeholder={'-y\n@modelcontextprotocol/server-github'}
                        spellCheck={false}
                        onChange={(e) =>
                          setEditing({ ...editing, argsText: e.target.value })
                        }
                      />
                    </Field>
                    <Field
                      label={t('环境变量', 'Environment variables')}
                      hint={t(
                        '每行 KEY=VALUE，仅保存于本地数据库',
                        'One KEY=VALUE per line, stored only in the local database',
                      )}
                    >
                      <Textarea
                        rows={3}
                        value={editing.envText}
                        placeholder="GITHUB_TOKEN=..."
                        spellCheck={false}
                        onChange={(e) =>
                          setEditing({ ...editing, envText: e.target.value })
                        }
                      />
                    </Field>
                  </>
                ) : (
                  <>
                    <Field label={t('服务地址', 'Server URL')}>
                      <Input
                        value={editing.url}
                        placeholder="https://example.com/mcp"
                        spellCheck={false}
                        onChange={(e) =>
                          setEditing({ ...editing, url: e.target.value })
                        }
                      />
                    </Field>
                    <Field
                      label={t('请求头', 'Headers')}
                      hint={t('每行 KEY=VALUE', 'One KEY=VALUE per line')}
                    >
                      <Textarea
                        rows={3}
                        value={editing.headersText}
                        placeholder="Authorization=Bearer ..."
                        spellCheck={false}
                        onChange={(e) =>
                          setEditing({ ...editing, headersText: e.target.value })
                        }
                      />
                    </Field>
                  </>
                )}
              </>
            )}

            {editing.kind === 'hook' && (
              <>
                <Field label={t('事件', 'Event')}>
                  <Select
                    value={editing.event}
                    onChange={(e) =>
                      setEditing({ ...editing, event: e.target.value })
                    }
                  >
                    {HOOK_EVENTS.map((event) => (
                      <option key={event} value={event}>
                        {event}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field
                  label={t('匹配器', 'Matcher')}
                  hint={t('留空表示匹配全部工具', 'Leave empty to match all tools')}
                >
                  <Input
                    value={editing.matcher}
                    placeholder="Bash|Write|Edit"
                    spellCheck={false}
                    onChange={(e) =>
                      setEditing({ ...editing, matcher: e.target.value })
                    }
                  />
                </Field>
                <Field label={t('执行命令', 'Command')}>
                  <Input
                    value={editing.command}
                    placeholder="npx prettier --write ."
                    spellCheck={false}
                    onChange={(e) =>
                      setEditing({ ...editing, command: e.target.value })
                    }
                  />
                </Field>
                <Field label={t('超时（秒）', 'Timeout (seconds)')}>
                  <Input
                    type="number"
                    min={1}
                    max={3600}
                    value={editing.timeout}
                    onChange={(e) =>
                      setEditing({ ...editing, timeout: e.target.value })
                    }
                  />
                </Field>
              </>
            )}

            {editing.kind === 'skill' && (
              <>
                <Field
                  label={t('技能标识', 'Skill slug')}
                  hint={t(
                    '仅小写字母、数字与短横线，留空则按名称生成',
                    'Lowercase letters, digits and hyphens only; generated from the name when empty',
                  )}
                >
                  <Input
                    value={editing.slug}
                    placeholder="release-checklist"
                    spellCheck={false}
                    onChange={(e) =>
                      setEditing({ ...editing, slug: e.target.value })
                    }
                  />
                </Field>
                <Field
                  label={t('描述', 'Description')}
                  hint={t('会写入 SKILL.md 的 front matter', 'Written to the SKILL.md front matter')}
                >
                  <Input
                    value={editing.description}
                    placeholder={t(
                      '发布前检查版本、变更日志与测试',
                      'Check version, changelog and tests before release',
                    )}
                    onChange={(e) =>
                      setEditing({ ...editing, description: e.target.value })
                    }
                  />
                </Field>
                <Field label={t('Skill 内容', 'Skill content')}>
                  <Textarea
                    rows={8}
                    value={editing.content}
                    placeholder={t(
                      '说明何时使用该技能，以及具体步骤、命令与约束。',
                      'Explain when to use this skill, plus the concrete steps, commands and constraints.',
                    )}
                    onChange={(e) =>
                      setEditing({ ...editing, content: e.target.value })
                    }
                  />
                </Field>
              </>
            )}

            {editing.kind === 'plugin' && (
              <Field
                label={t('插件路径', 'Plugin path')}
                hint={t(
                  '目录需包含 .claude-plugin/plugin.json；也支持 .zip。Lumen 只引用路径，不会复制或改写内容',
                  'The directory must contain .claude-plugin/plugin.json; .zip files are also supported. Lumen only references the path and never copies or rewrites it',
                )}
              >
                <Input
                  value={editing.path}
                  placeholder="D:\plugins\my-plugin"
                  spellCheck={false}
                  onChange={(e) =>
                    setEditing({ ...editing, path: e.target.value })
                  }
                />
              </Field>
            )}

            {editing.kind !== 'skill' && (
              <Field
                label={t('描述', 'Description')}
                hint={t('可选，仅用于 Lumen 界面展示', 'Optional; shown only in the Lumen UI')}
              >
                <Input
                  value={editing.description}
                  placeholder={t('这项扩展解决什么问题', 'What does this extension solve?')}
                  onChange={(e) =>
                    setEditing({ ...editing, description: e.target.value })
                  }
                />
              </Field>
            )}

            <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2">
              <span className="text-xs text-ink">
                {t('启用该扩展', 'Enable this extension')}
              </span>
              <Toggle
                checked={editing.enabled}
                onChange={(value) => setEditing({ ...editing, enabled: value })}
                label={t('启用扩展', 'Enable extension')}
              />
            </div>
          </div>
        )}
      </Modal>

      <ConfirmDialog
        open={!!removing}
        onClose={() => setRemoving(null)}
        onConfirm={remove}
        title={t(
          `删除扩展「${removing?.name ?? ''}」`,
          `Delete extension “${removing?.name ?? ''}”`,
        )}
        description={
          removing?.kind === 'skill'
            ? t(
                '删除后会同时清理 Lumen 数据目录下生成的 Skill 插件文件。',
                'Deleting also removes the generated Skill plugin files under the Lumen data directory.',
              )
            : t(
                '只删除 Lumen 中的登记，不会删除外部插件目录。',
                'Only the Lumen entry is removed; the external plugin directory is untouched.',
              )
        }
        loading={deleting}
      />
    </PageScroll>
  );
}

function summaryOf(item: ExtensionItem): string {
  const config = item.config ?? {};
  if (item.kind === 'mcp') {
    const transport = String(config.transport ?? 'stdio');
    if (transport === 'stdio') {
      const args = Array.isArray(config.args) ? config.args.join(' ') : '';
      return [String(config.command ?? ''), args].filter(Boolean).join(' ');
    }
    return `${transport.toUpperCase()}${tr('，', ', ')}${String(config.url ?? '')}`;
  }
  if (item.kind === 'hook') {
    const matcher = String(config.matcher ?? '');
    return `${String(config.event ?? '')}${
      matcher ? `${tr('，', ', ')}${matcher}` : ''
    }${tr('，', ', ')}${String(config.command ?? '')}`;
  }
  if (item.kind === 'skill') {
    return `SKILL.md${tr('，', ', ')}${String(config.slug ?? '')}`;
  }
  return String(config.path ?? '');
}
