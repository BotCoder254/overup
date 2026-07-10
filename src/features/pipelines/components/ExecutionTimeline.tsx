import { format } from 'date-fns';
import { cn } from '../../../lib/cn';
import type { Pipeline, PipelineJob } from '../../../types/pipeline';
import { formatDuration } from '../lib/format';
import { statusLabel } from './PipelineStatusBadge';

interface ExecutionTimelineProps {
  pipeline: Pipeline;
  jobs: PipelineJob[];
  selected?: string | null;
  onSelect?: (jobKey: string) => void;
}

/** Run-segment color per terminal/live state (solid palette only). */
function segmentClass(job: PipelineJob): string {
  if (job.status === 'in_progress') return 'bg-link';
  switch (job.conclusion) {
    case 'success':
      return 'bg-primary';
    case 'failure':
    case 'timed_out':
      return 'bg-danger';
    case 'skipped':
      return 'bg-steel/30';
    default:
      return 'bg-charcoal/40';
  }
}

/**
 * Horizontal lanes, one per job: steel = time spent queued/waiting on
 * dependencies, colored = execution. The scale spans pipeline creation to
 * completion (or now, while live).
 */
export function ExecutionTimeline({ pipeline, jobs, selected, onSelect }: ExecutionTimelineProps) {
  const startMs = new Date(pipeline.createdAt).getTime();
  const endMs = pipeline.finishedAt ? new Date(pipeline.finishedAt).getTime() : Date.now();
  const span = Math.max(endMs - startMs, 1000);

  const pct = (iso: string | null, fallback: number) => {
    if (!iso) return fallback;
    const at = new Date(iso).getTime();
    return Math.min(Math.max(((at - startMs) / span) * 100, 0), 100);
  };

  return (
    <div className="rounded border border-steel/20 bg-surface/50 p-3">
      <div className="mb-2 flex items-center justify-between text-[11px] text-steel">
        <span>{format(new Date(pipeline.createdAt), 'HH:mm:ss')}</span>
        <span>total {formatDuration(pipeline.createdAt, pipeline.finishedAt)}</span>
      </div>
      <div className="space-y-1.5">
        {jobs.map((job) => {
          const queuedLeft = pct(job.queuedAt, 0);
          const startedLeft = pct(job.startedAt, queuedLeft);
          const finishedLeft = pct(
            job.finishedAt,
            job.status === 'completed' ? startedLeft : 100,
          );
          const queueWidth = Math.max(startedLeft - queuedLeft, 0);
          const runWidth = Math.max(finishedLeft - startedLeft, job.startedAt ? 0.75 : 0);
          const title = [
            job.name ?? job.key,
            `state: ${statusLabel(job.status, job.conclusion)}`,
            `queued: ${format(new Date(job.queuedAt), 'PPpp')}`,
            job.startedAt ? `queue time: ${formatDuration(job.queuedAt, job.startedAt)}` : null,
            job.startedAt ? `execution: ${formatDuration(job.startedAt, job.finishedAt)}` : null,
            `attempt: ${job.attempt}`,
            job.exitCode !== null ? `exit code: ${job.exitCode}` : null,
            job.errorCategory ? `error: ${job.errorCategory.replaceAll('_', ' ')}` : null,
          ]
            .filter(Boolean)
            .join('\n');

          return (
            <button
              key={job.id}
              type="button"
              title={title}
              onClick={() => onSelect?.(job.key)}
              className={cn(
                'flex w-full items-center gap-2 rounded px-1 py-0.5 text-left transition-colors',
                selected === job.key ? 'bg-primary/10' : 'hover:bg-surface',
              )}
            >
              <span
                className={cn(
                  'w-28 shrink-0 truncate font-mono text-[11px]',
                  selected === job.key ? 'text-primary' : 'text-charcoal',
                )}
              >
                {job.name ?? job.key}
              </span>
              <span className="relative h-3 min-w-0 flex-1 overflow-hidden rounded bg-steel/10">
                {/* queue-latency segment */}
                {queueWidth > 0 && (
                  <span
                    className="absolute inset-y-0 rounded-none bg-steel/30"
                    style={{ left: `${queuedLeft}%`, width: `${queueWidth}%` }}
                  />
                )}
                {/* execution segment */}
                {job.startedAt && (
                  <span
                    className={cn(
                      'absolute inset-y-0 rounded',
                      segmentClass(job),
                      job.status === 'in_progress' && 'animate-pulse',
                    )}
                    style={{ left: `${startedLeft}%`, width: `${Math.max(runWidth, 0.75)}%` }}
                  />
                )}
              </span>
              <span className="w-14 shrink-0 text-right font-mono text-[11px] text-steel">
                {job.startedAt ? formatDuration(job.startedAt, job.finishedAt) : '—'}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
