import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Eye,
  EyeOff,
  MinusCircle,
  ScrollText,
  XCircle,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { Spinner } from '../../../components/ui/Spinner';
import { cn } from '../../../lib/cn';
import { useNow } from '../../../lib/useNow';
import type { PipelineEvent, PipelineJob } from '../../../types/pipeline';
import { useLogStore } from '../stores/logStore';

type StepState = 'pending' | 'running' | 'done' | 'failed' | 'stopped';

/**
 * Marker format the reference runner writes to the system log stream before
 * each step (runner/src/executor.rs). Fallback only — structured
 * job.step_started/finished events are preferred when present. Tolerant:
 * only the prefix and the step ordinal matter, and unparseable logs simply
 * degrade the timeline to a static plan list — never an error.
 */
const STEP_MARKER = /^▶ step (\d+)\/\d+: /;

interface StepInfo {
  state: StepState;
  /** Start time (event time when structured, log receive time otherwise). */
  startedAt?: string;
  durationMs?: number;
  /** Duration is exact (event-sourced) rather than log-approximate. */
  exact: boolean;
  exitCode?: number | null;
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

function formatMs(ms: number, exact: boolean): string {
  const total = Math.max(Math.round(ms / 1000), 0);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  const text = minutes > 0 ? `${minutes}m ${seconds}s` : `${seconds}s`;
  return exact ? text : `~${text}`;
}

interface StepTimelineProps {
  job: PipelineJob;
  /** This job's events; job.step_started/finished drive precise states. */
  events?: PipelineEvent[];
  /** Lifted log-section visibility: keys like `step:2` (shared with the
   * LogViewer). When present each row gets a show/hide toggle. */
  hiddenSections?: Set<string>;
  onToggleSection?: (key: string) => void;
  /** Scroll the terminal to a step's log section. */
  onJumpToSection?: (key: string) => void;
}

/**
 * The ordered step timeline: every planned step as an expandable block with
 * a live state. States, durations, and exit codes come from the structured
 * job.step_started/finished ledger events when available (exact, filtered
 * to the current attempt) and degrade to the runner's system-log step
 * markers (`~` timing from log receive times) for old runners or pruned
 * event history — when both are absent the list is the static plan with no
 * fabricated timing.
 */
export function StepTimeline({
  job,
  events,
  hiddenSections,
  onToggleSection,
  onJumpToSection,
}: StepTimelineProps) {
  const chunks = useLogStore((state) => state.jobs[job.id]?.chunks);
  const now = useNow(job.status === 'in_progress');
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  const steps = job.plan.steps;
  const infos = useMemo<StepInfo[]>(() => {
    // Structured per-step progress from the events ledger, current attempt
    // only so a rerun never inherits attempt 1's exit codes.
    const structured = new Map<
      number,
      { startedAt?: string; finishedAt?: string; status?: string; exitCode?: number | null }
    >();
    for (const event of events ?? []) {
      if (event.eventType !== 'job.step_started' && event.eventType !== 'job.step_finished') {
        continue;
      }
      const payload = event.payload ?? {};
      const attempt = payload.attempt;
      if (typeof attempt === 'number' && attempt !== job.attempt) continue;
      const index = payload.stepIndex;
      if (typeof index !== 'number' || index < 0 || index >= steps.length) continue;
      const entry = structured.get(index) ?? {};
      if (event.eventType === 'job.step_started') {
        entry.startedAt = event.createdAt;
      } else {
        entry.finishedAt = event.createdAt;
        entry.status = typeof payload.status === 'string' ? payload.status : undefined;
        entry.exitCode = typeof payload.exitCode === 'number' ? payload.exitCode : null;
      }
      structured.set(index, entry);
    }

    // Fallback: step ordinal (1-based) -> start marker receive time.
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
      const record = structured.get(index);
      if (record?.startedAt || record?.finishedAt) {
        let state: StepState = 'running';
        if (record.status === 'succeeded') state = 'done';
        else if (record.status === 'failed') state = 'failed';
        else if (finished) state = job.conclusion === 'success' ? 'done' : 'stopped';

        let durationMs: number | undefined;
        if (record.startedAt) {
          const endIso = record.finishedAt ?? (finished ? job.finishedAt ?? undefined : undefined);
          const endMs = endIso ? new Date(endIso).getTime() : now;
          durationMs = Math.max(endMs - new Date(record.startedAt).getTime(), 0);
        }
        return {
          state,
          startedAt: record.startedAt,
          durationMs,
          exact: Boolean(record.finishedAt),
          exitCode: record.exitCode,
        };
      }

      // Marker fallback (old runners / pre-structured logs).
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
      return { state, startedAt, durationMs, exact: false, exitCode: undefined };
    });
  }, [
    events,
    chunks,
    steps,
    job.attempt,
    job.status,
    job.conclusion,
    job.errorCategory,
    job.finishedAt,
    now,
  ]);

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
          const sectionKey = `step:${index}`;
          const isHidden = hiddenSections?.has(sectionKey) ?? false;
          return (
            <li key={`${index}-${step.name}`} className="rounded bg-canvas">
              <div
                className={cn(
                  'flex w-full items-center gap-1 rounded border border-steel/20 transition-colors',
                  info.state === 'running' && 'border-link/40',
                  info.state === 'failed' && 'border-danger/40',
                )}
              >
                <button
                  type="button"
                  aria-expanded={isOpen}
                  onClick={() =>
                    setExpanded((currentSet) => {
                      const next = new Set(currentSet);
                      if (next.has(index)) next.delete(index);
                      else next.add(index);
                      return next;
                    })
                  }
                  className="flex min-w-0 flex-1 items-center gap-2 rounded px-2 py-1.5 text-left transition-colors hover:bg-surface"
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
                  {info.state === 'failed' && info.exitCode != null && (
                    <span className="shrink-0 rounded border border-danger/40 bg-danger/10 px-1 font-mono text-[10px] text-danger">
                      exit {info.exitCode}
                    </span>
                  )}
                  <span className="w-14 shrink-0 text-right font-mono text-[11px] text-steel">
                    {info.durationMs !== undefined ? formatMs(info.durationMs, info.exact) : '—'}
                  </span>
                </button>
                {onJumpToSection && (
                  <button
                    type="button"
                    onClick={() => onJumpToSection(sectionKey)}
                    title="Jump to this step's log output"
                    aria-label={`Jump to logs of step ${index + 1}`}
                    className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  >
                    <ScrollText size={12} aria-hidden="true" />
                  </button>
                )}
                {onToggleSection && (
                  <button
                    type="button"
                    onClick={() => onToggleSection(sectionKey)}
                    title={isHidden ? 'Show this step in the log' : 'Collapse this step in the log'}
                    aria-pressed={isHidden}
                    aria-label={`Toggle log visibility of step ${index + 1}`}
                    className={cn(
                      'mr-1 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                      isHidden
                        ? 'bg-primary/10 text-primary'
                        : 'text-steel hover:bg-surface hover:text-charcoal',
                    )}
                  >
                    {isHidden ? (
                      <EyeOff size={12} aria-hidden="true" />
                    ) : (
                      <Eye size={12} aria-hidden="true" />
                    )}
                  </button>
                )}
              </div>
              {isOpen && (
                <div className="border-x border-b border-steel/20 px-2 py-1.5">
                  <div className="mb-1 flex items-center gap-2 font-mono text-[10px] text-steel">
                    <span>{step.shell}</span>
                    {info.startedAt && (
                      <span>
                        started {new Date(info.startedAt).toLocaleTimeString()}
                        {info.durationMs !== undefined &&
                          ` · ${formatMs(info.durationMs, info.exact)}`}
                        {info.exitCode != null && ` · exit ${info.exitCode}`}
                      </span>
                    )}
                  </div>
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
