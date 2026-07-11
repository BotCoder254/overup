import { format, formatDistanceToNow } from 'date-fns';
import { Lock } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import type { Secret, SecretScope } from '../../../types/secret';

export function secretScopeBadge(scope: SecretScope) {
  if (scope === 'repository') return <Badge variant="info">Repository</Badge>;
  if (scope === 'environment') return <Badge variant="success">Environment</Badge>;
  return <Badge variant="primary">Workspace</Badge>;
}

interface SecretsTableProps {
  slug: string;
  secrets: Secret[];
}

/**
 * The secrets catalog table. Rows navigate to the secret detail page;
 * columns prune below `md` so phones keep name / scope / last used.
 * There is deliberately no value column — values are write-only.
 */
export function SecretsTable({ slug, secrets }: SecretsTableProps) {
  const navigate = useNavigate();

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Name</Th>
          <Th>Scope</Th>
          <Th className="hidden md:table-cell">Target</Th>
          <Th className="hidden md:table-cell">Created by</Th>
          <Th>Last used</Th>
          <Th className="hidden lg:table-cell">Uses</Th>
          <Th className="hidden sm:table-cell">Rotated</Th>
        </Tr>
      </THead>
      <TBody>
        {secrets.map((secret) => (
          <Tr
            key={secret.id}
            className="cursor-pointer transition-colors hover:bg-surface"
            onClick={() => navigate(workspacePath(slug, `secrets/${secret.id}`))}
          >
            <Td className="max-w-[240px]">
              <span className="flex min-w-0 items-center gap-1.5">
                <Lock size={12} aria-hidden="true" className="shrink-0 text-steel" />
                <span className="truncate font-mono text-xs text-charcoal">{secret.name}</span>
              </span>
              {secret.description && (
                <span className="mt-0.5 block truncate text-xs text-steel">
                  {secret.description}
                </span>
              )}
            </Td>
            <Td>{secretScopeBadge(secret.scope)}</Td>
            <Td className="hidden max-w-[200px] truncate text-xs text-steel md:table-cell">
              {secret.repositoryName ?? secret.environmentName ?? '—'}
            </Td>
            <Td className="hidden text-xs text-steel md:table-cell">
              {secret.creatorLogin ?? '—'}
            </Td>
            <Td className="text-xs text-steel">
              {secret.lastUsedAt ? (
                <span title={format(new Date(secret.lastUsedAt), 'PPpp')}>
                  {formatDistanceToNow(new Date(secret.lastUsedAt), { addSuffix: true })}
                </span>
              ) : (
                'Never'
              )}
            </Td>
            <Td className="hidden text-xs text-charcoal lg:table-cell">
              {secret.usageCount.toLocaleString()}
            </Td>
            <Td
              className="hidden text-xs text-steel sm:table-cell"
              title={format(new Date(secret.valueSetAt), 'PPpp')}
            >
              {formatDistanceToNow(new Date(secret.valueSetAt), { addSuffix: true })}
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
