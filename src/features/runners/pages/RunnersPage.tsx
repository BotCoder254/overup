import { AlertTriangle, Plus, Server } from 'lucide-react';
import { useState } from 'react';
import { useParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import type { Runner } from '../../../types/runner';
import { useWorkspaceStream } from '../../dashboard/hooks/useWorkspaceStream';
import { RegenerateTokenTrigger } from '../components/RegenerateTokenTrigger';
import { RenameRunnerDialog } from '../components/RenameRunnerDialog';
import { RunnerRegistrationDialog } from '../components/RunnerRegistrationDialog';
import { RunnersTable } from '../components/RunnersTable';
import { useRevokeRunner, useRunners } from '../hooks/useRunners';

export function RunnersPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const { connected } = useWorkspaceStream();
  const runners = useRunners(connected);
  const revokeRunner = useRevokeRunner();

  const [createOpen, setCreateOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<Runner | null>(null);
  const [regenTarget, setRegenTarget] = useState<Runner | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<Runner | null>(null);

  const list = runners.data ?? [];

  return (
    <>
      <PageHeader
        title="Runners"
        description="Register self-hosted runners, watch their health, and control which pipelines they pick up."
        actions={
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus size={14} aria-hidden="true" />
            Register runner
          </Button>
        }
      />

      {runners.isLoading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : runners.isError && !runners.data ? (
        <EmptyState
          icon={AlertTriangle}
          title="Couldn't load runners"
          description="Something went wrong fetching this workspace's runners. Check your connection and try again."
          action={
            <Button size="sm" onClick={() => void runners.refetch()}>
              Try again
            </Button>
          }
        />
      ) : list.length === 0 ? (
        <EmptyState
          icon={Server}
          title="No runners yet"
          description="Register a self-hosted runner to start executing pipeline jobs in this workspace."
          action={
            <Button size="sm" onClick={() => setCreateOpen(true)}>
              <Plus size={14} aria-hidden="true" />
              Register runner
            </Button>
          }
        />
      ) : (
        <RunnersTable
          slug={slug}
          runners={list}
          onRename={setRenameTarget}
          onRegenerateToken={setRegenTarget}
          onRevoke={setRevokeTarget}
        />
      )}

      <RunnerRegistrationDialog open={createOpen} onClose={() => setCreateOpen(false)} />
      <RenameRunnerDialog runner={renameTarget} onClose={() => setRenameTarget(null)} />
      <RegenerateTokenTrigger runner={regenTarget} onClose={() => setRegenTarget(null)} />

      <Dialog
        open={revokeTarget !== null}
        onClose={() => setRevokeTarget(null)}
        title={revokeTarget ? `Revoke ${revokeTarget.name}?` : ''}
        description="The runner is permanently disconnected and can never authenticate again. Any in-flight job is recovered and requeued. This cannot be undone."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setRevokeTarget(null)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={revokeRunner.isPending}
              onClick={() =>
                revokeTarget &&
                revokeRunner.mutate(revokeTarget.id, { onSuccess: () => setRevokeTarget(null) })
              }
            >
              Revoke runner
            </Button>
          </>
        }
      />
    </>
  );
}
