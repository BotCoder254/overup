import { format, formatDistanceToNow } from 'date-fns';
import { AlertTriangle, Lock, Pencil, Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { SecretFormDialog } from '../../secrets/components/SecretFormDialog';
import { SecretsTable } from '../../secrets/components/SecretsTable';
import { useSecretsCatalog } from '../../secrets/hooks/useSecrets';
import { EnvironmentAuditList } from '../components/EnvironmentAuditList';
import { EnvironmentFormDialog } from '../components/EnvironmentFormDialog';
import { useDeleteEnvironment, useEnvironmentDetail } from '../hooks/useEnvironments';

function MetaRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5 py-2 sm:flex-row sm:items-center sm:gap-4">
      <dt className="w-40 shrink-0 text-xs font-medium uppercase tracking-wider text-steel">
        {label}
      </dt>
      <dd className="min-w-0 text-sm text-charcoal">{children}</dd>
    </div>
  );
}

/**
 * One environment: metadata, the secrets scoped to it, and its audit
 * history. Deleting cascades to the scoped secrets (spelled out in the
 * confirm dialog with the exact count).
 */
export function EnvironmentDetailPage() {
  const { slug = '', environmentId } = useParams<{ slug: string; environmentId: string }>();
  const navigate = useNavigate();
  const detail = useEnvironmentDetail(environmentId);
  const deleteEnvironment = useDeleteEnvironment();
  const secrets = useSecretsCatalog(environmentId ? { environmentId } : {});
  const scopedSecrets = (secrets.data?.pages ?? []).flatMap((page) => page.secrets);

  const [editOpen, setEditOpen] = useState(false);
  const [addSecretOpen, setAddSecretOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[40vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }

  const environment = detail.data?.environment;
  if (!environment) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Environment not found"
        description="It may have been deleted. The catalog shows every environment."
        action={
          <Button size="sm" onClick={() => navigate(workspacePath(slug, 'environments'))}>
            Back to Environments
          </Button>
        }
      />
    );
  }

  return (
    <>
      <PageHeader
        title={environment.name}
        parent={{ label: 'Environments', to: workspacePath(slug, 'environments') }}
        actions={
          <>
            <Button size="sm" onClick={() => setEditOpen(true)}>
              <Pencil size={14} aria-hidden="true" />
              Edit
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="text-danger hover:bg-danger/10"
              onClick={() => setConfirmDelete(true)}
            >
              <Trash2 size={14} aria-hidden="true" />
              Delete
            </Button>
          </>
        }
      />

      <EnvironmentFormDialog
        open={editOpen}
        onClose={() => setEditOpen(false)}
        editTarget={environment}
      />
      <SecretFormDialog
        open={addSecretOpen}
        onClose={() => setAddSecretOpen(false)}
        presetEnvironment={{ id: environment.id, name: environment.name }}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Metadata</h2>
          </CardHeader>
          <CardBody>
            <dl className="divide-y divide-steel/10">
              <MetaRow label="Workflow binding">
                <code className="rounded bg-surface px-1.5 py-0.5 font-mono text-xs">
                  environment: {environment.name}
                </code>
              </MetaRow>
              <MetaRow label="Description">{environment.description ?? '—'}</MetaRow>
              <MetaRow label="Secrets">{environment.secretCount}</MetaRow>
              <MetaRow label="Created">
                <span title={format(new Date(environment.createdAt), 'PPpp')}>
                  {format(new Date(environment.createdAt), 'PPpp')}
                  {environment.creatorLogin && (
                    <span className="text-steel"> by {environment.creatorLogin}</span>
                  )}
                </span>
              </MetaRow>
              <MetaRow label="Last modified">
                <span title={format(new Date(environment.updatedAt), 'PPpp')}>
                  {formatDistanceToNow(new Date(environment.updatedAt), { addSuffix: true })}
                  {environment.updaterLogin && (
                    <span className="text-steel"> by {environment.updaterLogin}</span>
                  )}
                </span>
              </MetaRow>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Audit history</h2>
          </CardHeader>
          <CardBody>
            <EnvironmentAuditList events={detail.data?.audit ?? []} />
          </CardBody>
        </Card>
      </div>

      <Card className="mt-4">
        <CardHeader>
          <h2 className="text-sm font-semibold text-charcoal">Environment secrets</h2>
          <span className="ml-auto">
            <Button size="sm" onClick={() => setAddSecretOpen(true)}>
              <Plus size={14} aria-hidden="true" />
              Add secret
            </Button>
          </span>
        </CardHeader>
        <CardBody>
          {secrets.isLoading ? (
            <div className="flex min-h-[10vh] items-center justify-center">
              <Spinner className="h-5 w-5 text-steel" />
            </div>
          ) : scopedSecrets.length === 0 ? (
            <EmptyState
              icon={Lock}
              title="No secrets in this environment"
              description="Secrets added here are injected only into jobs that declare this environment — with the highest precedence."
              action={
                <Button size="sm" onClick={() => setAddSecretOpen(true)}>
                  <Plus size={14} aria-hidden="true" />
                  Add secret
                </Button>
              }
            />
          ) : (
            <SecretsTable slug={slug} secrets={scopedSecrets} />
          )}
        </CardBody>
      </Card>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={`Delete ${environment.name}?`}
        description={
          environment.secretCount > 0
            ? `This also permanently deletes the ${environment.secretCount} secret${
                environment.secretCount === 1 ? '' : 's'
              } scoped to it. Jobs referencing this environment keep running, without its secrets. This cannot be undone.`
            : 'Jobs referencing this environment keep running, without environment secrets. This cannot be undone.'
        }
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={deleteEnvironment.isPending}
              onClick={() =>
                deleteEnvironment.mutate(environment.id, {
                  onSuccess: () => {
                    setConfirmDelete(false);
                    navigate(workspacePath(slug, 'environments'));
                  },
                })
              }
            >
              Delete environment
            </Button>
          </>
        }
      />
    </>
  );
}
