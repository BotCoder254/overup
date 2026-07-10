import { format } from 'date-fns';
import { Download, Package } from 'lucide-react';
import { Badge } from '../../../../components/ui/Badge';
import { Button } from '../../../../components/ui/Button';
import { EmptyState } from '../../../../components/ui/EmptyState';
import { Spinner } from '../../../../components/ui/Spinner';
import { TBody, THead, Table, Td, Th, Tr } from '../../../../components/ui/Table';
import type { Artifact } from '../../../../types/pipeline';
import { useArtifacts, useDownloadArtifact } from '../../hooks/usePipelines';
import { formatBytes } from '../../lib/format';

function statusBadge(status: Artifact['status']) {
  switch (status) {
    case 'uploaded':
      return <Badge variant="success">Available</Badge>;
    case 'pending':
      return <Badge variant="neutral">Uploading</Badge>;
    case 'expired':
      return <Badge variant="neutral">Expired</Badge>;
    default:
      return <Badge variant="danger">Failed</Badge>;
  }
}

/** Artifacts produced by this pipeline; downloads use short-lived presigned
 * URLs, so the bytes never pass through the control plane. */
export function ArtifactsPanel({ pipelineId }: { pipelineId: string }) {
  const artifacts = useArtifacts(pipelineId);
  const download = useDownloadArtifact();

  if (artifacts.isLoading) {
    return (
      <div className="flex justify-center py-8">
        <Spinner className="h-5 w-5 text-steel" />
      </div>
    );
  }
  const rows = artifacts.data ?? [];
  if (rows.length === 0) {
    return (
      <EmptyState
        icon={Package}
        title="No artifacts"
        description="Files a job leaves in .overup/artifacts/ inside its workspace are uploaded here."
      />
    );
  }

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Name</Th>
          <Th>Size</Th>
          <Th>Status</Th>
          <Th>Checksum</Th>
          <Th>Created</Th>
          <Th>
            <span className="sr-only">Download</span>
          </Th>
        </Tr>
      </THead>
      <TBody>
        {rows.map((artifact) => (
          <Tr key={artifact.id}>
            <Td className="font-mono text-xs text-charcoal">{artifact.name}</Td>
            <Td className="text-xs text-charcoal">{formatBytes(artifact.sizeBytes)}</Td>
            <Td>{statusBadge(artifact.status)}</Td>
            <Td className="max-w-[140px] truncate font-mono text-[10px] text-steel">
              {artifact.checksumSha256 ?? '—'}
            </Td>
            <Td
              className="text-xs text-steel"
              title={format(new Date(artifact.createdAt), 'PPpp')}
            >
              {format(new Date(artifact.createdAt), 'PP')}
            </Td>
            <Td>
              <Button
                variant="ghost"
                size="sm"
                disabled={artifact.status !== 'uploaded'}
                isLoading={download.isPending && download.variables === artifact.id}
                onClick={() => download.mutate(artifact.id)}
              >
                <Download size={14} aria-hidden="true" />
                <span className="sr-only">Download {artifact.name}</span>
              </Button>
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
