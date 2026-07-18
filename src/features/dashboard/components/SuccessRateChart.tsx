import { AlertTriangle, PieChart } from 'lucide-react';
import { useMemo } from 'react';
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { EmptyState } from '../../../components/ui/EmptyState';
import type { DashboardSummary } from '../../../types/dashboard';

interface SuccessRateChartProps {
  summary: DashboardSummary | undefined;
  loading: boolean;
  error: boolean;
}

/** Outcome breakdown for the selected window. Solid palette only. */
export function SuccessRateChart({ summary, loading, error }: SuccessRateChartProps) {
  const data = useMemo(
    () =>
      summary
        ? [
            { name: 'Succeeded', count: summary.pipelinesSucceeded, fill: '#6a59e8' },
            { name: 'Failed', count: summary.pipelinesFailed, fill: '#e5484d' },
            { name: 'Cancelled', count: summary.pipelinesCancelled, fill: '#8f8d88' },
          ]
        : [],
    [summary],
  );

  if (loading) {
    return <div className="h-56 animate-pulse rounded bg-surface" />;
  }
  if (error) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Couldn't load chart data"
        description="Something went wrong fetching the outcome breakdown for this window."
        className="min-h-0 border-0 bg-transparent py-10"
      />
    );
  }
  if (!summary || summary.pipelinesSucceeded + summary.pipelinesFailed + summary.pipelinesCancelled === 0) {
    return (
      <EmptyState
        icon={PieChart}
        title="No completed runs"
        description="No completed runs in this window."
        className="min-h-0 border-0 bg-transparent py-10"
      />
    );
  }

  return (
    <div className="h-56 w-full">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} layout="vertical" margin={{ top: 4, right: 16, bottom: 4, left: 8 }}>
          <CartesianGrid strokeDasharray="3 3" stroke="#8f8d8833" />
          <XAxis type="number" tick={{ fontSize: 11, fill: '#8f8d88' }} allowDecimals={false} />
          <YAxis
            type="category"
            dataKey="name"
            tick={{ fontSize: 12, fill: '#e8e6e3' }}
            width={80}
          />
          <Tooltip
            cursor={{ fill: '#161614' }}
            contentStyle={{
              borderRadius: 6,
              border: '1px solid #8f8d8833',
              fontSize: 12,
              backgroundColor: '#161614',
              color: '#e8e6e3',
            }}
          />
          <Bar dataKey="count" radius={[0, 6, 6, 0]}>
            {data.map((entry) => (
              <Cell key={entry.name} fill={entry.fill} />
            ))}
          </Bar>
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
