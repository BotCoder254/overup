import { format, formatDistanceToNow } from 'date-fns';
import { Boxes } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import type { Environment } from '../../../types/environment';

interface EnvironmentsTableProps {
  slug: string;
  environments: Environment[];
}

/**
 * The environments catalog table. Rows navigate to the environment detail
 * page; columns prune below `md` so phones keep name / secrets / created.
 */
export function EnvironmentsTable({ slug, environments }: EnvironmentsTableProps) {
  const navigate = useNavigate();

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Name</Th>
          <Th>Secrets</Th>
          <Th className="hidden md:table-cell">Created by</Th>
          <Th className="hidden sm:table-cell">Created</Th>
          <Th className="hidden lg:table-cell">Updated</Th>
        </Tr>
      </THead>
      <TBody>
        {environments.map((environment) => (
          <Tr
            key={environment.id}
            className="cursor-pointer transition-colors hover:bg-surface"
            onClick={() => navigate(workspacePath(slug, `environments/${environment.id}`))}
          >
            <Td className="max-w-[260px]">
              <span className="flex min-w-0 items-center gap-1.5">
                <Boxes size={12} aria-hidden="true" className="shrink-0 text-steel" />
                <span className="truncate font-mono text-xs text-charcoal">
                  {environment.name}
                </span>
              </span>
              {environment.description && (
                <span className="mt-0.5 block truncate text-xs text-steel">
                  {environment.description}
                </span>
              )}
            </Td>
            <Td>
              <Badge variant={environment.secretCount > 0 ? 'primary' : 'outline'}>
                {environment.secretCount}
              </Badge>
            </Td>
            <Td className="hidden text-xs text-steel md:table-cell">
              {environment.creatorLogin ?? '—'}
            </Td>
            <Td
              className="hidden text-xs text-steel sm:table-cell"
              title={format(new Date(environment.createdAt), 'PPpp')}
            >
              {format(new Date(environment.createdAt), 'PP')}
            </Td>
            <Td
              className="hidden text-xs text-steel lg:table-cell"
              title={format(new Date(environment.updatedAt), 'PPpp')}
            >
              {formatDistanceToNow(new Date(environment.updatedAt), { addSuffix: true })}
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
