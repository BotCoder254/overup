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
import type {
  PipelineEvent,
  PipelineJob,
  PipelineJobMetrics,
} from '../../../../types/pipeline';
import { Field, FieldList, PanelSection } from './fields';
import { formatBytes } from '../../lib/format';

interface PerformancePanelProps {
  jobs: PipelineJob[];
  events: PipelineEvent[];
}

/** Prefer the job row's stored metrics; fall back to the `job.metrics`
 * ledger event for pipelines that ran before metrics were persisted. */
function metricsFor(job: PipelineJob, events: PipelineEvent[]): PipelineJobMetrics {
  if (job.metrics) return job.metrics;
  const event = events.find((e) => e.eventType === 'job.metrics' && e.jobId === job.id);
  return (event?.payload as PipelineJobMetrics | undefined) ?? {};
}

function permilleToPercent(permille: number | undefined): string | undefined {
  if (permille === undefined) return undefined;
  return `${(permille / 10).toFixed(permille < 100 ? 1 : 0)}%`;
}

/** Queue latency vs execution time per job, CPU peaks, and runner-reported
 * resource metrics (memory, network, disk, image pull). Palette-only. */
export function PerformancePanel({ jobs, events }: PerformancePanelProps) {
  const rows = useMemo(
    () =>
      jobs.map((job) => {
        const queuedMs = job.startedAt
          ? new Date(job.startedAt).getTime() - new Date(job.queuedAt).getTime()
          : 0;
        const execMs = job.startedAt
          ? (job.finishedAt ? new Date(job.finishedAt).getTime() : Date.now()) -
            new Date(job.startedAt).getTime()
          : 0;
        return {
          name: job.name ?? job.key,
          'Queue (s)': Math.max(Math.round(queuedMs / 100) / 10, 0),
          'Execution (s)': Math.max(Math.round(execMs / 100) / 10, 0),
          logBytes: job.logBytes,
          metrics: metricsFor(job, events),
        };
      }),
    [jobs, events],
  );

  const cpuData = rows
    .filter((row) => row.metrics.cpuPeakPermille !== undefined)
    .map((row) => ({
      name: row.name,
      'CPU peak (%)': Math.round((row.metrics.cpuPeakPermille ?? 0) / 10),
    }));

  const totalLogBytes = jobs.reduce((sum, job) => sum + job.logBytes, 0);

  return (
    <div>
      <PanelSection title="Queue vs execution time">
        <div className="h-56 w-full">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={rows} margin={{ top: 4, right: 8, bottom: 4, left: 0 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#78767133" />
              <XAxis dataKey="name" tick={{ fontSize: 11, fill: '#787671' }} />
              <YAxis tick={{ fontSize: 11, fill: '#787671' }} unit="s" width={44} />
              <Tooltip
                cursor={{ fill: '#f6f5f4' }}
                contentStyle={{
                  borderRadius: 6,
                  border: '1px solid #78767133',
                  fontSize: 12,
                }}
              />
              <Legend wrapperStyle={{ fontSize: 12 }} />
              {/* Solid palette only: steel for waiting, primary for doing. */}
              <Bar dataKey="Queue (s)" stackId="time" fill="#787671" radius={[0, 0, 0, 0]} />
              <Bar dataKey="Execution (s)" stackId="time" fill="#5645d4" radius={[6, 6, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </PanelSection>

      {cpuData.length > 0 && (
        <PanelSection title="CPU peak per job">
          <div className="h-40 w-full">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={cpuData} margin={{ top: 4, right: 8, bottom: 4, left: 0 }}>
                <CartesianGrid strokeDasharray="3 3" stroke="#78767133" />
                <XAxis dataKey="name" tick={{ fontSize: 11, fill: '#787671' }} />
                <YAxis tick={{ fontSize: 11, fill: '#787671' }} unit="%" width={44} />
                <Tooltip
                  cursor={{ fill: '#f6f5f4' }}
                  contentStyle={{
                    borderRadius: 6,
                    border: '1px solid #78767133',
                    fontSize: 12,
                  }}
                />
                {/* 100% = one full core; multi-core jobs exceed it. */}
                <Bar dataKey="CPU peak (%)" fill="#5645d4" radius={[6, 6, 0, 0]} />
              </BarChart>
            </ResponsiveContainer>
          </div>
        </PanelSection>
      )}

      <PanelSection title="Job metrics">
        <FieldList>
          {rows.map((row) => {
            const m = row.metrics;
            const parts = [
              m.imagePullMs !== undefined
                ? `image pull ${(m.imagePullMs / 1000).toFixed(1)}s`
                : undefined,
              m.cpuPeakPermille !== undefined
                ? `cpu peak ${permilleToPercent(m.cpuPeakPermille)}`
                : undefined,
              m.cpuAvgPermille !== undefined
                ? `avg ${permilleToPercent(m.cpuAvgPermille)}`
                : undefined,
              m.memPeakBytes !== undefined
                ? `mem peak ${formatBytes(m.memPeakBytes)}`
                : undefined,
              m.netRxBytes !== undefined || m.netTxBytes !== undefined
                ? `net ↓${formatBytes(m.netRxBytes ?? 0)} ↑${formatBytes(m.netTxBytes ?? 0)}`
                : undefined,
              m.blkioReadBytes !== undefined || m.blkioWriteBytes !== undefined
                ? `disk r${formatBytes(m.blkioReadBytes ?? 0)} w${formatBytes(m.blkioWriteBytes ?? 0)}`
                : undefined,
              `logs ${formatBytes(row.logBytes)}`,
            ].filter(Boolean);
            return (
              <Field key={row.name} label={row.name}>
                <span className="font-mono text-xs">{parts.join(' · ')}</span>
              </Field>
            );
          })}
          <Field label="Total log volume">
            <span className="font-mono text-xs">{formatBytes(totalLogBytes)}</span>
          </Field>
        </FieldList>
      </PanelSection>
    </div>
  );
}
