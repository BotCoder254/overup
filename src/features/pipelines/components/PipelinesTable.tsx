import { format, formatDistanceToNow } from 'date-fns';
import {
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  MousePointerClick,
  Tag,
  Webhook,
} from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Avatar } from '../../../components/ui/Avatar';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import type { Pipeline } from '../../../types/pipeline';
import { branchOfRef, formatDuration, shortSha } from '../lib/format';
import { PipelineStatusBadge } from './PipelineStatusBadge';

/** Trigger source icon + label, matching the API trigger vocabulary. */
const TRIGGER_PRESENTATION: Record<
  Pipeline['trigger'],
  { icon: typeof Webhook; label: string }
> = {
  push: { icon: Webhook, label: 'Push' },
  manual: { icon: MousePointerClick, label: 'Manual' },
  pull_request: { icon: GitPullRequest, label: 'Pull request' },
  tag: { icon: Tag, label: 'Tag' },
};

function TriggerCell({ trigger }: { trigger: Pipeline['trigger'] }) {
  const { icon: Icon, label } = TRIGGER_PRESENTATION[trigger] ?? TRIGGER_PRESENTATION.push;
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
              {/* On phones the Commit/Created columns are hidden; fold the
                  actor + relative time in here. */}
              <div className="mt-0.5 flex items-center gap-1.5 text-xs text-steel sm:hidden">
                <Avatar
                  size="xs"
                  login={pipeline.actorLogin ?? pipeline.commitAuthor}
                  avatarUrl={pipeline.actorAvatarUrl}
                />
                <span className="truncate">
                  {pipeline.actorLogin ?? pipeline.commitAuthor ?? 'system'}
                </span>
                <span className="shrink-0">
                  · {formatDistanceToNow(new Date(pipeline.createdAt), { addSuffix: true })}
                </span>
              </div>
            </Td>
            <Td className="hidden sm:table-cell">
              <div className="flex items-center gap-1.5 font-mono text-xs text-charcoal">
                <GitCommitHorizontal size={13} className="text-steel" aria-hidden="true" />
                {shortSha(pipeline.commitSha)}
              </div>
              <div className="mt-0.5 flex max-w-[220px] items-center gap-1.5 text-xs text-steel">
                <Avatar
                  size="xs"
                  login={pipeline.actorLogin ?? pipeline.commitAuthor}
                  avatarUrl={pipeline.actorAvatarUrl}
                  title={pipeline.actorLogin ?? pipeline.commitAuthor ?? undefined}
                />
                <span className="truncate">
                  {pipeline.commitMessage ?? pipeline.actorLogin ?? pipeline.commitAuthor ?? '—'}
                </span>
              </div>
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
