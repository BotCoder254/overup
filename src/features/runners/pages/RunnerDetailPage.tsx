import { formatDistanceToNow, format } from 'date-fns';
import { KeyRound, Pencil, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import { useWorkspaceStream } from '../../dashboard/hooks/useWorkspaceStream';
import { HealthPanel } from '../components/HealthPanel';
import { LifecycleControls } from '../components/LifecycleControls';
import { RegenerateTokenTrigger } from '../components/RegenerateTokenTrigger';
import { RenameRunnerDialog } from '../components/RenameRunnerDialog';
import { RunnerMetaCard } from '../components/RunnerMetaCard';
import { RunnerStatusBadge } from '../components/RunnerStatusBadge';
import { useRevokeRunner, useRunnerDetail } from '../hooks/useRunners';

export function RunnerDetailPage() {
  const { slug = '', runnerId } = useParams<{ slug: string; runnerId: string }>();
  const navigate = useNavigate();
  const { connected } = useWorkspaceStream();
  const detail = useRunnerDetail(runnerId, connected);
  const revokeRunner = useRevokeRunner();

  const [renaming, setRenaming] = useState(false);
  const [regenerating, setRegenerating] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);

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
        <PageHeader title="Runner" parent={{ label: 'Runners', to: workspacePath(slug, 'runners') }} />
        <p className="text-sm text-steel">This runner could not be loaded — it may have been removed.</p>
      </>
    );
  }

  const runner = detail.data;

  return (
    <>
      <PageHeader
        title={runner.name}
        parent={{ label: 'Runners', to: workspacePath(slug, 'runners') }}
        actions={
          <div className="flex flex-wrap items-center justify-end gap-2">
            <LifecycleControls runner={runner} />
            <Button size="sm" variant="secondary" onClick={() => setRenaming(true)}>
              <Pencil size={14} aria-hidden="true" />
              Rename
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setRegenerating(true)}>
              <KeyRound size={14} aria-hidden="true" />
              Regenerate token
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="text-danger hover:bg-danger/10 hover:text-danger"
              onClick={() => setConfirmRevoke(true)}
            >
              <Trash2 size={14} aria-hidden="true" />
              Revoke
            </Button>
          </div>
        }
      />

      <div className="mb-6 flex flex-wrap items-center gap-2 text-xs text-steel">
        <RunnerStatusBadge status={runner.status} draining={runner.draining} />
        {runner.labels.map((label) => (
          <Badge key={label} variant="outline">
            {label}
          </Badge>
        ))}
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Identity</h2>
          </CardHeader>
          <CardBody>
            <dl className="space-y-3 text-sm">
              <div className="flex items-center justify-between gap-4">
                <dt className="text-steel">Runner ID</dt>
                <dd className="truncate font-mono text-xs text-charcoal">{runner.id}</dd>
              </div>
              <div className="flex items-center justify-between gap-4">
                <dt className="text-steel">Version</dt>
                <dd className="text-charcoal">{runner.version ?? '—'}</dd>
              </div>
              <div className="flex items-center justify-between gap-4">
                <dt className="text-steel">Registered</dt>
                <dd title={format(new Date(runner.createdAt), 'PPpp')} className="text-charcoal">
                  {formatDistanceToNow(new Date(runner.createdAt), { addSuffix: true })}
                </dd>
              </div>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Connectivity</h2>
          </CardHeader>
          <CardBody>
            <dl className="space-y-3 text-sm">
              <div className="flex items-center justify-between gap-4">
                <dt className="text-steel">Status</dt>
                <dd>
                  <RunnerStatusBadge status={runner.status} draining={runner.draining} />
                </dd>
              </div>
              <div className="flex items-center justify-between gap-4">
                <dt className="text-steel">Last seen</dt>
                <dd className="text-charcoal">
                  {runner.lastSeenAt ? (
                    <span title={format(new Date(runner.lastSeenAt), 'PPpp')}>
                      {formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })}
                    </span>
                  ) : (
                    'Never'
                  )}
                </dd>
              </div>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Health</h2>
          </CardHeader>
          <CardBody>
            <HealthPanel health={runner.lastHealth} />
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Host</h2>
          </CardHeader>
          <CardBody>
            <RunnerMetaCard health={runner.lastHealth} />
          </CardBody>
        </Card>
      </div>

      <RenameRunnerDialog runner={renaming ? runner : null} onClose={() => setRenaming(false)} />
      <RegenerateTokenTrigger
        runner={regenerating ? runner : null}
        onClose={() => setRegenerating(false)}
      />

      <Dialog
        open={confirmRevoke}
        onClose={() => setConfirmRevoke(false)}
        title={`Revoke ${runner.name}?`}
        description="The runner is permanently disconnected and can never authenticate again. Any in-flight job is recovered and requeued. This cannot be undone."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmRevoke(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={revokeRunner.isPending}
              onClick={() =>
                runnerId &&
                revokeRunner.mutate(runnerId, {
                  onSuccess: () => navigate(workspacePath(slug, 'runners')),
                })
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
