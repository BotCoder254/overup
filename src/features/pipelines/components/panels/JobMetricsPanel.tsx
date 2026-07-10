import type { PipelineJob } from '../../../../types/pipeline';
import { formatBytes, formatDuration } from '../../lib/format';
import { Field, FieldList, PanelSection } from './fields';

function formatMs(ms: number | undefined): string {
  if (ms === undefined) return '—';
  const total = Math.max(Math.round(ms / 1000), 0);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  if (total > 0) return `${seconds}s`;
  return `${ms}ms`;
}

function permille(value: number | undefined): string {
  return value !== undefined ? `${(value / 10).toFixed(1)}% of one core` : '—';
}

/** Runner-reported, server-clamped resource telemetry for this job plus the
 * scheduling durations derived from its own timestamps. */
export function JobMetricsPanel({ job }: { job: PipelineJob }) {
  const metrics = job.metrics ?? {};

  return (
    <div>
      <PanelSection title="Timing">
        <FieldList>
          <Field label="Queue wait">{formatDuration(job.queuedAt, job.startedAt)}</Field>
          <Field label="Execution">
            {job.startedAt ? formatDuration(job.startedAt, job.finishedAt) : '—'}
          </Field>
          <Field label="Image pull">{formatMs(metrics.imagePullMs)}</Field>
          <Field label="Steps (measured)">{formatMs(metrics.execMs)}</Field>
        </FieldList>
      </PanelSection>

      <PanelSection title="Resources">
        <FieldList>
          <Field label="CPU peak">{permille(metrics.cpuPeakPermille)}</Field>
          <Field label="CPU average">{permille(metrics.cpuAvgPermille)}</Field>
          <Field label="Memory peak">
            {metrics.memPeakBytes !== undefined ? formatBytes(metrics.memPeakBytes) : '—'}
          </Field>
          <Field label="Network in">
            {metrics.netRxBytes !== undefined ? formatBytes(metrics.netRxBytes) : '—'}
          </Field>
          <Field label="Network out">
            {metrics.netTxBytes !== undefined ? formatBytes(metrics.netTxBytes) : '—'}
          </Field>
          <Field label="Disk read">
            {metrics.blkioReadBytes !== undefined ? formatBytes(metrics.blkioReadBytes) : '—'}
          </Field>
          <Field label="Disk write">
            {metrics.blkioWriteBytes !== undefined ? formatBytes(metrics.blkioWriteBytes) : '—'}
          </Field>
          <Field label="Samples">{metrics.sampleCount ?? '—'}</Field>
        </FieldList>
        {!job.metrics && (
          <p className="mt-2 text-xs text-steel">
            Resource metrics are reported by the runner when the job completes.
          </p>
        )}
      </PanelSection>
    </div>
  );
}
