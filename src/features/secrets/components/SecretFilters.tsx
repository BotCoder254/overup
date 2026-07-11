import { Search } from 'lucide-react';
import { useRepositories } from '../../repositories/hooks/useRepositories';
import { useEnvironmentsCatalog } from '../../environments/hooks/useEnvironments';

/** UI-level filter state for the secrets catalog. */
export interface SecretFilterState {
  q: string;
  scope: string;
  repositoryId: string;
  environmentId: string;
}

export const EMPTY_SECRET_FILTERS: SecretFilterState = {
  q: '',
  scope: '',
  repositoryId: '',
  environmentId: '',
};

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface SecretFiltersProps {
  value: SecretFilterState;
  onChange: (patch: Partial<SecretFilterState>) => void;
}

/**
 * Server-side catalog filters: name/description search, scope, and
 * repository. Controls go full-width on phones and wrap into a row from
 * `sm` up (the ledger filter-bar pattern).
 */
export function SecretFilters({ value, onChange }: SecretFiltersProps) {
  const repositories = useRepositories();
  const environments = useEnvironmentsCatalog();
  const environmentOptions = (environments.data?.pages ?? []).flatMap(
    (page) => page.environments,
  );

  return (
    <div className="mb-4 flex flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="secret-search">
        Search secrets
      </label>
      <div className="relative w-full sm:w-64">
        <Search
          size={14}
          aria-hidden="true"
          className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
        />
        <input
          id="secret-search"
          type="search"
          placeholder="Search name or description…"
          maxLength={200}
          className={`${controlClasses} w-full pl-8`}
          value={value.q}
          onChange={(event) => onChange({ q: event.target.value })}
        />
      </div>

      <label className="sr-only" htmlFor="secret-scope-filter">
        Filter by scope
      </label>
      <select
        id="secret-scope-filter"
        className={`${controlClasses} w-full sm:w-auto`}
        value={value.scope}
        onChange={(event) =>
          onChange({
            scope: event.target.value,
            // A stale target filter from another scope would silently
            // exclude everything — clear the one that no longer applies.
            ...(event.target.value !== 'repository' ? { repositoryId: '' } : {}),
            ...(event.target.value !== 'environment' ? { environmentId: '' } : {}),
          })
        }
      >
        <option value="">All scopes</option>
        <option value="workspace">Workspace</option>
        <option value="repository">Repository</option>
        <option value="environment">Environment</option>
      </select>

      <label className="sr-only" htmlFor="secret-repo-filter">
        Filter by repository
      </label>
      <select
        id="secret-repo-filter"
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

      {value.scope === 'environment' && (
        <>
          <label className="sr-only" htmlFor="secret-environment-filter">
            Filter by environment
          </label>
          <select
            id="secret-environment-filter"
            className={`${controlClasses} w-full sm:w-auto`}
            value={value.environmentId}
            onChange={(event) => onChange({ environmentId: event.target.value })}
          >
            <option value="">All environments</option>
            {environmentOptions.map((environment) => (
              <option key={environment.id} value={environment.id}>
                {environment.name}
              </option>
            ))}
          </select>
        </>
      )}
    </div>
  );
}
