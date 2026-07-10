import { Search } from 'lucide-react';
import type { Runner } from '../../../types/runner';
import { useRepositories } from '../../repositories/hooks/useRepositories';
import { useWorkflows } from '../../workflows/hooks/useWorkflows';

/** UI-level filter state for the job queue; mirrored into the URL. */
export interface QueueFilterState {
  status: string;
  repositoryId: string;
  workflowId: string;
  runnerId: string;
  label: string;
  q: string;
}

export const EMPTY_QUEUE_FILTERS: QueueFilterState = {
  status: '',
  repositoryId: '',
  workflowId: '',
  runnerId: '',
  label: '',
  q: '',
};

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface QueueFiltersProps {
  value: QueueFilterState;
  onChange: (patch: Partial<QueueFilterState>) => void;
  runners: Runner[];
}

/**
 * Server-side queue filters: free-text search, active status, repository,
 * workflow, runner, and a runs-on label. Controls go full-width on phones
 * and wrap into a row from `sm` up — same treatment as the pipeline ledger.
 */
export function QueueFilters({ value, onChange, runners }: QueueFiltersProps) {
  const repositories = useRepositories();
  const workflows = useWorkflows();

  return (
    <div className="mb-4 flex flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="queue-search">
        Search queued jobs
      </label>
      <div className="relative w-full sm:w-64">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          id="queue-search"
          type="search"
          placeholder="Search job, workflow…"
          maxLength={200}
          className={`${controlClasses} w-full pl-8`}
          value={value.q}
          onChange={(event) => onChange({ q: event.target.value })}
        />
      </div>

      <label className="sr-only" htmlFor="queue-status-filter">
        Filter by status
      </label>
      <select
        id="queue-status-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.status}
        onChange={(event) => onChange({ status: event.target.value })}
      >
        <option value="">All active</option>
        <option value="queued">Queued</option>
        <option value="in_progress">Running</option>
      </select>

      <label className="sr-only" htmlFor="queue-repo-filter">
        Filter by repository
      </label>
      <select
        id="queue-repo-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.repositoryId}
        onChange={(event) => onChange({ repositoryId: event.target.value })}
      >
        <option value="">All repositories</option>
        {(repositories.data ?? []).map((repo) => (
          <option key={repo.id} value={repo.id}>
            {repo.fullName}
          </option>
        ))}
      </select>

      <label className="sr-only" htmlFor="queue-workflow-filter">
        Filter by workflow
      </label>
      <select
        id="queue-workflow-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.workflowId}
        onChange={(event) => onChange({ workflowId: event.target.value })}
      >
        <option value="">All workflows</option>
        {(workflows.data ?? []).map((workflow) => (
          <option key={workflow.id} value={workflow.id}>
            {workflow.name}
          </option>
        ))}
      </select>

      <label className="sr-only" htmlFor="queue-runner-filter">
        Filter by runner
      </label>
      <select
        id="queue-runner-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.runnerId}
        onChange={(event) => onChange({ runnerId: event.target.value })}
      >
        <option value="">All runners</option>
        {runners.filter((runner) => !runner.revoked).map((runner) => (
          <option key={runner.id} value={runner.id}>
            {runner.name}
          </option>
        ))}
      </select>

      <label className="sr-only" htmlFor="queue-label-filter">
        Filter by label
      </label>
      <input
        id="queue-label-filter"
        type="text"
        placeholder="Label (e.g. ubuntu-latest)"
        maxLength={128}
        className={`${controlClasses} w-full sm:w-44`}
        value={value.label}
        onChange={(event) => onChange({ label: event.target.value })}
      />
    </div>
  );
}
