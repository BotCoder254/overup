import { Search } from 'lucide-react';
import { useRepositories } from '../../repositories/hooks/useRepositories';
import { useWorkflows } from '../../workflows/hooks/useWorkflows';

/**
 * UI-level filter state for the execution ledger. `state` folds status and
 * conclusion into one select: plain statuses pass through; conclusion
 * values imply status=completed server-side.
 */
export interface LedgerFilterState {
  state: string;
  repositoryId: string;
  workflowId: string;
  trigger: string;
  branch: string;
  q: string;
  /** yyyy-mm-dd (native date inputs); converted to RFC3339 by the page. */
  from: string;
  to: string;
}

export const EMPTY_LEDGER_FILTERS: LedgerFilterState = {
  state: '',
  repositoryId: '',
  workflowId: '',
  trigger: '',
  branch: '',
  q: '',
  from: '',
  to: '',
};

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface PipelineFiltersProps {
  value: LedgerFilterState;
  onChange: (patch: Partial<LedgerFilterState>) => void;
}

/**
 * Server-side ledger filters: free-text search, execution state, repository,
 * workflow, trigger, branch, and a created-at date range. Controls go
 * full-width on phones and wrap into a row from `sm` up.
 */
export function PipelineFilters({ value, onChange }: PipelineFiltersProps) {
  const repositories = useRepositories();
  const workflows = useWorkflows();

  return (
    <div className="mb-4 flex flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="pipeline-search">
        Search pipelines
      </label>
      <div className="relative w-full sm:w-64">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          id="pipeline-search"
          type="search"
          placeholder="Search commit, SHA, workflow…"
          maxLength={200}
          className={`${controlClasses} w-full pl-8`}
          value={value.q}
          onChange={(event) => onChange({ q: event.target.value })}
        />
      </div>

      <label className="sr-only" htmlFor="pipeline-state-filter">
        Filter by state
      </label>
      <select
        id="pipeline-state-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.state}
        onChange={(event) => onChange({ state: event.target.value })}
      >
        <option value="">All states</option>
        <option value="queued">Queued</option>
        <option value="in_progress">Running</option>
        <option value="completed">Completed</option>
        <option value="success">Succeeded</option>
        <option value="failure">Failed</option>
        <option value="partial">Partial</option>
        <option value="cancelled">Cancelled</option>
        <option value="timed_out">Timed out</option>
      </select>

      <label className="sr-only" htmlFor="pipeline-repo-filter">
        Filter by repository
      </label>
      <select
        id="pipeline-repo-filter"
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

      <label className="sr-only" htmlFor="pipeline-workflow-filter">
        Filter by workflow
      </label>
      <select
        id="pipeline-workflow-filter"
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

      <label className="sr-only" htmlFor="pipeline-trigger-filter">
        Filter by trigger
      </label>
      <select
        id="pipeline-trigger-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.trigger}
        onChange={(event) => onChange({ trigger: event.target.value })}
      >
        <option value="">All triggers</option>
        <option value="push">Push</option>
        <option value="pull_request">Pull request</option>
        <option value="tag">Tag</option>
        <option value="manual">Manual</option>
      </select>

      <label className="sr-only" htmlFor="pipeline-branch-filter">
        Filter by branch
      </label>
      <input
        id="pipeline-branch-filter"
        type="text"
        placeholder="Branch"
        maxLength={255}
        className={`${controlClasses} w-full sm:w-36`}
        value={value.branch}
        onChange={(event) => onChange({ branch: event.target.value })}
      />

      <label className="sr-only" htmlFor="pipeline-from-filter">
        Created after
      </label>
      <input
        id="pipeline-from-filter"
        type="date"
        aria-label="Created after"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.from}
        onChange={(event) => onChange({ from: event.target.value })}
      />
      <span className="hidden text-xs text-steel sm:inline">to</span>
      <label className="sr-only" htmlFor="pipeline-to-filter">
        Created before
      </label>
      <input
        id="pipeline-to-filter"
        type="date"
        aria-label="Created before"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.to}
        onChange={(event) => onChange({ to: event.target.value })}
      />
    </div>
  );
}
