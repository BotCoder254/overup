import { Search } from 'lucide-react';
import { useRepositories } from '../../repositories/hooks/useRepositories';
import { useWorkflows } from '../../workflows/hooks/useWorkflows';

/** UI-level filter state for the artifact catalog. */
export interface ArtifactFilterState {
  q: string;
  status: string;
  repositoryId: string;
  workflowId: string;
  /** yyyy-mm-dd (native date inputs); converted to RFC3339 by the page. */
  from: string;
  to: string;
}

export const EMPTY_ARTIFACT_FILTERS: ArtifactFilterState = {
  q: '',
  status: '',
  repositoryId: '',
  workflowId: '',
  from: '',
  to: '',
};

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface ArtifactFiltersProps {
  value: ArtifactFilterState;
  onChange: (patch: Partial<ArtifactFilterState>) => void;
}

/**
 * Server-side catalog filters: name search, availability, repository,
 * workflow, and a created-at date range. Controls go full-width on phones
 * and wrap into a row from `sm` up (the ledger filter-bar pattern).
 */
export function ArtifactFilters({ value, onChange }: ArtifactFiltersProps) {
  const repositories = useRepositories();
  const workflows = useWorkflows();

  return (
    <div className="mb-4 flex flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="artifact-search">
        Search artifacts
      </label>
      <div className="relative w-full sm:w-64">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          id="artifact-search"
          type="search"
          placeholder="Search artifact name…"
          maxLength={200}
          className={`${controlClasses} w-full pl-8`}
          value={value.q}
          onChange={(event) => onChange({ q: event.target.value })}
        />
      </div>

      <label className="sr-only" htmlFor="artifact-status-filter">
        Filter by status
      </label>
      <select
        id="artifact-status-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.status}
        onChange={(event) => onChange({ status: event.target.value })}
      >
        <option value="">All statuses</option>
        <option value="uploaded">Available</option>
        <option value="pending">Uploading</option>
        <option value="failed">Failed</option>
        <option value="expired">Expired</option>
      </select>

      <label className="sr-only" htmlFor="artifact-repo-filter">
        Filter by repository
      </label>
      <select
        id="artifact-repo-filter"
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

      <label className="sr-only" htmlFor="artifact-workflow-filter">
        Filter by workflow
      </label>
      <select
        id="artifact-workflow-filter"
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

      <label className="sr-only" htmlFor="artifact-from-filter">
        Created after
      </label>
      <input
        id="artifact-from-filter"
        type="date"
        aria-label="Created after"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.from}
        onChange={(event) => onChange({ from: event.target.value })}
      />
      <span className="hidden text-xs text-steel sm:inline">to</span>
      <label className="sr-only" htmlFor="artifact-to-filter">
        Created before
      </label>
      <input
        id="artifact-to-filter"
        type="date"
        aria-label="Created before"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.to}
        onChange={(event) => onChange({ to: event.target.value })}
      />
    </div>
  );
}
