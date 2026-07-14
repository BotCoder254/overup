import { RefreshCw, Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card } from '../../../components/ui/Card';
import { Input } from '../../../components/ui/Input';
import { Spinner } from '../../../components/ui/Spinner';
import type { AvailableRepo } from '../../../types/repository';
import { AvailableRepoRow } from '../components/AvailableRepoRow';
import { InstallAppCallout } from '../components/InstallAppCallout';
import { RepoCard } from '../components/RepoCard';
import {
  useAvailableRepositories,
  useImportRepository,
  useInstallations,
  useRepositories,
} from '../hooks/useRepositories';

export function RepositoriesPage() {
  const installations = useInstallations();
  const repositories = useRepositories();
  const hasInstallation = (installations.data?.installations.length ?? 0) > 0;
  const available = useAvailableRepositories(hasInstallation);
  const importRepo = useImportRepository();
  const [filter, setFilter] = useState('');

  // Landing back from the GitHub App setup redirect.
  const [searchParams, setSearchParams] = useSearchParams();
  const announcedRef = useRef(false);
  useEffect(() => {
    if (announcedRef.current) return;
    if (searchParams.get('installed') === '1') {
      announcedRef.current = true;
      toast.success('GitHub App installation linked to this workspace.');
      setSearchParams({}, { replace: true });
    } else if (searchParams.get('error') === 'install_failed') {
      announcedRef.current = true;
      toast.error('Linking the GitHub App installation failed. Please try again.');
      setSearchParams({}, { replace: true });
    }
  }, [searchParams, setSearchParams]);

  const onImport = (repo: AvailableRepo) => {
    importRepo.mutate({
      installationId: repo.installationId,
      githubRepoId: repo.githubRepoId,
      fullName: repo.fullName,
    });
  };

  const needle = filter.trim().toLowerCase();
  const connected = useMemo(() => {
    const rows = repositories.data ?? [];
    if (!needle) return rows;
    return rows.filter(
      (repository) =>
        repository.fullName.toLowerCase().includes(needle) ||
        (repository.language ?? '').toLowerCase().includes(needle) ||
        (repository.description ?? '').toLowerCase().includes(needle),
    );
  }, [repositories.data, needle]);
  const importable = useMemo(() => {
    const rows = available.data?.filter((repo) => !repo.connected) ?? [];
    if (!needle) return rows;
    return rows.filter((repo) => repo.fullName.toLowerCase().includes(needle));
  }, [available.data, needle]);

  const loading = installations.isLoading || repositories.isLoading;
  const refreshing =
    repositories.isFetching || available.isFetching || installations.isFetching;

  const onRefresh = () => {
    void Promise.all([
      repositories.refetch(),
      installations.refetch(),
      ...(hasInstallation ? [available.refetch()] : []),
    ]).then((results) => {
      if (results.some((result) => result.isError)) {
        toast.error('Could not refresh repositories.');
      } else {
        toast.success('Repositories refreshed.');
      }
    });
  };

  return (
    <>
      <PageHeader
        title="Repositories"
        description="Connect GitHub repositories to the workspace and keep their branches, workflows, and webhooks in sync."
        actions={
          <>
            {hasInstallation && (
              <div className="relative w-full sm:w-auto">
                <Search
                  size={14}
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
                  aria-hidden="true"
                />
                <Input
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                  placeholder="Search repositories…"
                  aria-label="Search repositories"
                  className="h-9 w-full pl-8 text-sm sm:w-56"
                />
              </div>
            )}
            <Button
              size="sm"
              variant="ghost"
              onClick={onRefresh}
              isLoading={refreshing}
              disabled={refreshing}
              aria-label="Refresh repositories"
            >
              <RefreshCw size={14} aria-hidden="true" />
              Refresh
            </Button>
            {installations.data && hasInstallation && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => window.location.assign(installations.data.installUrl)}
              >
                Manage GitHub App
              </Button>
            )}
          </>
        }
      />

      {loading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : !hasInstallation ? (
        <InstallAppCallout installUrl={installations.data?.installUrl} />
      ) : (
        <div className="space-y-8">
          <section aria-labelledby="connected-repos">
            <h2 id="connected-repos" className="mb-3 text-sm font-semibold text-charcoal">
              Connected{' '}
              <span className="font-normal text-steel">({connected.length})</span>
            </h2>
            {connected.length === 0 ? (
              <Card className="p-8 text-center text-sm text-steel">
                {needle
                  ? 'No connected repositories match that search.'
                  : 'No repositories connected yet — import one from the list below.'}
              </Card>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
                {connected.map((repository) => (
                  <RepoCard key={repository.id} repository={repository} />
                ))}
              </div>
            )}
          </section>

          <section aria-labelledby="available-repos">
            <h2 id="available-repos" className="mb-3 text-sm font-semibold text-charcoal">
              Available to import{' '}
              {available.data && (
                <span className="font-normal text-steel">({importable.length})</span>
              )}
            </h2>
            {available.isLoading ? (
              <Card className="flex items-center justify-center gap-2 p-8 text-sm text-steel">
                <Spinner className="h-4 w-4" />
                Loading repositories from GitHub…
              </Card>
            ) : available.isError ? (
              <Card className="p-8 text-center text-sm text-steel">
                Could not load repositories from GitHub. Try refreshing.
              </Card>
            ) : importable.length === 0 ? (
              <Card className="p-8 text-center text-sm text-steel">
                {needle
                  ? 'No importable repositories match that search.'
                  : 'Every repository this installation can see is already connected. Grant the app access to more repositories on GitHub to see them here.'}
              </Card>
            ) : (
              <Card>
                <ul className="divide-y divide-steel/10">
                  {importable.map((repo) => (
                    <AvailableRepoRow
                      key={repo.githubRepoId}
                      repo={repo}
                      onImport={onImport}
                      importing={
                        importRepo.isPending &&
                        importRepo.variables?.githubRepoId === repo.githubRepoId
                      }
                    />
                  ))}
                </ul>
              </Card>
            )}
          </section>
        </div>
      )}
    </>
  );
}
