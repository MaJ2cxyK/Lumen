export type CliBrand =
  | 'claude'
  | 'openai'
  | 'opencode'
  | 'gemini'
  | 'qwen'
  | 'copilot'
  | 'cursor'
  | 'amp'
  | 'aider'
  | 'cline'
  | 'roo'
  | 'kilo'
  | 'augment'
  | 'droid'
  | 'goose'
  | 'crush'
  | 'openhands'
  | 'kimi'
  | 'iflow'
  | 'trae'
  | 'qoder'
  | 'plandex'
  | 'continue'
  | 'codebuddy'
  | 'hermes'
  | 'pi'
  | 'terminal';

// Lumen-drawn monochrome lettermarks: no third-party logo artwork or brand
// colors are bundled.
const MARKS: Record<CliBrand, string> = {
  claude: 'CL',
  openai: 'OA',
  opencode: 'OC',
  gemini: 'GE',
  qwen: 'QW',
  copilot: 'CP',
  cursor: 'CU',
  amp: 'AM',
  aider: 'AD',
  cline: 'CN',
  roo: 'RO',
  kilo: 'KL',
  augment: 'AG',
  droid: 'DR',
  goose: 'GO',
  crush: 'CR',
  openhands: 'OH',
  kimi: 'KM',
  iflow: 'IF',
  trae: 'TR',
  qoder: 'QD',
  plandex: 'PL',
  continue: 'CT',
  codebuddy: 'CB',
  hermes: 'HE',
  pi: 'PI',
  terminal: '>_',
};

// 根据适配器 / 名称 / 启动命令推断品牌，用于展示对应的字母标。
export function cliBrand(input: {
  adapter?: string | null;
  name?: string | null;
  command?: string | null;
}): CliBrand {
  const name = (input.name ?? '').toLowerCase();
  const command = (input.command ?? '').toLowerCase();
  const text = `${name} ${input.adapter ?? ''} ${command}`.toLowerCase();
  if (text.includes('claude')) return 'claude';
  if (text.includes('opencode')) return 'opencode';
  if (text.includes('codex')) return 'openai';
  if (text.includes('gemini')) return 'gemini';
  if (text.includes('qwen')) return 'qwen';
  if (text.includes('copilot')) return 'copilot';
  if (text.includes('cursor')) return 'cursor';
  if (name === 'amp' || command.includes('amp')) return 'amp';
  if (text.includes('aider')) return 'aider';
  if (text.includes('cline')) return 'cline';
  if (text.includes('roo')) return 'roo';
  if (text.includes('kilo')) return 'kilo';
  if (text.includes('augment') || text.includes('auggie')) return 'augment';
  if (text.includes('droid') || text.includes('factory')) return 'droid';
  if (text.includes('goose')) return 'goose';
  if (text.includes('crush')) return 'crush';
  if (text.includes('openhands')) return 'openhands';
  if (text.includes('kimi') || text.includes('moonshot')) return 'kimi';
  if (text.includes('iflow')) return 'iflow';
  if (text.includes('trae')) return 'trae';
  if (text.includes('qoder')) return 'qoder';
  if (text.includes('plandex')) return 'plandex';
  if (text.includes('continue') || command === 'cn') return 'continue';
  if (text.includes('codebuddy')) return 'codebuddy';
  if (text.includes('hermes')) return 'hermes';
  if (name === 'pi' || /[\\/\s]pi(\.cmd|\.exe|\.bat)?$/i.test(text)) return 'pi';
  return 'terminal';
}

export function CliLogo({
  brand,
  size = 16,
  className,
}: {
  brand: CliBrand;
  size?: number;
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      aria-hidden
      className={className}
      style={{ flexShrink: 0 }}
    >
      <rect
        x="1"
        y="1"
        width="22"
        height="22"
        rx="6"
        fill="currentColor"
        opacity="0.14"
      />
      <text
        x="12"
        y="12.6"
        textAnchor="middle"
        dominantBaseline="middle"
        fontSize="9.5"
        fontWeight="700"
        fill="currentColor"
        style={{ letterSpacing: '-0.03em' }}
      >
        {MARKS[brand]}
      </text>
    </svg>
  );
}
