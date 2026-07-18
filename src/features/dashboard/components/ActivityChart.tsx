import { format } from 'date-fns';
import { AlertTriangle, BarChart3 } from 'lucide-react';
import { useMemo } from 'react';
import {
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';
import { EmptyState } from '../../../components/ui/EmptyState';
import type { ActivityBucket, DashboardRange } from '../../../types/dashboard';

interface ActivityChartProps {
  buckets: ActivityBucket[];
  range: DashboardRange;
  loading: boolean;
  error: boolean;
}

/** Pipeline execution volume over time, stacked by outcome. Solid palette only. */
export function ActivityChart({ buckets, range, loading, error }: ActivityChartProps) {
  const bucketFormat = range === '24h' ? 'HH:mm' : 'MMM d';
  const data = useMemo(
    () =>
      buckets.map((bucket) => ({
        name: format(new Date(bucket.bucket), bucketFormat),
        Succeeded: bucket.succeeded,
        Failed: bucket.failed,
        Cancelled: bucket.cancelled,
        Queued: bucket.queued,
      })),
    [buckets, bucketFormat],
  );

  if (loading) {
    return <div className="h-56 animate-pulse rounded bg-surface" />;
  }
  if (error) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Couldn't load chart data"
        description="Something went wrong fetching pipeline activity for this window."
        className="min-h-0 border-0 bg-transparent py-10"
      />
    );
  }
  if (data.length === 0) {
    return (
      <EmptyState
        icon={BarChart3}
        title="No activity yet"
        description="No pipeline activity in this window."
        className="min-h-0 border-0 bg-transparent py-10"
      />
    );
  }

  return (
    <div className="h-56 w-full">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 4, right: 8, bottom: 4, left: 0 }}>
          <CartesianGrid strokeDasharray="3 3" stroke="#8f8d8833" />
          <XAxis
            dataKey="name"
            tick={{ fontSize: 11, fill: '#8f8d88' }}
            interval="preserveStartEnd"
            angle={-35}
            textAnchor="end"
            height={40}
          />
          <YAxis tick={{ fontSize: 11, fill: '#8f8d88' }} width={32} allowDecimals={false} />
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
          <Legend wrapperStyle={{ fontSize: 12 }} />
          <Bar dataKey="Succeeded" stackId="activity" fill="#6a59e8" />
          <Bar dataKey="Failed" stackId="activity" fill="#e5484d" />
          <Bar dataKey="Cancelled" stackId="activity" fill="#8f8d88" />
          <Bar dataKey="Queued" stackId="activity" fill="#4d9fff" radius={[6, 6, 0, 0]} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
