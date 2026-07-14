import { format } from 'date-fns';
import { ArrowUpRight } from 'lucide-react';
import { Link } from 'react-router-dom';
import { Avatar } from '../../../../components/ui/Avatar';
import { Badge } from '../../../../components/ui/Badge';
import type { Pipeline, PipelineJob } from '../../../../types/pipeline';
import { branchOfRef, formatDuration, shortSha } from '../../lib/format';
import { Field, FieldList, PanelSection } from './fields';

/** Immutable execution facts for the pipeline and the selected job. */
export function MetadataPanel({
  pipeline,
  job,
  jobHref,
}: {
  pipeline: Pipeline;
  job: PipelineJob | null;
  /** Link into the selected job's dedicated execution workspace. */
  jobHref?: string;
}) {
  return (
    <div>
      <PanelSection title="Pipeline">
        <FieldList>
          <Field label="Identifier">
            <span className="font-mono text-xs">{pipeline.id}</span>
          </Field>
          <Field label="Run number">#{pipeline.number}</Field>
          <Field label="Workflow">
            <span className="font-mono text-xs">{pipeline.workflowPath}</span>
          </Field>
          <Field label="Repository">{pipeline.repoFullName}</Field>
          <Field label="Commit">
            <span className="font-mono text-xs">{shortSha(pipeline.commitSha)}</span>
            {pipeline.commitMessage ? ` — ${pipeline.commitMessage}` : ''}
          </Field>
          <Field label="Author">
            <span className="inline-flex items-center gap-1.5">
              <Avatar
                size="xs"
                login={pipeline.actorLogin ?? pipeline.commitAuthor}
                avatarUrl={pipeline.actorAvatarUrl}
                title={pipeline.actorLogin ?? undefined}
              />
              {pipeline.commitAuthor ?? pipeline.actorLogin ?? '—'}
            </span>
          </Field>
          <Field label="Branch">
            <span className="font-mono text-xs">{branchOfRef(pipeline.gitRef)}</span>
          </Field>
          <Field label="Trigger">
            <Badge variant="outline">{pipeline.trigger}</Badge>
          </Field>
          <Field label="Created">{format(new Date(pipeline.createdAt), 'PPpp')}</Field>
          <Field label="Queue time">
            {formatDuration(pipeline.createdAt, pipeline.startedAt)}
          </Field>
          <Field label="Execution">
            {pipeline.startedAt
              ? formatDuration(pipeline.startedAt, pipeline.finishedAt)
              : '—'}
          </Field>
        </FieldList>
      </PanelSection>

      {pipeline.triggerInputs && Object.keys(pipeline.triggerInputs).length > 0 && (
        <PanelSection title="Inputs">
          <FieldList>
            {Object.entries(pipeline.triggerInputs).map(([name, value]) => (
              <Field key={name} label={name}>
                <span className="font-mono text-xs">{String(value)}</span>
              </Field>
            ))}
          </FieldList>
        </PanelSection>
      )}

      {job && (
        <PanelSection title={`Job · ${job.name ?? job.key}`}>
          <FieldList>
            <Field label="Identifier">
              <span className="font-mono text-xs">{job.id}</span>
            </Field>
            <Field label="Attempt">{job.attempt}</Field>
            <Field label="Stage">
              <span className="font-mono text-xs">{job.stage.replaceAll('_', ' ')}</span>
            </Field>
            <Field label="Runner">
              {job.runnerId ? <span className="font-mono text-xs">{job.runnerId}</span> : '—'}
            </Field>
            <Field label="Queue time">{formatDuration(job.queuedAt, job.startedAt)}</Field>
            <Field label="Execution">
              {job.startedAt ? formatDuration(job.startedAt, job.finishedAt) : '—'}
            </Field>
            <Field label="Depends on">
              {job.needs.length > 0 ? job.needs.join(', ') : 'nothing'}
            </Field>
          </FieldList>
          {jobHref && (
            <Link
              to={jobHref}
              className="mt-2 inline-flex items-center gap-1 text-sm text-link hover:underline"
            >
              Open job workspace
              <ArrowUpRight size={13} aria-hidden="true" />
            </Link>
          )}
        </PanelSection>
      )}
    </div>
  );
}
