import { format, formatDistanceToNow } from 'date-fns';
import { FileCode2, GitBranch, Lock, RefreshCw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import { Tabs } from '../../../components/ui/Tabs';
import { workspacePath } from '../../../app/navigation';
import { ValidationBadge } from '../../workflows/components/ValidationBadge';
import { RepositoryEventsList } from '../components/RepositoryEventsList';
import { RepositorySyncPanel } from '../components/RepositorySyncPanel';
import { SyncHistoryList } from '../components/SyncHistoryList';
import { SyncStatusBadge } from '../components/SyncStatusBadge';
import {
  useRemoveRepository,
  useRepositoryDetail,
  useSyncRepository,
} from '../hooks/useRepositories';

type TabId = 'workflows' | 'branches' | 'events' | 'history';

export function RepositoryDetailPage() {
  const { slug = '', repoId } = useParams<{ slug: string; repoId: string }>();
  const navigate = useNavigate();
  const detail = useRepositoryDetail(repoId);
  const syncRepo = useSyncRepository();
  const removeRepo = useRemoveRepository();
  const [tab, setTab] = useState<TabId>('workflows');
  const [confirmRemove, setConfirmRemove] = useState(false);

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }
  if (detail.isError || !detail.data) {
    return (
      <>
        <PageHeader
          title="Repository"
          parent={{ label: 'Repositories', to: workspacePath(slug, 'repositories') }}
        />
        <p className="text-sm text-steel">
          This repository could not be loaded — it may have been removed.
        </p>
      </>
    );
  }

  const { repository, branches, workflows, syncRuns, health } = detail.data;
  const syncing = repository.syncStatus === 'syncing' || repository.syncStatus === 'pending';

  return (
    <>
      <PageHeader
        title={repository.fullName}
        parent={{ label: 'Repositories', to: workspacePath(slug, 'repositories') }}
        description={repository.description ?? undefined}
        actions={
          <>
            <Button
              size="sm"
              variant="secondary"
              isLoading={syncRepo.isPending || syncing}
              onClick={() => repoId && syncRepo.mutate(repoId)}
            >
              <RefreshCw size={14} aria-hidden="true" />
              {syncing ? 'Syncing…' : 'Re-sync'}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="text-danger hover:bg-danger/10 hover:text-danger"
              onClick={() => setConfirmRemove(true)}
            >
              <Trash2 size={14} aria-hidden="true" />
              Remove
            </Button>
          </>
        }
      />

      <div className="mb-6 flex flex-wrap items-center gap-2 text-xs text-steel">
        <span className="inline-flex items-center gap-1.5">
          <Avatar size="sm" login={repository.owner} avatarUrl={repository.ownerAvatarUrl} />
          <span className="font-medium text-charcoal">{repository.owner}</span>
        </span>
        <SyncStatusBadge status={repository.syncStatus} />
        {repository.private && (
          <Badge variant="outline">
            <Lock size={10} aria-hidden="true" />
            Private
          </Badge>
        )}
        <Badge variant="outline">
          <GitBranch size={10} aria-hidden="true" />
          {repository.defaultBranch}
        </Badge>
        {repository.language && <Badge variant="neutral">{repository.language}</Badge>}
        {repository.lastSyncedAt && (
          <span>
            Last synced{' '}
            {formatDistanceToNow(new Date(repository.lastSyncedAt), { addSuffix: true })}
          </span>
        )}
        {repository.syncStatus === 'failed' && repository.syncError && (
          <span className="text-danger">({repository.syncError})</span>
        )}
      </div>

      <RepositorySyncPanel repository={repository} health={health} />

      <Tabs
        ariaLabel="Repository sections"
        active={tab}
        onChange={(id) => setTab(id as TabId)}
        className="mb-4"
        tabs={[
          { id: 'workflows', label: `Workflows (${workflows.length})` },
          { id: 'branches', label: `Branches (${branches.length})` },
          { id: 'events', label: 'Events' },
          { id: 'history', label: 'Sync history' },
        ]}
      />

      {tab === 'workflows' &&
        (workflows.length === 0 ? (
          <Card className="p-8 text-center text-sm text-steel">
            No workflow files detected in <code className="font-mono">.github/workflows</code>.
          </Card>
        ) : (
          <Table>
            <THead>
              <Tr>
                <Th>Workflow</Th>
                <Th className="hidden sm:table-cell">Triggers</Th>
                <Th className="hidden sm:table-cell">Jobs</Th>
                <Th>Validation</Th>
              </Tr>
            </THead>
            <TBody>
              {workflows.map((workflow) => (
                <Tr key={workflow.id} className="hover:bg-surface">
                  <Td>
                    <Link
                      to={workspacePath(slug, `workflows/${workflow.id}`)}
                      className="font-medium text-charcoal hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <FileCode2 size={14} className="text-steel" aria-hidden="true" />
                        {workflow.name}
                      </span>
                    </Link>
                    <p className="mt-0.5 font-mono text-xs text-steel">{workflow.path}</p>
                    {/* Phones hide the Triggers/Jobs columns; fold a compact line in. */}
                    <p className="mt-0.5 text-xs text-steel sm:hidden">
                      {workflow.triggers.join(', ')} · {workflow.jobCount} job
                      {workflow.jobCount === 1 ? '' : 's'}
                    </p>
                  </Td>
                  <Td className="hidden sm:table-cell">
                    <div className="flex flex-wrap gap-1">
                      {workflow.triggers.map((trigger) => (
                        <Badge key={trigger} variant="info">
                          {trigger}
                        </Badge>
                      ))}
                    </div>
                  </Td>
                  <Td className="hidden text-steel sm:table-cell">{workflow.jobCount}</Td>
                  <Td>
                    <ValidationBadge status={workflow.validationStatus} />
                  </Td>
                </Tr>
              ))}
            </TBody>
          </Table>
        ))}

      {tab === 'branches' &&
        (branches.length === 0 ? (
          <Card className="p-8 text-center text-sm text-steel">No branches synced yet.</Card>
        ) : (
          <Table>
            <THead>
              <Tr>
                <Th>Branch</Th>
                <Th>Commit</Th>
                <Th className="hidden sm:table-cell">Updated</Th>
              </Tr>
            </THead>
            <TBody>
              {branches.map((branch) => (
                <Tr key={branch.name}>
                  <Td>
                    <span className="inline-flex items-center gap-1.5">
                      <GitBranch size={14} className="text-steel" aria-hidden="true" />
                      <span className="font-medium">{branch.name}</span>
                      {branch.isDefault && <Badge variant="primary">default</Badge>}
                    </span>
                    <p className="mt-0.5 text-xs text-steel sm:hidden">
                      {formatDistanceToNow(new Date(branch.updatedAt), { addSuffix: true })}
                    </p>
                  </Td>
                  <Td className="font-mono text-xs text-steel">{branch.commitSha.slice(0, 7)}</Td>
                  <Td className="hidden text-steel sm:table-cell">
                    <span title={format(new Date(branch.updatedAt), 'PPpp')}>
                      {formatDistanceToNow(new Date(branch.updatedAt), { addSuffix: true })}
                    </span>
                  </Td>
                </Tr>
              ))}
            </TBody>
          </Table>
        ))}

      {tab === 'events' && repoId && (
        <RepositoryEventsList repositoryId={repoId} slug={slug} health={health} />
      )}

      {tab === 'history' && <SyncHistoryList runs={syncRuns} />}

      <Dialog
        open={confirmRemove}
        onClose={() => setConfirmRemove(false)}
        title={`Remove ${repository.fullName}?`}
        description="The repository, its synced workflows, branches, and sync history are removed from this workspace. Nothing changes on GitHub — you can import it again at any time."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmRemove(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={removeRepo.isPending}
              onClick={() =>
                repoId &&
                removeRepo.mutate(repoId, {
                  onSuccess: () => navigate(workspacePath(slug, 'repositories')),
                })
              }
            >
              Remove repository
            </Button>
          </>
        }
      />
    </>
  );
}
