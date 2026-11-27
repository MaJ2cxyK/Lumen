import { useCallback, useEffect, useState } from 'react';
import {
  KeyRound,
  Pencil,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Zap,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { Provider, ProviderTestResult } from '../lib/types';
import { useI18n } from '../lib/i18n';
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
  SectionTitle,
  Select,
  Toggle,
} from './ui';

interface ProviderForm {
  id?: string;
  name: string;
  protocol: 'anthropic' | 'openai';
  baseUrl: string;
  model: string;
  models: string[];
  apiKey: string;
  enabled: boolean;
}

const EMPTY_FORM: ProviderForm = {
  name: '',
  protocol: 'anthropic',
  baseUrl: '',
  model: '',
  models: [],
  apiKey: '',
  enabled: true,
};

export function ProvidersPanel() {
  const [providers, setProviders] = useState<Provider[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<ProviderForm | null>(null);
  const [saving, setSaving] = useState(false);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelOptions, setModelOptions] = useState<string[]>([]);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ProviderTestResult>>({});
  const [removing, setRemoving] = useState<Provider | null>(null);
  const [deleting, setDeleting] = useState(false);
  const { t } = useI18n();

  const load = useCallback(async () => {
    try {
      setProviders(await api.providersList());
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  function openCreate() {
    setModelOptions([]);
    setEditing({ ...EMPTY_FORM });
  }

  function openEdit(provider: Provider) {
    setModelOptions(provider.models ?? []);
    setEditing({
      id: provider.id,
      name: provider.name,
      protocol: provider.protocol === 'openai' ? 'openai' : 'anthropic',
      baseUrl: provider.baseUrl,
      model: provider.model ?? '',
      models: provider.models ?? [],
      apiKey: '',
      enabled: provider.enabled,
    });
  }

  async function fetchModels() {
    if (!editing) return;
    if (!editing.baseUrl.trim()) {
      toast.error(t('请先填写 Base URL', 'Enter the Base URL first'));
      return;
    }
    setFetchingModels(true);
    try {
      const remote = await api.providerModels({
        id: editing.id,
        protocol: editing.protocol,
        baseUrl: editing.baseUrl.trim(),
        apiKey: editing.apiKey.trim() || undefined,
      });
      const merged = Array.from(new Set([...remote, ...editing.models]));
      setModelOptions(merged);
      toast.success(
        t(
          `获取到 ${remote.length} 个模型，勾选后保存`,
          `Fetched ${remote.length} models. Select the ones to save`,
        ),
      );
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setFetchingModels(false);
    }
  }

  function toggleModel(model: string) {
    if (!editing) return;
    const checked = editing.models.includes(model);
    const models = checked
      ? editing.models.filter((item) => item !== model)
      : [...editing.models, model];
    let next = editing.model;
    if (checked && editing.model === model) {
      next = models[0] ?? '';
    } else if (!checked && !editing.model) {
      next = models[0] ?? '';
    }
    setEditing({ ...editing, models, model: next });
  }

  async function save() {
    if (!editing) return;
    setSaving(true);
    try {
      await api.providerUpsert({
        id: editing.id,
        name: editing.name,
        protocol: editing.protocol,
        baseUrl: editing.baseUrl,
        model: editing.model || undefined,
        models: editing.models,
        apiKey: editing.apiKey || undefined,
        enabled: editing.enabled,
      });
      toast.success(editing.id ? t('供应商已更新', 'Provider updated') : t('供应商已添加', 'Provider added'));
      setEditing(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function test(provider: Provider) {
    setTestingId(provider.id);
    try {
      const result = await api.providerTest(provider.id);
      setResults((prev) => ({ ...prev, [provider.id]: result }));
      if (result.ok) {
        toast.success(t(`连接成功，${result.latencyMs}ms`, `Connected in ${result.latencyMs}ms`));
      } else {
        toast.error(result.message);
      }
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setTestingId(null);
    }
  }

  async function toggle(provider: Provider, enabled: boolean) {
    try {
      await api.providerUpsert({
        id: provider.id,
        name: provider.name,
        protocol: provider.protocol,
        baseUrl: provider.baseUrl,
        model: provider.model ?? undefined,
        enabled,
      });
      setProviders((prev) =>
        prev.map((p) => (p.id === provider.id ? { ...p, enabled } : p)),
      );
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function remove() {
    if (!removing) return;
    setDeleting(true);
    try {
      await api.providerDelete(removing.id);
      toast.success(t('供应商已删除', 'Provider deleted'));
      setRemoving(null);
      await load();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleting(false);
    }
  }

  return (
    <Card>
      <SectionTitle
        icon={KeyRound}
        title={t('自定义 API 供应商', 'Custom API providers')}
        extra={
          <Button size="xs" variant="outline" onClick={openCreate}>
            <Plus size={12} />
            {t('添加', 'Add')}
          </Button>
        }
      />
      <p className="text-xs leading-5 text-muted">
        {t(
          '兼容 Anthropic 与 OpenAI 协议。Base URL、模型与密钥只保存在本机，运行时按 Agent 绑定以环境变量注入，不改写任何 CLI 的全局配置。',
          'Compatible with the Anthropic and OpenAI protocols. Base URL, model, and key are stored locally only; at runtime they are injected as environment variables per agent binding without modifying any CLI global config.',
        )}
      </p>

      <div className="mt-3 flex items-start gap-2 rounded-lg border border-line/60 bg-base/40 px-3 py-2">
        <ShieldCheck size={13} className="mt-0.5 shrink-0 text-success" />
        <p className="text-xs leading-4 text-muted">
          {t(
            '密钥经 Windows DPAPI（当前用户）加密后写入本地数据库，界面与日志只显示末四位掩码，Lumen 不提供查看明文入口。说明：同一 Windows 用户下的高权限进程无法被应用层彻底隔离。',
            'Keys are encrypted with Windows DPAPI (current user) before being written to the local database. The UI and logs only show a last-four mask; Lumen offers no way to view plaintext. Note: high-privilege processes under the same Windows user cannot be fully isolated at the application layer.',
          )}
        </p>
      </div>

      {loading ? (
        <Loading />
      ) : providers.length === 0 ? (
        <div className="mt-3">
          <Empty
            icon={KeyRound}
            title={t('尚未添加供应商', 'No providers yet')}
            description={t(
              '添加后可在 Agent 编辑页选择，让该 Agent 的 CLI 走自定义线路。',
              'After adding one, select it on the agent edit page to route that agent’s CLI through it.',
            )}
            action={
              <Button size="sm" onClick={openCreate}>
                <Plus size={13} />
                {t('添加供应商', 'Add provider')}
              </Button>
            }
          />
        </div>
      ) : (
        <div className="mt-3 space-y-2">
          {providers.map((provider) => {
            const result = results[provider.id];
            return (
              <div
                key={provider.id}
                className="rounded-xl border border-line bg-base/30 px-3 py-2.5"
              >
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-xs font-semibold text-ink">
                        {provider.name}
                      </span>
                      <Badge tone={provider.protocol === 'anthropic' ? 'primary' : 'muted'}>
                        {provider.protocol === 'anthropic' ? 'Anthropic' : 'OpenAI'}
                      </Badge>
                      <span className="font-mono text-xs text-muted">
                        {provider.model || t('未指定模型', 'No model specified')}
                      </span>
                      {provider.models.length > 1 && (
                        <span className="text-xs text-muted">
                          {t(`共 ${provider.models.length} 个`, `${provider.models.length} total`)}
                        </span>
                      )}
                      {!provider.enabled && <Badge>{t('已停用', 'Disabled')}</Badge>}
                    </div>
                    <p className="mt-1 truncate font-mono text-xs text-muted">
                      {provider.baseUrl}
                    </p>
                    <p className="mt-0.5 font-mono text-xs text-muted">
                      {t('密钥', 'Key')} {provider.keyHint || t('未设置', 'Not set')}
                    </p>
                    {result && (
                      <p
                        className={`mt-1 text-xs ${
                          result.ok ? 'text-success' : 'text-danger'
                        }`}
                      >
                        {result.ok
                          ? t(`连接正常，${result.latencyMs}ms`, `Connected, ${result.latencyMs}ms`)
                          : t(`测试失败：${result.message}`, `Test failed: ${result.message}`)}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <Toggle
                      checked={provider.enabled}
                      onChange={(value) => toggle(provider, value)}
                      label={t(`启用 ${provider.name}`, `Enable ${provider.name}`)}
                    />
                    <Button
                      size="xs"
                      variant="outline"
                      loading={testingId === provider.id}
                      onClick={() => test(provider)}
                    >
                      <Zap size={12} />
                      {t('测试', 'Test')}
                    </Button>
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => openEdit(provider)}
                    >
                      <Pencil size={12} />
                    </Button>
                    <Button
                      size="xs"
                      variant="ghost"
                      onClick={() => setRemoving(provider)}
                    >
                      <Trash2 size={12} className="text-danger" />
                    </Button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <Modal
        open={!!editing}
        onClose={() => setEditing(null)}
        title={editing?.id ? t('编辑供应商', 'Edit provider') : t('添加供应商', 'Add provider')}
        description={t(
          '密钥保存后不再回显，只能覆盖或删除。',
          'Saved keys are never shown again; you can only overwrite or delete them.',
        )}
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
            <Field label={t('名称', 'Name')}>
              <Input
                value={editing.name}
                placeholder={t('例如：公司网关 / 本地 Ollama', 'e.g. Company gateway / local Ollama')}
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </Field>
            <Field label={t('协议', 'Protocol')}>
              <Select
                value={editing.protocol}
                onChange={(e) => {
                  setModelOptions([]);
                  setEditing({
                    ...editing,
                    protocol: e.target.value as 'anthropic' | 'openai',
                  });
                }}
              >
                <option value="anthropic">Anthropic（/v1/messages）</option>
                <option value="openai">OpenAI（/v1/chat/completions）</option>
              </Select>
            </Field>
            <Field
              label="Base URL"
              hint={
                editing.protocol === 'anthropic'
                  ? t(
                      '例如 https://api.anthropic.com，不要以斜杠结尾',
                      'e.g. https://api.anthropic.com — no trailing slash',
                    )
                  : t(
                      '例如 https://api.openai.com/v1，不要以斜杠结尾',
                      'e.g. https://api.openai.com/v1 — no trailing slash',
                    )
              }
            >
              <Input
                value={editing.baseUrl}
                placeholder={
                  editing.protocol === 'anthropic'
                    ? 'https://api.anthropic.com'
                    : 'https://api.openai.com/v1'
                }
                spellCheck={false}
                onChange={(e) =>
                  setEditing({ ...editing, baseUrl: e.target.value })
                }
              />
            </Field>
            <Field
              label={t('模型', 'Model')}
              hint={t('测试连接与运行时会使用该默认模型名称', 'Used as the default model for connection tests and runs')}
            >
              <div className="flex gap-1.5">
                <Input
                  value={editing.model}
                  placeholder={t('例如 claude-sonnet-4-5 / gpt-4.1-mini', 'e.g. claude-sonnet-4-5 / gpt-4.1-mini')}
                  spellCheck={false}
                  onChange={(e) => setEditing({ ...editing, model: e.target.value })}
                />
                <Button
                  size="xs"
                  variant="outline"
                  className="shrink-0"
                  loading={fetchingModels}
                  onClick={fetchModels}
                >
                  <RefreshCw size={12} />
                  {t('获取模型', 'Fetch models')}
                </Button>
              </div>
            </Field>
            {modelOptions.length > 0 && (
              <div className="rounded-lg border border-line bg-base/40 px-2.5 py-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-xs text-muted">
                    {t(
                      '勾选要保存的模型，当前默认模型标为「默认」',
                      'Select the models to save; the current default is marked "Default"',
                    )}
                  </span>
                  <div className="flex shrink-0 items-center gap-2">
                    <button
                      type="button"
                      className="text-xs text-primary hover:underline"
                      onClick={() =>
                        setEditing({
                          ...editing,
                          models: modelOptions,
                          model: editing.model || modelOptions[0],
                        })
                      }
                    >
                      {t('全选', 'Select all')}
                    </button>
                    <button
                      type="button"
                      className="text-xs text-muted hover:text-danger"
                      onClick={() => setEditing({ ...editing, models: [], model: '' })}
                    >
                      {t('清空', 'Clear')}
                    </button>
                  </div>
                </div>
                <div className="mt-1.5 max-h-36 space-y-0.5 overflow-auto">
                  {modelOptions.map((model) => (
                    <label
                      key={model}
                      className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 hover:bg-elevated/60"
                    >
                      <input
                        type="checkbox"
                        className="accent-primary"
                        checked={editing.models.includes(model)}
                        onChange={() => toggleModel(model)}
                      />
                      <span className="font-mono text-xs text-ink">{model}</span>
                      {editing.model === model && (
                        <Badge tone="primary">{t('默认', 'Default')}</Badge>
                      )}
                    </label>
                  ))}
                </div>
              </div>
            )}
            <Field
              label="API Key"
              hint={
                editing.id
                  ? t(
                      '留空表示保持原密钥；输入新值会覆盖，保存后仅显示末四位',
                      'Leave empty to keep the current key; a new value overwrites it, and only the last four digits are shown after saving',
                    )
                  : t('加密保存，保存后仅显示末四位', 'Encrypted; only the last four digits are shown after saving')
              }
            >
              <Input
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                value={editing.apiKey}
                placeholder={
                  editing.id
                    ? t('已保存，留空不修改', 'Saved; leave empty to keep it')
                    : t('粘贴密钥', 'Paste key')
                }
                onChange={(e) =>
                  setEditing({ ...editing, apiKey: e.target.value })
                }
              />
            </Field>
            <div className="flex items-center justify-between rounded-lg border border-line bg-base/40 px-3 py-2">
              <span className="text-xs text-ink">{t('启用该供应商', 'Enable this provider')}</span>
              <Toggle
                checked={editing.enabled}
                onChange={(value) => setEditing({ ...editing, enabled: value })}
                label={t('启用供应商', 'Enable provider')}
              />
            </div>
          </div>
        )}
      </Modal>

      <ConfirmDialog
        open={!!removing}
        onClose={() => setRemoving(null)}
        onConfirm={remove}
        title={t(`删除供应商「${removing?.name ?? ''}」`, `Delete provider "${removing?.name ?? ''}"`)}
        description={t(
          '绑定该供应商的 Agent 将恢复使用 CLI 原有配置。',
          'Agents bound to this provider will revert to their CLI default config.',
        )}
        loading={deleting}
      />
    </Card>
  );
}
