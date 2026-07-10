import { useMemo } from 'react';
import { AlertTriangle } from 'lucide-react';
import { cn } from '../../../lib/cn';
import type { PipelineJob } from '../../../types/pipeline';
import { statusLabel } from './PipelineStatusBadge';

interface PipelineGraphProps {
  trigger: string;
  jobs: PipelineJob[];
  selected?: string | null;
  onSelect?: (jobId: string) => void;
}

const NODE_W = 168;
const NODE_H = 52;
const GAP_X = 56;
const GAP_Y = 20;
const PAD = 16;

interface Positioned {
  job: PipelineJob;
  x: number;
  y: number;
}

/**
 * Same layered Kahn layout as the workflow graph, driven by the pipeline's
 * `needs` snapshot; node styling reflects LIVE execution state.
 */
function layoutJobs(jobs: PipelineJob[]): Positioned[] | null {
  const byKey = new Map(jobs.map((job) => [job.key, job]));
  const layer = new Map<string, number>();

  let remaining = jobs.slice();
  let guard = 0;
  while (remaining.length > 0) {
    if (guard++ > jobs.length + 1) return null;
    const next: PipelineJob[] = [];
    let progressed = false;
    for (const job of remaining) {
      const deps = job.needs.filter((need) => byKey.has(need));
      if (deps.every((need) => layer.has(need))) {
        layer.set(job.key, deps.length === 0 ? 0 : Math.max(...deps.map((d) => layer.get(d)!)) + 1);
        progressed = true;
      } else {
        next.push(job);
      }
    }
    if (!progressed) return null;
    remaining = next;
  }

  const byLayer = new Map<number, PipelineJob[]>();
  for (const job of jobs) {
    const l = layer.get(job.key)!;
    byLayer.set(l, [...(byLayer.get(l) ?? []), job]);
  }
  const layerCount = byLayer.size;
  const maxPerLayer = Math.max(...Array.from(byLayer.values(), (l) => l.length), 1);
  const totalHeight = maxPerLayer * NODE_H + (maxPerLayer - 1) * GAP_Y;

  const positioned: Positioned[] = [];
  for (let l = 0; l < layerCount; l += 1) {
    const column = byLayer.get(l) ?? [];
    const columnHeight = column.length * NODE_H + (column.length - 1) * GAP_Y;
    const offsetY = (totalHeight - columnHeight) / 2;
    column.forEach((job, index) => {
      positioned.push({
        job,
        x: PAD + (l + 1) * (NODE_W + GAP_X),
        y: PAD + offsetY + index * (NODE_H + GAP_Y),
      });
    });
  }
  return positioned;
}

function edgePath(fromX: number, fromY: number, toX: number, toY: number): string {
  const mid = (fromX + toX) / 2;
  return `M ${fromX} ${fromY} C ${mid} ${fromY}, ${mid} ${toY}, ${toX} ${toY}`;
}

/** Live state -> solid palette styling (strict design rules, 6px corners). */
function nodeClasses(job: PipelineJob): { rect: string; title: string; sub: string } {
  if (job.status === 'in_progress') {
    return { rect: 'fill-link/10 stroke-link', title: 'fill-link', sub: 'fill-steel' };
  }
  if (job.status === 'queued') {
    return { rect: 'fill-canvas stroke-steel/40', title: 'fill-charcoal', sub: 'fill-steel' };
  }
  switch (job.conclusion) {
    case 'success':
      return { rect: 'fill-primary/10 stroke-primary', title: 'fill-primary', sub: 'fill-steel' };
    case 'failure':
    case 'timed_out':
      return { rect: 'fill-danger/10 stroke-danger', title: 'fill-danger', sub: 'fill-steel' };
    case 'skipped':
      return { rect: 'fill-surface stroke-steel/30', title: 'fill-steel', sub: 'fill-steel/70' };
    default: // cancelled
      return { rect: 'fill-surface stroke-charcoal/40', title: 'fill-charcoal', sub: 'fill-steel' };
  }
}

export function PipelineGraph({ trigger, jobs, selected, onSelect }: PipelineGraphProps) {
  const nodes = useMemo(() => layoutJobs(jobs), [jobs]);

  if (jobs.length === 0) {
    return <p className="py-8 text-center text-sm text-steel">No jobs to visualize.</p>;
  }
  if (!nodes) {
    return (
      <div className="flex items-center gap-2 rounded border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger">
        <AlertTriangle size={14} aria-hidden="true" />
        The dependency graph contains a cycle and cannot be laid out.
      </div>
    );
  }

  const positions = new Map(nodes.map((node) => [node.job.key, node]));
  const maxX = Math.max(...nodes.map((n) => n.x)) + NODE_W + PAD;
  const jobsMaxY = Math.max(...nodes.map((n) => n.y)) + NODE_H + PAD;

  const triggerH = 36;
  const triggerY = Math.max((jobsMaxY - PAD - triggerH) / 2, PAD);
  const height = Math.max(jobsMaxY, triggerY + triggerH + PAD);
  const rootJobs = nodes.filter((node) => node.job.needs.every((need) => !positions.has(need)));

  return (
    <div className="overflow-auto rounded border border-steel/20 bg-surface/50 p-2">
      <svg
        width={maxX}
        height={height}
        role="img"
        aria-label={`Execution graph with ${jobs.length} jobs`}
        className="min-w-full"
      >
        {/* trigger -> root job edges */}
        {rootJobs.map((node) => (
          <path
            key={`t-${node.job.key}`}
            d={edgePath(PAD + 120, triggerY + triggerH / 2, node.x, node.y + NODE_H / 2)}
            className="fill-none stroke-steel/30"
            strokeWidth={1.5}
          />
        ))}
        {/* needs edges — an edge lights up when its downstream job runs */}
        {nodes.map((node) =>
          node.job.needs
            .filter((need) => positions.has(need))
            .map((need) => {
              const from = positions.get(need)!;
              const active =
                node.job.status === 'in_progress' ||
                selected === node.job.key ||
                selected === need;
              return (
                <path
                  key={`${need}->${node.job.key}`}
                  d={edgePath(from.x + NODE_W, from.y + NODE_H / 2, node.x, node.y + NODE_H / 2)}
                  className={cn('fill-none', active ? 'stroke-primary' : 'stroke-steel/30')}
                  strokeWidth={active ? 2 : 1.5}
                />
              );
            }),
        )}

        {/* trigger pseudo-node */}
        <g>
          <rect
            x={PAD}
            y={triggerY}
            width={120}
            height={triggerH}
            rx={6}
            className="fill-primary/10 stroke-primary/30"
          />
          <text
            x={PAD + 60}
            y={triggerY + triggerH / 2 + 4}
            textAnchor="middle"
            className="fill-primary font-mono text-[11px]"
          >
            {trigger}
          </text>
        </g>

        {/* job nodes */}
        {nodes.map(({ job, x, y }) => {
          const isSelected = selected === job.key;
          const classes = nodeClasses(job);
          const label = job.name ?? job.key;
          const sub = statusLabel(job.status, job.conclusion);
          return (
            <g
              key={job.key}
              onClick={() => onSelect?.(job.key)}
              className={cn(
                onSelect && 'cursor-pointer',
                job.status === 'in_progress' && 'animate-pulse',
              )}
              role={onSelect ? 'button' : undefined}
              aria-label={`Job ${job.key}: ${sub}`}
            >
              <rect
                x={x}
                y={y}
                width={NODE_W}
                height={NODE_H}
                rx={6}
                strokeWidth={isSelected ? 2.5 : 1.5}
                className={cn('transition-colors', classes.rect)}
              />
              <text x={x + 12} y={y + 21} className={cn('text-xs font-medium', classes.title)}>
                {label.length > 20 ? `${label.slice(0, 19)}…` : label}
              </text>
              <text x={x + 12} y={y + 38} className={cn('text-[10px]', classes.sub)}>
                {job.status === 'in_progress' ? job.stage.replaceAll('_', ' ') : sub}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
