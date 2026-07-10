import { GitBranch, GitCommitHorizontal, MousePointerClick, Server, Webhook } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { useNow } from '../../../lib/useNow';
import type { Pipeline, PipelineJob } from '../../../types/pipeline';
import type { Runner } from '../../../types/runner';
import { branchOfRef, formatDuration, shortSha } from '../lib/format';

interface JobIdentityBarProps {
  slug: string;
  pipeline: Pipeline;
  job: PipelineJob;
  runner: Runner | null;
}

function Item({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <span className="inline-flex items-baseline gap-1.5 whitespace-nowrap">
      <span className="text-[10px] uppercase tracking-wide text-steel">{label}</span>
      <span className="text-charcoal">{children}</span>
    </span>
  );
}

/**
 * The immutable execution identity, pinned to the top of the Job Execution
 * page (the content canvas is the scroll container, so `sticky` holds it in
 * view through long debugging sessions). Durations tick live while the job
 * is not terminal.
 */
export function JobIdentityBar({ slug, pipeline, job, runner }: JobIdentityBarProps) {
  useNow(job.status !== 'completed');
  const TriggerIcon = pipeline.trigger === 'push' ? Webhook : MousePointerClick;

  return (
    <div className="sticky top-0 z-10 -mx-1 mb-4 border-b border-steel/20 bg-canvas px-1 pb-2">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-xs">
        <Item label="repo">
          <Link
            to={workspacePath(slug, `repositories/${pipeline.repositoryId}`)}
            className="text-link hover:underline"
          >
            {pipeline.repoFullName}
          </Link>
        </Item>
        <Item label="workflow">
          {pipeline.workflowId ? (
            <Link
              to={workspacePath(slug, `workflows/${pipeline.workflowId}`)}
              className="text-link hover:underline"
            >
              {pipeline.workflowName}
            </Link>
          ) : (
            pipeline.workflowName
          )}
        </Item>
        <Item label="pipeline">
          <Link
            to={workspacePath(slug, `pipelines/${pipeline.id}`)}
            className="text-link hover:underline"
          >
            #{pipeline.number}
          </Link>
        </Item>
        <Item label="trigger">
          <span className="inline-flex items-center gap-1">
            <TriggerIcon size={12} className="text-steel" aria-hidden="true" />
            {pipeline.trigger}
          </span>
        </Item>
        <Item label="branch">
          <span className="inline-flex items-center gap-1">
            <GitBranch size={12} className="text-steel" aria-hidden="true" />
            {branchOfRef(pipeline.gitRef)}
          </span>
        </Item>
        <Item label="commit">
          <span className="inline-flex items-center gap-1">
            <GitCommitHorizontal size={12} className="text-steel" aria-hidden="true" />
            {shortSha(pipeline.commitSha)}
          </span>
        </Item>
        <Item label="runner">
          {runner ? (
            <Link
              to={workspacePath(slug, `runners/${runner.id}`)}
              className="inline-flex items-center gap-1 text-link hover:underline"
            >
              <Server size={12} aria-hidden="true" />
              {runner.name}
            </Link>
          ) : (
            '—'
          )}
        </Item>
        <Item label="image">{job.plan.image}</Item>
        <Item label="attempt">{job.attempt}</Item>
        <Item label="queued">{formatDuration(job.queuedAt, job.startedAt)}</Item>
        <Item label="ran">
          {job.startedAt ? formatDuration(job.startedAt, job.finishedAt) : '—'}
        </Item>
        <Item label="stage">{job.stage.replaceAll('_', ' ')}</Item>
      </div>
    </div>
  );
}
