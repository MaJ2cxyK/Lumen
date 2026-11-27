import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import type { UsagePoint } from '../lib/types';
import { formatNumber, formatTokens } from '../lib/format';
import { tr } from '../lib/i18n';

interface ChartRow {
  date: string;
  tokens: number;
  cost: number;
}

function ChartTip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: Array<{ payload: ChartRow }>;
  label?: string;
}) {
  if (!active || !payload || payload.length === 0) return null;
  const row = payload[0].payload;
  return (
    <div className="rounded-lg border border-line bg-modal px-3 py-2 text-xs shadow-[0_4px_12px_-4px_rgba(0,0,0,0.3)]">
      <p className="mb-1 text-xs text-muted">{label}</p>
      <p className="text-ink">
        Tokens <span className="ml-2 font-mono text-primary">{formatNumber(row.tokens)}</span>
      </p>
      <p className="text-ink">
        {tr('成本', 'Cost')}{' '}
        <span className="ml-2 font-mono text-success">${row.cost.toFixed(4)}</span>
      </p>
    </div>
  );
}

export function TrendChart({
  data,
  height = 230,
}: {
  data: UsagePoint[];
  height?: number;
}) {
  const rows: ChartRow[] = data.map((point) => ({
    date: point.date,
    tokens: point.tokens,
    cost: point.costMicros / 1_000_000,
  }));

  return (
    <ResponsiveContainer width="100%" height={height}>
      <ComposedChart data={rows} margin={{ top: 8, right: 4, bottom: 0, left: -8 }}>
        <defs>
          <linearGradient id="tokenFill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" style={{ stopColor: 'var(--color-primary)', stopOpacity: 0.16 }} />
            <stop offset="100%" style={{ stopColor: 'var(--color-primary)', stopOpacity: 0.01 }} />
          </linearGradient>
        </defs>
        <CartesianGrid stroke="var(--color-line)" strokeDasharray="3 4" vertical={false} />
        <XAxis
          dataKey="date"
          tickFormatter={(value: string) => value.slice(5)}
          tickLine={false}
          axisLine={{ stroke: 'var(--color-line)' }}
          dy={4}
        />
        <YAxis
          yAxisId="tokens"
          tickFormatter={(value: number) => formatTokens(value)}
          tickLine={false}
          axisLine={false}
          width={46}
        />
        <YAxis
          yAxisId="cost"
          orientation="right"
          tickFormatter={(value: number) => `$${value.toFixed(2)}`}
          tickLine={false}
          axisLine={false}
          width={44}
        />
        <Tooltip content={<ChartTip />} cursor={{ stroke: 'var(--color-muted)' }} />
        <Area
          yAxisId="tokens"
          type="monotone"
          dataKey="tokens"
          stroke="var(--color-primary)"
          strokeWidth={1.75}
          fill="url(#tokenFill)"
          dot={false}
          activeDot={{ r: 3, fill: 'var(--color-primary)', stroke: 'var(--color-base)' }}
          isAnimationActive={false}
        />
        <Line
          yAxisId="cost"
          type="monotone"
          dataKey="cost"
          stroke="var(--color-success)"
          strokeWidth={1.5}
          strokeDasharray="4 3"
          dot={false}
          activeDot={{ r: 3, fill: 'var(--color-success)', stroke: 'var(--color-base)' }}
          isAnimationActive={false}
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}
