import { GitBranch, Layers, MoreHorizontal, Square } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { MenuItem, Popover } from '../../../components/ui/Popover';
import { TBody, THead, Table, Td, Th, Tr } from '../../../components/ui/Table';
import { useNow } from '../../../lib/useNow';
import type { QueueJob } from '../../../types/job';
import { branchOfRef, formatDuration } from '../../pipelines/lib/format';
import { QueueReasonBadge } from './QueueReasonBadge';

interface QueueTableProps {
  slug: string;
  jobs: QueueJob[];
  onCancel: (job: QueueJob) => void;
}

/** Waiting time for queued rows, run time for executing rows — both tick. */
function waitingCell(job: QueueJob): string {
  if (job.status === 'in_progress') {
    return job.startedAt ? formatDuration(job.startedAt, null) : formatDuration(job.queuedAt, null);
  }
  return formatDuration(job.queuedAt, null);
}

/**
 * The scheduler console's primary table: one row per active job across
 * every live pipeline, in queue order. Columns prune on small screens and
 * the pruned facts fold into the Job cell, matching the pipeline ledger.
 */
export function QueueTable({ slug, jobs, onCancel }: QueueTableProps) {
  const navigate = useNavigate();
  useNow(jobs.length > 0);

  const jobPath = (job: QueueJob) =>
    workspacePath(slug, `pipelines/${job.pipelineId}/jobs/${job.id}`);

  return (
    <Table>
      <THead>
        <Tr>
          <Th className="w-8">#</Th>
          <Th>Job</Th>
          <Th className="hidden sm:table-cell">Pipeline</Th>
          <Th className="hidden md:table-cell">Repository</Th>
          <Th className="hidden lg:table-cell">Labels</Th>
          <Th>Waiting</Th>
          <Th>Reason</Th>
          <Th className="w-8">
            <span className="sr-only">Actions</span>
          </Th>
        </Tr>
      </THead>
      <TBody>
        {jobs.map((job, index) => (
          <Tr
            key={job.id}
            onClick={() => navigate(jobPath(job))}
            className="cursor-pointer transition-colors hover:bg-surface"
          >
            <Td className="font-mono text-xs text-steel">{index + 1}</Td>
            <Td className="max-w-0">
              <div className="min-w-0">
                <div className="truncate font-medium text-charcoal">{job.name ?? job.key}</div>
                <div className="flex flex-wrap items-center gap-x-2 text-xs text-steel">
                  <span className="min-w-0 truncate">
                    {job.workflowName}
                    <span className="font-mono"> #{job.pipelineNumber}</span>
                  </span>
                  <span className="inline-flex items-center gap-1 font-mono">
                    <GitBranch size={11} aria-hidden="true" />
                    {branchOfRef(job.gitRef)}
                  </span>
                  {(job.actorLogin || job.actorAvatarUrl) && (
                    <span className="inline-flex items-center gap-1 align-middle">
                      <Avatar size="xs" login={job.actorLogin} avatarUrl={job.actorAvatarUrl} />
                      {job.actorLogin}
                    </span>
                  )}
                </div>
                {/* On phones the Repository column is hidden; fold it in here. */}
                <div className="truncate text-xs text-steel md:hidden">{job.repoFullName}</div>
              </div>
            </Td>
            <Td className="hidden sm:table-cell">
              <span className="inline-flex items-center gap-1.5 font-mono text-xs text-charcoal">
                <Layers size={13} className="text-steel" aria-hidden="true" />#
                {job.pipelineNumber}
              </span>
            </Td>
            <Td className="hidden text-xs text-charcoal md:table-cell">{job.repoFullName}</Td>
            <Td className="hidden lg:table-cell">
              {job.runsOn.length > 0 ? (
                <span className="flex max-w-[220px] flex-wrap gap-1">
                  {job.runsOn.map((label) => (
                    <Badge key={label} variant="outline">
                      {label}
                    </Badge>
                  ))}
                </span>
              ) : (
                <span className="text-xs text-steel">any</span>
              )}
            </Td>
            <Td className="whitespace-nowrap font-mono text-xs text-charcoal">
              {waitingCell(job)}
            </Td>
            <Td>
              <QueueReasonBadge reason={job.queueReason} />
            </Td>
            <Td onClick={(event) => event.stopPropagation()}>
              <Popover
                ariaLabel={`Actions for ${job.name ?? job.key}`}
                align="end"
                renderTrigger={(triggerProps) => (
                  <button
                    type="button"
                    {...triggerProps}
                    className="rounded p-1 text-steel transition-colors hover:bg-surface hover:text-charcoal"
                  >
                    <MoreHorizontal size={16} aria-hidden="true" />
                    <span className="sr-only">Actions</span>
                  </button>
                )}
              >
                {({ close }) => (
                  <>
                    <MenuItem
                      onSelect={() => {
                        close();
                        navigate(jobPath(job));
                      }}
                    >
                      Open job
                    </MenuItem>
                    <MenuItem
                      onSelect={() => {
                        close();
                        navigate(workspacePath(slug, `pipelines/${job.pipelineId}`));
                      }}
                    >
                      View pipeline
                    </MenuItem>
                    <MenuItem
                      icon={Square}
                      destructive
                      onSelect={() => {
                        close();
                        onCancel(job);
                      }}
                    >
                      Cancel job
                    </MenuItem>
                  </>
                )}
              </Popover>
            </Td>
          </Tr>
        ))}
      </TBody>
    </Table>
  );
}
