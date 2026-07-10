import { ArrowRight, Server } from 'lucide-react';
import { Badge } from '../../../components/ui/Badge';
import { cn } from '../../../lib/cn';
import type { GraphJob } from './WorkflowGraph';

interface JobsPanelProps {
  jobs: GraphJob[];
  selected?: string | null;
  onSelect?: (jobKey: string) => void;
}

/** Flat job list — mirrors graph selection. */
export function JobsPanel({ jobs, selected, onSelect }: JobsPanelProps) {
  if (jobs.length === 0) {
    return <p className="py-8 text-center text-sm text-steel">This workflow defines no jobs.</p>;
  }
  return (
    <ul className="divide-y divide-steel/10" aria-label="Jobs">
      {jobs.map((job) => (
        <li key={job.key}>
          <button
            type="button"
            onClick={() => onSelect?.(job.key)}
            className={cn(
              'w-full px-3 py-2.5 text-left transition-colors hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
              selected === job.key && 'bg-primary/10',
            )}
          >
            <div className="flex items-center gap-2">
              <span
                className={cn(
                  'truncate text-sm font-medium',
                  selected === job.key ? 'text-primary' : 'text-charcoal',
                )}
              >
                {job.name ?? job.key}
              </span>
              {job.uses ? (
                <Badge variant="info">reusable</Badge>
              ) : (
                <span className="ml-auto shrink-0 text-xs text-steel">{job.stepCount} steps</span>
              )}
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-steel">
              <span className="font-mono">{job.key}</span>
              {job.runsOn.length > 0 && (
                <span className="inline-flex items-center gap-1">
                  <Server size={11} aria-hidden="true" />
                  {job.runsOn.join(', ')}
                </span>
              )}
              {job.needs.length > 0 && (
                <span className="inline-flex items-center gap-1">
                  <ArrowRight size={11} aria-hidden="true" />
                  needs {job.needs.join(', ')}
                </span>
              )}
            </div>
            {job.uses && (
              <p className="mt-1 truncate font-mono text-xs text-steel">{job.uses}</p>
            )}
          </button>
        </li>
      ))}
    </ul>
  );
}
