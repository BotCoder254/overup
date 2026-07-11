import { format, formatDistanceToNow } from 'date-fns';
import { AlertTriangle, EyeOff, Pencil, RefreshCw, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { EmptyState } from '../../../components/ui/EmptyState';
import { FormField } from '../../../components/ui/FormField';
import { Spinner } from '../../../components/ui/Spinner';
import { Textarea } from '../../../components/ui/Textarea';
import { SecretAuditList } from '../components/SecretAuditList';
import { SecretFormDialog } from '../components/SecretFormDialog';
import { secretScopeBadge } from '../components/SecretsTable';
import {
  useDeleteSecret,
  useSecretDetail,
  useUpdateSecretDescription,
} from '../hooks/useSecrets';

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
 * One secret: metadata, usage telemetry, and its immutable audit history —
 * never the value. Actions: replace the value, edit the description, and
 * delete (each audited).
 */
export function SecretDetailPage() {
  const { slug = '', secretId } = useParams<{ slug: string; secretId: string }>();
  const navigate = useNavigate();
  const detail = useSecretDetail(secretId);
  const updateDescription = useUpdateSecretDescription();
  const deleteSecret = useDeleteSecret();

  const [replaceOpen, setReplaceOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editingDescription, setEditingDescription] = useState<string | null>(null);

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[40vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }

  const secret = detail.data?.secret;
  if (!secret) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Secret not found"
        description="It may have been deleted. The catalog shows every stored secret."
        action={
          <Button size="sm" onClick={() => navigate(workspacePath(slug, 'secrets'))}>
            Back to Secrets
          </Button>
        }
      />
    );
  }

  return (
    <>
      <PageHeader
        title={secret.name}
        parent={{ label: 'Secrets', to: workspacePath(slug, 'secrets') }}
        actions={
          <>
            <Button size="sm" onClick={() => setReplaceOpen(true)}>
              <RefreshCw size={14} aria-hidden="true" />
              Replace value
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

      <SecretFormDialog
        open={replaceOpen}
        onClose={() => setReplaceOpen(false)}
        replaceTarget={secret}
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Metadata</h2>
            <span className="ml-auto">{secretScopeBadge(secret.scope)}</span>
          </CardHeader>
          <CardBody>
            <dl className="divide-y divide-steel/10">
              <MetaRow label="Value">
                <span className="flex items-center gap-1.5 text-steel">
                  <EyeOff size={14} aria-hidden="true" />
                  Encrypted — can be replaced, never shown
                </span>
              </MetaRow>
              <MetaRow label="Scope">
                {secret.scope === 'repository' ? 'Repository' : 'Workspace'}
              </MetaRow>
              {secret.repositoryId && (
                <MetaRow label="Repository">
                  <Link
                    to={workspacePath(slug, `repositories/${secret.repositoryId}`)}
                    className="text-link hover:underline"
                  >
                    {secret.repositoryName ?? 'Repository'}
                  </Link>
                </MetaRow>
              )}
              <MetaRow label="Description">
                {editingDescription === null ? (
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="min-w-0 break-words">{secret.description ?? '—'}</span>
                    <button
                      type="button"
                      onClick={() => setEditingDescription(secret.description ?? '')}
                      aria-label="Edit description"
                      title="Edit description"
                      className="shrink-0 rounded border border-steel/20 bg-canvas p-1 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                    >
                      <Pencil size={12} aria-hidden="true" />
                    </button>
                  </span>
                ) : (
                  <span className="block space-y-2">
                    <FormField
                      id="secret-description-edit"
                      label="Description"
                      optional
                      error={
                        editingDescription.length > 500 ? 'At most 500 characters.' : undefined
                      }
                    >
                      {(aria) => (
                        <Textarea
                          {...aria}
                          rows={2}
                          maxLength={600}
                          value={editingDescription}
                          onChange={(event) => setEditingDescription(event.target.value)}
                        />
                      )}
                    </FormField>
                    <span className="flex gap-2">
                      <Button
                        size="sm"
                        disabled={editingDescription.length > 500}
                        isLoading={updateDescription.isPending}
                        onClick={() =>
                          updateDescription.mutate(
                            { secretId: secret.id, description: editingDescription },
                            { onSuccess: () => setEditingDescription(null) },
                          )
                        }
                      >
                        Save
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setEditingDescription(null)}
                      >
                        Cancel
                      </Button>
                    </span>
                  </span>
                )}
              </MetaRow>
              <MetaRow label="Created">
                <span title={format(new Date(secret.createdAt), 'PPpp')}>
                  {format(new Date(secret.createdAt), 'PPpp')}
                  {secret.creatorLogin && (
                    <span className="text-steel"> by {secret.creatorLogin}</span>
                  )}
                </span>
              </MetaRow>
              <MetaRow label="Last modified">
                <span title={format(new Date(secret.updatedAt), 'PPpp')}>
                  {formatDistanceToNow(new Date(secret.updatedAt), { addSuffix: true })}
                  {secret.updaterLogin && (
                    <span className="text-steel"> by {secret.updaterLogin}</span>
                  )}
                </span>
              </MetaRow>
              <MetaRow label="Last used">
                {secret.lastUsedAt ? (
                  <span title={format(new Date(secret.lastUsedAt), 'PPpp')}>
                    {formatDistanceToNow(new Date(secret.lastUsedAt), { addSuffix: true })}
                  </span>
                ) : (
                  'Never'
                )}
              </MetaRow>
              <MetaRow label="Injections">{secret.usageCount.toLocaleString()}</MetaRow>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Audit history</h2>
          </CardHeader>
          <CardBody>
            <SecretAuditList events={detail.data?.audit ?? []} />
          </CardBody>
        </Card>
      </div>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={`Delete ${secret.name}?`}
        description="Future pipeline runs stop receiving this secret immediately. The encrypted value is destroyed and cannot be recovered. This cannot be undone."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={deleteSecret.isPending}
              onClick={() =>
                deleteSecret.mutate(secret.id, {
                  onSuccess: () => {
                    setConfirmDelete(false);
                    navigate(workspacePath(slug, 'secrets'));
                  },
                })
              }
            >
              Delete secret
            </Button>
          </>
        }
      />
    </>
  );
}
