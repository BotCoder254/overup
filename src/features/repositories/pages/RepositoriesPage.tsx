import { RefreshCw } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card } from '../../../components/ui/Card';
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

  const importable = available.data?.filter((repo) => !repo.connected) ?? [];
  const connected = repositories.data ?? [];
  const loading = installations.isLoading || repositories.isLoading;

  return (
    <>
      <PageHeader
        title="Repositories"
        description="Connect GitHub repositories to the workspace and keep their branches, workflows, and webhooks in sync."
        actions={
          <>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                void repositories.refetch();
                void available.refetch();
              }}
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
                No repositories connected yet — import one from the list below.
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
                Every repository this installation can see is already connected. Grant the app
                access to more repositories on GitHub to see them here.
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
