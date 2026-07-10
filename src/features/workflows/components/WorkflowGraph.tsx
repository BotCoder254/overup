import { useMemo } from 'react';
import { AlertTriangle } from 'lucide-react';
import { cn } from '../../../lib/cn';

export interface GraphJob {
  key: string;
  name: string | null;
  needs: string[];
  uses: string | null;
  runsOn: string[];
  stepCount: number;
}

interface WorkflowGraphProps {
  triggers: string[];
  jobs: GraphJob[];
  selected?: string | null;
  onSelect?: (jobKey: string) => void;
}

const NODE_W = 168;
const NODE_H = 52;
const GAP_X = 56;
const GAP_Y = 20;
const PAD = 16;

interface Positioned {
  job: GraphJob;
  x: number;
  y: number;
}

/**
 * Layered DAG layout (Kahn): a job's layer is one past its deepest
 * dependency. Returns null when the `needs` graph has a cycle — the caller
 * shows a flat fallback instead of attempting a layout.
 */
function layoutJobs(jobs: GraphJob[]): Positioned[] | null {
  const byKey = new Map(jobs.map((job) => [job.key, job]));
  const layer = new Map<string, number>();

  let remaining = jobs.slice();
  let guard = 0;
  while (remaining.length > 0) {
    if (guard++ > jobs.length + 1) return null; // cycle: no progress possible
    const next: GraphJob[] = [];
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

  const byLayer = new Map<number, GraphJob[]>();
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
        // Column 0 is reserved for trigger pseudo-nodes.
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

/**
 * Dependency graph: triggers -> root jobs -> `needs` edges. Pure SVG in the
 * shell palette — solid colors, 6px corners, no external graph library.
 * Clicking a job notifies the parent so the editor can highlight its YAML.
 */
export function WorkflowGraph({ triggers, jobs, selected, onSelect }: WorkflowGraphProps) {
  const nodes = useMemo(() => layoutJobs(jobs), [jobs]);

  if (jobs.length === 0) {
    return <p className="py-8 text-center text-sm text-steel">No jobs to visualize.</p>;
  }

  if (!nodes) {
    return (
      <div className="flex items-center gap-2 rounded border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger">
        <AlertTriangle size={14} aria-hidden="true" />
        The `needs` graph contains a circular dependency, so it cannot be laid out. Fix the cycle
        to see the visualization.
      </div>
    );
  }

  const positions = new Map(nodes.map((node) => [node.job.key, node]));
  const maxX = Math.max(...nodes.map((n) => n.x)) + NODE_W + PAD;
  const jobsMaxY = Math.max(...nodes.map((n) => n.y)) + NODE_H + PAD;

  // Trigger pseudo-nodes in column 0, vertically centered.
  const triggerH = 36;
  const triggersHeight = triggers.length * triggerH + (triggers.length - 1) * GAP_Y;
  const triggerOffset = Math.max((jobsMaxY - PAD - triggersHeight) / 2, PAD);
  const triggerNodes = triggers.map((trigger, index) => ({
    trigger,
    x: PAD,
    y: triggerOffset + index * (triggerH + GAP_Y),
  }));
  const height = Math.max(
    jobsMaxY,
    triggerNodes.length > 0 ? triggerNodes[triggerNodes.length - 1].y + triggerH + PAD : 0,
  );

  const rootJobs = nodes.filter((node) =>
    node.job.needs.every((need) => !positions.has(need)),
  );

  return (
    <div className="overflow-auto rounded border border-steel/20 bg-surface/50 p-2">
      <svg
        width={maxX}
        height={height}
        role="img"
        aria-label={`Dependency graph with ${jobs.length} jobs`}
        className="min-w-full"
      >
        {/* trigger -> root job edges */}
        {triggerNodes.map((t) =>
          rootJobs.map((node) => (
            <path
              key={`t-${t.trigger}-${node.job.key}`}
              d={edgePath(t.x + 120, t.y + triggerH / 2, node.x, node.y + NODE_H / 2)}
              className="fill-none stroke-steel/30"
              strokeWidth={1.5}
            />
          )),
        )}
        {/* needs edges */}
        {nodes.map((node) =>
          node.job.needs
            .filter((need) => positions.has(need))
            .map((need) => {
              const from = positions.get(need)!;
              const active = selected === node.job.key || selected === need;
              return (
                <path
                  key={`${need}->${node.job.key}`}
                  d={edgePath(
                    from.x + NODE_W,
                    from.y + NODE_H / 2,
                    node.x,
                    node.y + NODE_H / 2,
                  )}
                  className={cn('fill-none', active ? 'stroke-primary' : 'stroke-steel/30')}
                  strokeWidth={active ? 2 : 1.5}
                />
              );
            }),
        )}

        {/* trigger pseudo-nodes */}
        {triggerNodes.map((t) => (
          <g key={t.trigger}>
            <rect
              x={t.x}
              y={t.y}
              width={120}
              height={triggerH}
              rx={6}
              className="fill-primary/10 stroke-primary/30"
            />
            <text
              x={t.x + 60}
              y={t.y + triggerH / 2 + 4}
              textAnchor="middle"
              className="fill-primary font-mono text-[11px]"
            >
              {t.trigger.length > 16 ? `${t.trigger.slice(0, 15)}…` : t.trigger}
            </text>
          </g>
        ))}

        {/* job nodes */}
        {nodes.map(({ job, x, y }) => {
          const isSelected = selected === job.key;
          const label = job.name ?? job.key;
          const sub = job.uses
            ? 'reusable workflow'
            : job.runsOn.length > 0
              ? job.runsOn.join(', ')
              : `${job.stepCount} steps`;
          return (
            <g
              key={job.key}
              onClick={() => onSelect?.(job.key)}
              className={onSelect ? 'cursor-pointer' : undefined}
              role={onSelect ? 'button' : undefined}
              aria-label={`Job ${job.key}`}
            >
              <rect
                x={x}
                y={y}
                width={NODE_W}
                height={NODE_H}
                rx={6}
                strokeWidth={isSelected ? 2 : 1}
                className={cn(
                  'fill-canvas transition-colors',
                  isSelected ? 'stroke-primary' : 'stroke-steel/40',
                )}
              />
              <text
                x={x + 12}
                y={y + 21}
                className={cn(
                  'text-xs font-medium',
                  isSelected ? 'fill-primary' : 'fill-charcoal',
                )}
              >
                {label.length > 20 ? `${label.slice(0, 19)}…` : label}
              </text>
              <text x={x + 12} y={y + 38} className="fill-steel text-[10px]">
                {sub.length > 24 ? `${sub.slice(0, 23)}…` : sub}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}
