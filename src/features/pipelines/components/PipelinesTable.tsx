import { format, formatDistanceToNow } from 'date-fns';
import { GitBranch, GitCommitHorizontal, MousePointerClick, Webhook } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import type { Pipeline } from '../../../types/pipeline';
import { branchOfRef, formatDuration, shortSha } from '../lib/format';
import { PipelineStatusBadge } from './PipelineStatusBadge';

/** Trigger source icon + label, matching the API trigger vocabulary. */
function TriggerCell({ trigger }: { trigger: Pipeline['trigger'] }) {
  const Icon = trigger === 'push' ? Webhook : MousePointerClick;
  const label = trigger === 'push' ? 'Push' : 'Manual';
  return (
    <span className="inline-flex items-center gap-1.5 text-steel">
      <Icon size={13} aria-hidden="true" />
      {label}
    </span>
  );
}

export function PipelinesTable({ slug, pipelines }: { slug: string; pipelines: Pipeline[] }) {
  const navigate = useNavigate();

  return (
    <Table>
      <THead>
        <Tr>
          <Th>Status</Th>
          <Th>Pipeline</Th>
          <Th className="hidden sm:table-cell">Commit</Th>
          <Th className="hidden md:table-cell">Branch</Th>
          <Th className="hidden md:table-cell">Trigger</Th>
          <Th className="hidden md:table-cell">Duration</Th>
          <Th className="hidden sm:table-cell">Created</Th>
        </Tr>
      </THead>
      <TBody>
        {pipelines.map((pipeline) => (
          <Tr
            key={pipeline.id}
            onClick={() => navigate(workspacePath(slug, `pipelines/${pipeline.id}`))}
            className="cursor-pointer transition-colors hover:bg-surface"
          >
            <Td>
              <PipelineStatusBadge status={pipeline.status} conclusion={pipeline.conclusion} />
            </Td>
            <Td>
              <div className="font-medium text-charcoal">
                {pipeline.workflowName}{' '}
                <span className="font-mono text-xs text-steel">#{pipeline.number}</span>
              </div>
              <div className="text-xs text-steel">{pipeline.repoFullName}</div>
              {/* On phones the Created column is hidden; fold it in here. */}
              <div className="text-xs text-steel sm:hidden">
                {formatDistanceToNow(new Date(pipeline.createdAt), { addSuffix: true })}
              </div>
            </Td>
            <Td className="hidden sm:table-cell">
              <div className="flex items-center gap-1.5 font-mono text-xs text-charcoal">
                <GitCommitHorizontal size={13} className="text-steel" aria-hidden="true" />
                {shortSha(pipeline.commitSha)}
              </div>
              {pipeline.commitMessage && (
                <div className="max-w-[220px] truncate text-xs text-steel">
                  {pipeline.commitMessage}
                  {pipeline.commitAuthor ? ` — ${pipeline.commitAuthor}` : ''}
                </div>
              )}
            </Td>
            <Td className="hidden md:table-cell">
              <span className="inline-flex items-center gap-1.5 font-mono text-xs text-charcoal">
                <GitBranch size={13} className="text-steel" aria-hidden="true" />
                {branchOfRef(pipeline.gitRef)}
              </span>
            </Td>
            <Td className="hidden md:table-cell">
              <TriggerCell trigger={pipeline.trigger} />
            </Td>
            <Td className="hidden font-mono text-xs text-charcoal md:table-cell">
              {formatDuration(pipeline.startedAt, pipeline.finishedAt)}
            </Td>
            <Td
              className="hidden text-xs text-steel sm:table-cell"
              title={format(new Date(pipeline.createdAt), 'PPpp')}
            >
              {formatDistanceToNow(new Date(pipeline.createdAt), { addSuffix: true })}
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
