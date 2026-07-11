import { format, formatDistanceToNow } from 'date-fns';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import type { ArtifactCatalogEntry } from '../../../types/artifact';
import type { Artifact } from '../../../types/pipeline';
import { formatBytes } from '../../pipelines/lib/format';

export function artifactStatusBadge(status: Artifact['status']) {
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

interface ArtifactsTableProps {
  slug: string;
  artifacts: ArtifactCatalogEntry[];
}

/**
 * The catalog table. Rows navigate to the artifact detail page; columns
 * prune below `md` so phones keep name / size / status.
 */
export function ArtifactsTable({ slug, artifacts }: ArtifactsTableProps) {
  const navigate = useNavigate();

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Name</Th>
          <Th className="hidden md:table-cell">Repository</Th>
          <Th className="hidden lg:table-cell">Pipeline</Th>
          <Th>Size</Th>
          <Th>Status</Th>
          <Th className="hidden lg:table-cell">Expires</Th>
          <Th className="hidden sm:table-cell">Created</Th>
        </Tr>
      </THead>
      <TBody>
        {artifacts.map((artifact) => (
          <Tr
            key={artifact.id}
            className="cursor-pointer transition-colors hover:bg-surface"
            onClick={() => navigate(workspacePath(slug, `artifacts/${artifact.id}`))}
          >
            <Td className="max-w-[220px] truncate font-mono text-xs text-charcoal">
              {artifact.name}
            </Td>
            <Td className="hidden max-w-[200px] truncate text-xs text-steel md:table-cell">
              {artifact.repositoryFullName}
            </Td>
            <Td className="hidden text-xs text-steel lg:table-cell">
              #{artifact.pipelineNumber} · {artifact.workflowName}
            </Td>
            <Td className="text-xs text-charcoal">{formatBytes(artifact.sizeBytes)}</Td>
            <Td>{artifactStatusBadge(artifact.status)}</Td>
            <Td className="hidden text-xs text-steel lg:table-cell">
              {artifact.expiresAt
                ? formatDistanceToNow(new Date(artifact.expiresAt), { addSuffix: true })
                : '—'}
            </Td>
            <Td
              className="hidden text-xs text-steel sm:table-cell"
              title={format(new Date(artifact.createdAt), 'PPpp')}
            >
              {format(new Date(artifact.createdAt), 'PP')}
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
