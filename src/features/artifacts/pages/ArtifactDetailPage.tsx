import { format, formatDistanceToNow } from 'date-fns';
import { AlertTriangle, Copy, Download, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { toast } from 'sonner';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { Dialog } from '../../../components/ui/Dialog';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Spinner } from '../../../components/ui/Spinner';
import { formatBytes, shortSha } from '../../pipelines/lib/format';
import { artifactStatusBadge } from '../components/ArtifactsTable';
import {
  useArtifactDetail,
  useDeleteArtifact,
  useDownloadArtifact,
} from '../hooks/useArtifactsCatalog';

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
 * One artifact: metadata (size, type, checksum, retention) and the exact
 * provenance of the execution that produced it, with download/delete
 * actions. Reached from the catalog and from pipeline/job artifact panels.
 */
export function ArtifactDetailPage() {
  const { slug = '', artifactId } = useParams<{ slug: string; artifactId: string }>();
  const navigate = useNavigate();
  const detail = useArtifactDetail(artifactId);
  const download = useDownloadArtifact();
  const deleteArtifact = useDeleteArtifact();
  const [confirmDelete, setConfirmDelete] = useState(false);

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[40vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }

  const artifact = detail.data;
  if (!artifact) {
    return (
      <EmptyState
        icon={AlertTriangle}
        title="Artifact not found"
        description="It may have been deleted or expired. The catalog shows everything still stored."
        action={
          <Button size="sm" onClick={() => navigate(workspacePath(slug, 'artifacts'))}>
            Back to Artifacts
          </Button>
        }
      />
    );
  }

  const copyChecksum = async () => {
    if (!artifact.checksumSha256) return;
    try {
      await navigator.clipboard.writeText(artifact.checksumSha256);
      toast.success('Checksum copied to clipboard.');
    } catch {
      toast.error('Could not access the clipboard.');
    }
  };

  return (
    <>
      <PageHeader
        title={artifact.name}
        parent={{ label: 'Artifacts', to: workspacePath(slug, 'artifacts') }}
        actions={
          <>
            <Button
              size="sm"
              disabled={artifact.status !== 'uploaded'}
              isLoading={download.isPending}
              onClick={() => download.mutate(artifact.id)}
            >
              <Download size={14} aria-hidden="true" />
              Download
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

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Metadata</h2>
            <span className="ml-auto">{artifactStatusBadge(artifact.status)}</span>
          </CardHeader>
          <CardBody>
            <dl className="divide-y divide-steel/10">
              <MetaRow label="Size">{formatBytes(artifact.sizeBytes)}</MetaRow>
              <MetaRow label="Content type">{artifact.contentType ?? '—'}</MetaRow>
              <MetaRow label="Checksum (SHA-256)">
                {artifact.checksumSha256 ? (
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate font-mono text-xs">{artifact.checksumSha256}</span>
                    <button
                      type="button"
                      onClick={() => void copyChecksum()}
                      aria-label="Copy checksum"
                      title="Copy checksum"
                      className="shrink-0 rounded border border-steel/20 bg-canvas p-1 text-steel transition-colors hover:bg-surface hover:text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                    >
                      <Copy size={12} aria-hidden="true" />
                    </button>
                  </span>
                ) : (
                  '—'
                )}
              </MetaRow>
              <MetaRow label="Created">
                <span title={format(new Date(artifact.createdAt), 'PPpp')}>
                  {format(new Date(artifact.createdAt), 'PPpp')}
                </span>
              </MetaRow>
              <MetaRow label="Expires">
                {artifact.expiresAt ? (
                  <span title={format(new Date(artifact.expiresAt), 'PPpp')}>
                    {formatDistanceToNow(new Date(artifact.expiresAt), { addSuffix: true })}
                  </span>
                ) : (
                  'Never'
                )}
              </MetaRow>
            </dl>
          </CardBody>
        </Card>

        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold text-charcoal">Produced by</h2>
          </CardHeader>
          <CardBody>
            <dl className="divide-y divide-steel/10">
              <MetaRow label="Repository">
                <Link
                  to={workspacePath(slug, `repositories/${artifact.repositoryId}`)}
                  className="text-link hover:underline"
                >
                  {artifact.repositoryFullName}
                </Link>
              </MetaRow>
              <MetaRow label="Workflow">
                {artifact.workflowId ? (
                  <Link
                    to={workspacePath(slug, `workflows/${artifact.workflowId}`)}
                    className="text-link hover:underline"
                  >
                    {artifact.workflowName}
                  </Link>
                ) : (
                  artifact.workflowName
                )}
              </MetaRow>
              <MetaRow label="Pipeline">
                <Link
                  to={workspacePath(slug, `pipelines/${artifact.pipelineId}`)}
                  className="text-link hover:underline"
                >
                  #{artifact.pipelineNumber}
                </Link>
              </MetaRow>
              <MetaRow label="Job">
                <Link
                  to={workspacePath(slug, `pipelines/${artifact.pipelineId}/jobs/${artifact.jobId}`)}
                  className="text-link hover:underline"
                >
                  {artifact.jobName ?? artifact.jobKey}
                </Link>
              </MetaRow>
              <MetaRow label="Branch">{artifact.branch}</MetaRow>
              <MetaRow label="Commit">
                <span className="font-mono text-xs">{shortSha(artifact.commitSha)}</span>
              </MetaRow>
              <MetaRow label="Runner">{artifact.runnerName ?? '—'}</MetaRow>
            </dl>
          </CardBody>
        </Card>
      </div>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={`Delete ${artifact.name}?`}
        description="The stored object is removed and the download stops working immediately. This cannot be undone."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              className="bg-danger text-white hover:bg-danger/80"
              isLoading={deleteArtifact.isPending}
              onClick={() =>
                deleteArtifact.mutate(artifact.id, {
                  onSuccess: () => {
                    setConfirmDelete(false);
                    navigate(workspacePath(slug, 'artifacts'));
                  },
                })
              }
            >
              Delete artifact
            </Button>
          </>
        }
      />
    </>
  );
}
