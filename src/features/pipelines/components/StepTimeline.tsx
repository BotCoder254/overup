import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  MinusCircle,
  XCircle,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { Spinner } from '../../../components/ui/Spinner';
import { cn } from '../../../lib/cn';
import { useNow } from '../../../lib/useNow';
import type { PipelineJob } from '../../../types/pipeline';
import { useLogStore } from '../stores/logStore';

type StepState = 'pending' | 'running' | 'done' | 'failed' | 'stopped';

/**
 * Marker format the reference runner writes to the system log stream before
 * each step (runner/src/executor.rs). Tolerant: only the prefix and the
 * step ordinal matter, and unparseable logs simply degrade the timeline to
 * a static plan list — never an error.
 */
const STEP_MARKER = /^▶ step (\d+)\/\d+: /;

interface StepInfo {
  state: StepState;
  /** Server receive time of the step's start marker, when logs carry one. */
  startedAt?: string;
  /** Approximate: derived from inter-marker log timestamps, not the runner clock. */
  durationMs?: number;
}

function stateIcon(state: StepState) {
  switch (state) {
    case 'done':
      return <CheckCircle2 size={15} className="text-primary" aria-hidden="true" />;
    case 'failed':
      return <XCircle size={15} className="text-danger" aria-hidden="true" />;
    case 'running':
      return <Spinner className="h-3.5 w-3.5 text-link" />;
    case 'stopped':
      return <MinusCircle size={15} className="text-charcoal/50" aria-hidden="true" />;
    default:
      return <Circle size={15} className="text-steel/50" aria-hidden="true" />;
  }
}

function formatApproxMs(ms: number): string {
  const total = Math.max(Math.round(ms / 1000), 0);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `~${minutes}m ${seconds}s` : `~${seconds}s`;
}

/**
 * The ordered step timeline: every planned step as an expandable block with
 * a live state derived from the runner's system-log step markers. Timing is
 * shown with `~` because it comes from log receive times — when markers are
 * absent (logs pruned, job not yet watched) the list degrades gracefully to
 * the static plan with no fabricated timing.
 */
export function StepTimeline({ job }: { job: PipelineJob }) {
  const chunks = useLogStore((state) => state.jobs[job.id]?.chunks);
  const now = useNow(job.status === 'in_progress');
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const steps = job.plan.steps;
  const infos = useMemo<StepInfo[]>(() => {
    // Step ordinal (1-based) -> start marker receive time.
    const markers = new Map<number, string | undefined>();
    for (const chunk of chunks ?? []) {
      if (chunk.stream !== 'system') continue;
      for (const line of chunk.content.split('\n')) {
        const match = STEP_MARKER.exec(line);
        if (match) markers.set(Number(match[1]), chunk.createdAt);
      }
    }
    const current = markers.size > 0 ? Math.max(...Array.from(markers.keys())) : 0;
    const finished = job.status === 'completed';
    const failedRun = job.conclusion === 'failure' && job.errorCategory === 'step_failed';

    return steps.map((_, index) => {
      const ordinal = index + 1;
      const startedAt = markers.get(ordinal);

      let state: StepState = 'pending';
      if (markers.has(ordinal)) {
        if (ordinal < current) {
          state = 'done';
        } else if (!finished) {
          state = 'running';
        } else if (job.conclusion === 'success') {
          state = 'done';
        } else if (failedRun) {
          state = 'failed';
        } else {
          state = 'stopped';
        }
      } else if (finished && job.conclusion === 'success' && markers.size === 0) {
        // Successful job whose hot logs are gone: every step completed.
        state = 'done';
      }

      let durationMs: number | undefined;
      if (startedAt) {
        const next = markers.get(ordinal + 1);
        const endIso = next ?? job.finishedAt ?? undefined;
        const endMs = endIso ? new Date(endIso).getTime() : now;
        durationMs = Math.max(endMs - new Date(startedAt).getTime(), 0);
      }
      return { state, startedAt, durationMs };
    });
  }, [chunks, steps, job.status, job.conclusion, job.errorCategory, job.finishedAt, now]);

  if (steps.length === 0) {
    return (
      <div className="rounded border border-steel/20 bg-surface/50 p-3">
        <p className="text-sm text-steel">This job has no runnable steps.</p>
      </div>
    );
  }

  return (
    <div className="rounded border border-steel/20 bg-surface/50 p-2">
      <div className="mb-1 flex items-center justify-between px-1 text-[11px] text-steel">
        <span className="font-semibold uppercase tracking-wide">Steps</span>
        <span>{steps.length} planned</span>
      </div>
      <ol className="space-y-1">
        {steps.map((step, index) => {
          const info = infos[index];
          const isOpen = expanded.has(index);
          return (
            <li key={`${index}-${step.name}`} className="rounded bg-canvas">
              <button
                type="button"
                aria-expanded={isOpen}
                onClick={() =>
                  setExpanded((current) => {
                    const next = new Set(current);
                    if (next.has(index)) next.delete(index);
                    else next.add(index);
                    return next;
                  })
                }
                className={cn(
                  'flex w-full items-center gap-2 rounded border border-steel/20 px-2 py-1.5 text-left transition-colors hover:bg-surface',
                  info.state === 'running' && 'border-link/40',
                  info.state === 'failed' && 'border-danger/40',
                )}
              >
                {isOpen ? (
                  <ChevronDown size={13} className="shrink-0 text-steel" aria-hidden="true" />
                ) : (
                  <ChevronRight size={13} className="shrink-0 text-steel" aria-hidden="true" />
                )}
                <span className="shrink-0">{stateIcon(info.state)}</span>
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-charcoal">
                  {index + 1}. {step.name}
                </span>
                <span className="hidden shrink-0 font-mono text-[10px] text-steel sm:inline">
                  {step.shell}
                </span>
                <span className="w-16 shrink-0 text-right font-mono text-[11px] text-steel">
                  {info.durationMs !== undefined ? formatApproxMs(info.durationMs) : '—'}
                </span>
              </button>
              {isOpen && (
                <div className="border-x border-b border-steel/20 px-2 py-1.5">
                  <pre className="overflow-x-auto whitespace-pre-wrap break-all font-mono text-[11px] text-steel">
                    {step.run}
                  </pre>
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
