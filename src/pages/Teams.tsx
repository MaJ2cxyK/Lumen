import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Bot,
  Briefcase,
  Code2,
  Plus,
  ShieldCheck,
  Sparkles,
  Trash2,
  TrendingUp,
  Users,
  Wand2,
  type LucideIcon,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, errorMessage } from '../lib/api';
import type { Seat, Team, TeamTemplate } from '../lib/types';
import {
  TEMPLATE_META,
  capabilityLabel,
  roleDescription,
  roleName,
} from '../lib/labels';
import { tr, useI18n } from '../lib/i18n';
import { useAppStore } from '../store';
import { hiddenCliNames } from '../lib/visibility';
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
  Textarea,
  Toggle,
  cn,
} from '../components/ui';

const TEMPLATE_ICON: Record<string, LucideIcon> = {
  code: Code2,
  briefcase: Briefcase,
  office: Briefcase,
  trending: TrendingUp,
};

export default function Teams({ embedded = false }: { embedded?: boolean }) {
  const { t } = useI18n();
  const projects = useAppStore((s) => s.projects);
  const agents = useAppStore((s) => s.agents);
  const loadCore = useAppStore((s) => s.loadCore);
  const hiddenAgentNames = hiddenCliNames();
  const [loaded, setLoaded] = useState(false);

  const [projectId, setProjectId] = useState('');
  const [teams, setTeams] = useState<Team[]>([]);
  const [templates, setTemplates] = useState<TeamTemplate[]>([]);
  const [teamId, setTeamId] = useState('');
  const [seats, setSeats] = useState<Seat[]>([]);
  const [teamsLoading, setTeamsLoading] = useState(false);
  const [seatsLoading, setSeatsLoading] = useState(false);
  const [autoLoading, setAutoLoading] = useState(false);
  const [reordering, setReordering] = useState(false);

  const [modalOpen, setModalOpen] = useState(false);
  const [templateKey, setTemplateKey] = useState('software-dev');
  const [teamName, setTeamName] = useState('');
  const [teamDesc, setTeamDesc] = useState('');
  const [saving, setSaving] = useState(false);
  const [deletingTeam, setDeletingTeam] = useState<Team | null>(null);
  const [deleteLoading, setDeleteLoading] = useState(false);

  useEffect(() => {
    let active = true;
    (async () => {
      try {
        const [templateList] = await Promise.all([api.teamTemplates(), loadCore()]);
        if (!active) return;
        setTemplates(templateList);
      } catch (err) {
        toast.error(errorMessage(err));
      } finally {
        if (active) setLoaded(true);
      }
    })();
    return () => {
      active = false;
    };
  }, [loadCore]);

  useEffect(() => {
    if (!projectId && projects.length > 0) setProjectId(projects[0].id);
  }, [projects, projectId]);

  useEffect(() => {
    if (!projectId) {
      setTeams([]);
      setTeamId('');
      return;
    }
    let active = true;
    setTeamsLoading(true);
    api
      .teamsList(projectId)
      .then((list) => {
        if (!active) return;
        setTeams(list);
        setTeamId((current) => (list.some((t) => t.id === current) ? current : list[0]?.id ?? ''));
      })
      .catch((err) => toast.error(errorMessage(err)))
      .finally(() => {
        if (active) setTeamsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [projectId]);

  useEffect(() => {
    if (!teamId) {
      setSeats([]);
      return;
    }
    let active = true;
    setSeatsLoading(true);
    api
      .seatsList(teamId)
      .then((list) => {
        if (active) setSeats(list);
      })
      .catch((err) => toast.error(errorMessage(err)))
      .finally(() => {
        if (active) setSeatsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [teamId]);

  const currentTeam = teams.find((team) => team.id === teamId) ?? null;
  const currentTemplate = templates.find((t) => t.key === currentTeam?.templateKey);
  const modalTemplate = templates.find((t) => t.key === templateKey);

  const orderedSeats = seats;

  async function moveSeat(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= orderedSeats.length || reordering) return;
    const ids = orderedSeats.map((seat) => seat.id);
    [ids[index], ids[target]] = [ids[target], ids[index]];
    setReordering(true);
    try {
      await api.seatReorder(teamId, ids);
      await reloadSeats();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setReordering(false);
    }
  }

  async function reloadTeams(preferredId?: string) {
    if (!projectId) return;
    try {
      const list = await api.teamsList(projectId);
      setTeams(list);
      if (preferredId) setTeamId(preferredId);
      else if (!list.some((t) => t.id === teamId)) setTeamId(list[0]?.id ?? '');
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function reloadSeats() {
    if (!teamId) return;
    try {
      setSeats(await api.seatsList(teamId));
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  function openCreate() {
    if (!projectId) {
      toast.error(t('请先创建项目', 'Create a project first'));
      return;
    }
    const template = templates[0];
    setTemplateKey(template?.key ?? 'software-dev');
    setTeamName(template?.name ?? '');
    setTeamDesc(template?.description ?? '');
    setModalOpen(true);
  }

  function chooseTemplate(key: string) {
    setTemplateKey(key);
    const template = templates.find((t) => t.key === key);
    if (template) {
      setTeamName(template.name);
      setTeamDesc(template.description);
    }
  }

  async function createTeam() {
    if (!teamName.trim()) {
      toast.error(t('请填写团队名称', 'Enter a team name'));
      return;
    }
    setSaving(true);
    try {
      const id = await api.teamCreate({
        projectId,
        name: teamName.trim(),
        templateKey,
        description: teamDesc.trim() || undefined,
      });
      toast.success(t('团队已创建', 'Team created'));
      setModalOpen(false);
      await reloadTeams(id);
      await loadCore();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function autoAssign() {
    if (!teamId) return;
    setAutoLoading(true);
    try {
      const count = await api.seatAutoAssign(teamId);
      toast.success(
        count > 0
          ? t(`已自动指派 ${count} 个席位`, `${count} seats auto-assigned`)
          : t('所有席位均已指派', 'All seats are assigned'),
      );
      await reloadSeats();
      await reloadTeams();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setAutoLoading(false);
    }
  }

  async function assignAgent(seat: Seat, agentId: string) {
    try {
      await api.seatAssign(seat.id, agentId || null);
      await reloadSeats();
      await reloadTeams();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function toggleGate(seat: Seat, gate: boolean) {
    try {
      await api.seatUpdate({
        seatId: seat.id,
        roleName: seat.roleName,
        responsibility: seat.responsibility ?? undefined,
        gate,
      });
      await reloadSeats();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function confirmDeleteTeam() {
    if (!deletingTeam) return;
    setDeleteLoading(true);
    try {
      await api.teamDelete(deletingTeam.id);
      toast.success(t('团队已删除', 'Team deleted'));
      setDeletingTeam(null);
      await reloadTeams();
      await loadCore();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setDeleteLoading(false);
    }
  }

  const capabilityOf = (seat: Seat) => {
    if (!currentTemplate) return '';
    return (
      currentTemplate.roles.find((role) => role.roleKey === seat.roleKey)?.capability ?? ''
    );
  };

  return (
    <PageScroll embedded={embedded}>
      <PageHeader
        back={
          embedded ? undefined : { to: '/settings', label: t('返回设置', 'Back to settings') }
        }
        title={t('团队与席位', 'Teams & seats')}
        actions={
          <>
            <Select
              value={projectId}
              onChange={(e) => setProjectId(e.target.value)}
              className="w-52"
            >
              {projects.length === 0 && (
                <option value="">{t('暂无项目', 'No projects')}</option>
              )}
              {projects.map((project) => (
                <option key={project.id} value={project.id}>
                  {project.name}
                </option>
              ))}
            </Select>
            <Button size="sm" onClick={openCreate} disabled={projects.length === 0}>
              <Plus size={14} />
              {t('新建团队', 'New team')}
            </Button>
          </>
        }
      />

      {!loaded ? (
        <Loading />
      ) : projects.length === 0 ? (
        <Empty
          icon={Users}
          title={t('还没有项目', 'No projects yet')}
          description={t(
            '团队必须归属于项目，请先到「新建任务」页添加项目目录',
            'Teams belong to projects. Add a project directory on the “New task” page first.',
          )}
          action={
            <Link to="/projects">
              <Button size="sm">{t('前往新建任务', 'Go to New task')}</Button>
            </Link>
          }
          className="py-16"
        />
      ) : (
        <div className="space-y-5">
          {teamsLoading ? (
            <Loading label={t('加载团队', 'Loading teams')} />
          ) : teams.length === 0 ? (
            <Empty
              icon={Users}
              title={t('该项目还没有团队', 'No teams in this project')}
              description={t(
                '选择模板快速创建，席位会自动生成',
                'Pick a template to create one; seats are generated automatically',
              )}
              action={
                <Button size="sm" onClick={openCreate}>
                  <Plus size={14} />
                  {t('新建团队', 'New team')}
                </Button>
              }
              className="py-12"
            />
          ) : (
            <div className="flex flex-wrap gap-2.5">
              {teams.map((team) => (
                <button
                  key={team.id}
                  onClick={() => setTeamId(team.id)}
                  className={cn(
                    'group relative w-[240px] rounded-xl border p-3 text-left transition-colors',
                    team.id === teamId
                      ? 'border-primary/60 bg-primary/10'
                      : 'border-line bg-card hover:border-line/80 hover:bg-hover',
                  )}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-semibold text-ink">{team.name}</p>
                      <p className="mt-1 text-xs text-muted">
                        {TEAM_META(team.templateKey)}
                      </p>
                    </div>
                    <Button
                      variant="ghost"
                      size="xs"
                      className="opacity-0 group-hover:opacity-100 hover:text-danger"
                      onClick={(event) => {
                        event.stopPropagation();
                        setDeletingTeam(team);
                      }}
                      title={t('删除团队', 'Delete team')}
                    >
                      <Trash2 size={12} />
                    </Button>
                  </div>
                  <div className="mt-2.5 flex items-center justify-between">
                    <div className="flex items-center gap-1 text-xs text-muted">
                      <Bot size={11} />
                      {t(
                        `${team.assignedCount}/${team.seatsCount} 席位已指派`,
                        `${team.assignedCount}/${team.seatsCount} seats assigned`,
                      )}
                    </div>
                    <div className="h-1 w-16 overflow-hidden rounded-full bg-soft2">
                      <div
                        className={cn(
                          'h-full rounded-full',
                          team.assignedCount >= team.seatsCount ? 'bg-success' : 'bg-primary',
                        )}
                        style={{
                          width: `${
                            team.seatsCount > 0
                              ? Math.max(
                                  4,
                                  Math.round((team.assignedCount / team.seatsCount) * 100),
                                )
                              : 0
                          }%`,
                        }}
                      />
                    </div>
                  </div>
                </button>
              ))}
            </div>
          )}

          {currentTeam && (
            <div>
              <div className="mb-3 flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <h2 className="text-sm font-semibold text-ink">{currentTeam.name}</h2>
                  <Badge tone="primary">{TEAM_META(currentTeam.templateKey)}</Badge>
                  <span className="text-xs text-muted">
                    {t(
                      `${currentTeam.assignedCount}/${currentTeam.seatsCount} 已指派`,
                      `${currentTeam.assignedCount}/${currentTeam.seatsCount} assigned`,
                    )}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="hidden items-center gap-1 text-xs text-muted sm:flex">
                    <ArrowUpDown size={11} />
                    {t('可调整展示顺序', 'Order can be adjusted')}
                  </span>
                  <Button variant="outline" size="sm" loading={autoLoading} onClick={autoAssign}>
                    <Wand2 size={13} />
                    {t('自动指派', 'Auto-assign')}
                  </Button>
                </div>
              </div>

              {seatsLoading ? (
                <Loading label={t('加载席位', 'Loading seats')} />
              ) : orderedSeats.length === 0 ? (
                <Empty title={t('该团队暂无席位', 'No seats in this team')} className="py-10" />
              ) : (
                <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
                  {orderedSeats.map((seat, index) => {
                    const capability = capabilityOf(seat);
                    return (
                      <Card key={seat.id} className="relative">
                        <div className="flex items-start justify-between gap-3">
                          <div className="flex min-w-0 gap-2.5">
                            <span className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-md border border-line bg-base/60 font-mono text-xs text-muted">
                              {String(index + 1).padStart(2, '0')}
                            </span>
                            <div className="min-w-0">
                              <div className="flex flex-wrap items-center gap-1.5">
                                <p className="text-sm font-semibold text-ink">
                                  {roleName(seat.roleKey, seat.roleName)}
                                </p>
                                {seat.gate && (
                                  <Badge>
                                    <span className="h-1.5 w-1.5 rounded-full bg-warn" />
                                    {t('门禁', 'Gate')}
                                  </Badge>
                                )}
                              </div>
                              <p className="mt-1 text-xs leading-4 text-muted">
                                {seat.responsibility
                                  ? roleDescription(seat.roleKey, seat.responsibility)
                                  : t('暂无职责说明', 'No responsibility set')}
                              </p>
                              <div className="mt-2 flex flex-wrap items-center gap-1.5">
                                {capability && (
                                  <Badge tone="primary">{capabilityLabel(capability)}</Badge>
                                )}
                                <Badge>{seat.roleKey}</Badge>
                              </div>
                            </div>
                          </div>
                          <div className="flex shrink-0 flex-col items-end gap-1.5">
                            <div className="flex items-center gap-0.5">
                              <Button
                                variant="ghost"
                                size="xs"
                                disabled={index === 0}
                                onClick={() => moveSeat(index, -1)}
                                title={t('上移', 'Move up')}
                              >
                                <ArrowUp size={12} />
                              </Button>
                              <Button
                                variant="ghost"
                                size="xs"
                                disabled={index === orderedSeats.length - 1}
                                onClick={() => moveSeat(index, 1)}
                                title={t('下移', 'Move down')}
                              >
                                <ArrowDown size={12} />
                              </Button>
                            </div>
                            <Select
                              value={seat.agentId ?? ''}
                              onChange={(e) => assignAgent(seat, e.target.value)}
                              className="h-8 w-[150px] py-0 text-xs"
                            >
                              <option value="">{t('未指派 Agent', 'Unassigned')}</option>
                              {agents
                                .filter(
                                  (agent) =>
                                    !hiddenAgentNames.has(agent.name) ||
                                    agent.id === seat.agentId,
                                )
                                .map((agent) => (
                                  <option key={agent.id} value={agent.id}>
                                    {agent.name}
                                    {hiddenAgentNames.has(agent.name) ? t('（已隐藏）', ' (hidden)') : ''}
                                  </option>
                                ))}
                            </Select>
                          </div>
                        </div>

                        <div className="mt-3 flex items-center justify-between border-t border-line/60 pt-2.5">
                          <div className="flex items-center gap-1.5 text-xs text-muted">
                            <ShieldCheck size={12} />
                            {t('阶段门禁', 'Stage gate')}
                            <span className="text-muted/60">
                              {t('，完成后暂停等待审批', ', pause for approval when done')}
                            </span>
                          </div>
                          <Toggle
                            checked={seat.gate}
                            onChange={(value) => toggleGate(seat, value)}
                            label={`${roleName(seat.roleKey, seat.roleName)} ${t('门禁', 'gate')}`}
                          />
                        </div>
                      </Card>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      <Modal
        open={modalOpen}
        onClose={() => setModalOpen(false)}
        title={t('新建团队', 'New team')}
        description={t(
          '选择模板后会自动生成对应的席位与职责',
          'Pick a template to generate matching seats and responsibilities',
        )}
        width="max-w-xl"
        footer={
          <>
            <Button variant="outline" size="sm" onClick={() => setModalOpen(false)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button size="sm" loading={saving} onClick={createTeam}>
              {t('创建团队', 'Create team')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
            {templates.map((template) => {
              const Icon = TEMPLATE_ICON[template.icon] ?? Sparkles;
              const selected = template.key === templateKey;
              return (
                <button
                  key={template.key}
                  onClick={() => chooseTemplate(template.key)}
                  className={cn(
                    'rounded-lg border p-3 text-left transition-colors',
                    selected
                      ? 'border-primary/60 bg-primary/10'
                      : 'border-line bg-base/40 hover:bg-hover',
                  )}
                >
                  <Icon size={16} className={selected ? 'text-primary' : 'text-muted'} />
                  <p className="mt-2 text-xs font-semibold text-ink">{template.name}</p>
                  <p className="mt-1 text-xs leading-4 text-muted">
                    {template.description}
                  </p>
                  <p className="mt-1.5 text-xs text-primary/80">
                    {t(`${template.roles.length} 个席位`, `${template.roles.length} seats`)}
                  </p>
                </button>
              );
            })}
          </div>
          <Field label={t('团队名称', 'Team name')}>
            <Input value={teamName} onChange={(e) => setTeamName(e.target.value)} />
          </Field>
          <Field label={t('描述', 'Description')}>
            <Textarea
              rows={2}
              value={teamDesc}
              onChange={(e) => setTeamDesc(e.target.value)}
              placeholder={t('这个团队负责什么', 'What does this team handle?')}
            />
          </Field>
          {modalTemplate && (
            <div className="rounded-lg border border-line/70 bg-base/40 px-3 py-2">
              <p className="text-xs text-muted">{t('将生成以下席位', 'Seats to be created')}</p>
              <p className="mt-1 text-xs leading-5 text-ink/80">
                {modalTemplate.roles.map((role) => roleName(role.roleKey, role.roleName)).join(' → ')}
              </p>
            </div>
          )}
        </div>
      </Modal>

      <ConfirmDialog
        open={Boolean(deletingTeam)}
        onClose={() => setDeletingTeam(null)}
        onConfirm={confirmDeleteTeam}
        title={t(
          `删除团队「${deletingTeam?.name ?? ''}」`,
          `Delete team “${deletingTeam?.name ?? ''}”`,
        )}
        description={t(
          '团队下的席位会一并删除，关联任务将解除团队',
          'Seats under this team will be removed, and linked tasks will be unassigned from it',
        )}
        loading={deleteLoading}
      />
    </PageScroll>
  );
}

function TEAM_META(templateKey?: string | null): string {
  if (!templateKey) return tr('自定义团队', 'Custom team');
  return TEMPLATE_META[templateKey]?.name ?? templateKey;
}
