import { formatDistanceToNow } from 'date-fns';
import { Search, Workflow as WorkflowIcon } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Badge } from '../../../components/ui/Badge';
import { EmptyState } from '../../../components/ui/EmptyState';
import { Input } from '../../../components/ui/Input';
import { Spinner } from '../../../components/ui/Spinner';
import { TBody, Table, Td, Th, THead, Tr } from '../../../components/ui/Table';
import { workspacePath } from '../../../app/navigation';
import { ValidationBadge } from '../components/ValidationBadge';
import { useWorkflows } from '../hooks/useWorkflows';

export function WorkflowsPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const navigate = useNavigate();
  const workflows = useWorkflows();
  const [filter, setFilter] = useState('');

  const filtered = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const rows = workflows.data ?? [];
    if (!needle) return rows;
    return rows.filter(
      (workflow) =>
        workflow.name.toLowerCase().includes(needle) ||
        workflow.path.toLowerCase().includes(needle) ||
        workflow.repoFullName.toLowerCase().includes(needle) ||
        workflow.triggers.some((trigger) => trigger.includes(needle)),
    );
  }, [workflows.data, filter]);

  return (
    <>
      <PageHeader
        title="Workflows"
        description="Every automation workflow discovered across the workspace's connected repositories — with triggers, structure, and validation status."
        actions={
          <div className="relative">
            <Search
              size={14}
              className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-steel"
              aria-hidden="true"
            />
            <Input
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder="Filter workflows…"
              aria-label="Filter workflows"
              className="h-9 w-56 pl-8 text-sm"
            />
          </div>
        }
      />

      {workflows.isLoading ? (
        <div className="flex min-h-[40vh] items-center justify-center">
          <Spinner className="h-6 w-6 text-steel" />
        </div>
      ) : (workflows.data?.length ?? 0) === 0 ? (
        <EmptyState
          icon={WorkflowIcon}
          title="No workflows discovered yet"
          description="Connect a repository with files in .github/workflows and they will be discovered, parsed, and validated automatically on every sync."
          className="min-h-[50vh] border-0 bg-transparent"
        />
      ) : filtered.length === 0 ? (
        <p className="py-12 text-center text-sm text-steel">No workflows match that filter.</p>
      ) : (
        <Table>
          <THead>
            <Tr>
              <Th>Workflow</Th>
              <Th>Repository</Th>
              <Th>Triggers</Th>
              <Th>Jobs</Th>
              <Th>Validation</Th>
              <Th>Updated</Th>
            </Tr>
          </THead>
          <TBody>
            {filtered.map((workflow) => (
              <Tr
                key={workflow.id}
                onClick={() => navigate(workspacePath(slug, `workflows/${workflow.id}`))}
                className="cursor-pointer hover:bg-surface"
              >
                <Td>
                  <p className="font-medium text-charcoal">{workflow.name}</p>
                  <p className="mt-0.5 font-mono text-xs text-steel">{workflow.path}</p>
                </Td>
                <Td className="text-steel">{workflow.repoFullName}</Td>
                <Td>
                  <div className="flex max-w-[14rem] flex-wrap gap-1">
                    {workflow.triggers.slice(0, 4).map((trigger) => (
                      <Badge key={trigger} variant="info">
                        {trigger}
                      </Badge>
                    ))}
                    {workflow.triggers.length > 4 && (
                      <Badge variant="neutral">+{workflow.triggers.length - 4}</Badge>
                    )}
                  </div>
                </Td>
                <Td className="text-steel">{workflow.jobCount}</Td>
                <Td>
                  <ValidationBadge status={workflow.validationStatus} />
                </Td>
                <Td className="whitespace-nowrap text-xs text-steel">
                  {formatDistanceToNow(new Date(workflow.updatedAt), { addSuffix: true })}
                </Td>
              </Tr>
            ))}
          </TBody>
        </Table>
      )}
    </>
  );
}
