import { Badge } from '../../../../components/ui/Badge';
import type { PipelineJob } from '../../../../types/pipeline';
import { Field, FieldList, PanelSection } from './fields';

/**
 * Resolved runtime environment of the selected job. Env values arrive from
 * the API already masked when their names look confidential — names stay
 * visible, sensitive contents never reach the browser.
 */
export function EnvironmentPanel({ job }: { job: PipelineJob }) {
  const env = Object.entries(job.plan.env ?? {});

  return (
    <div>
      <PanelSection title="Execution target">
        <FieldList>
          <Field label="Container image">
            <span className="font-mono text-xs">{job.plan.image}</span>
          </Field>
          <Field label="Requested labels">
            {job.runsOn.length > 0 ? (
              <span className="flex flex-wrap gap-1">
                {job.runsOn.map((label) => (
                  <Badge key={label} variant="outline">
                    {label}
                  </Badge>
                ))}
              </span>
            ) : (
              <span className="text-steel">any runner</span>
            )}
          </Field>
          <Field label="Workspace">
            <span className="font-mono text-xs">/workspace (isolated, deleted after run)</span>
          </Field>
        </FieldList>
      </PanelSection>

      <PanelSection title={`Environment variables (${env.length})`}>
        {env.length === 0 ? (
          <p className="text-sm text-steel">No environment variables are defined.</p>
        ) : (
          <div className="overflow-hidden rounded border border-steel/20">
            {env.map(([key, value]) => (
              <div
                key={key}
                className="flex items-baseline gap-2 border-b border-steel/10 px-2 py-1 text-xs last:border-b-0"
              >
                <span className="w-40 shrink-0 truncate font-mono text-charcoal">{key}</span>
                <span className="min-w-0 break-all font-mono text-steel">{value}</span>
              </div>
            ))}
          </div>
        )}
      </PanelSection>

      {job.plan.notices.length > 0 && (
        <PanelSection title="Notices">
          <ul className="list-inside list-disc space-y-1 text-xs text-steel">
            {job.plan.notices.map((notice) => (
              <li key={notice}>{notice}</li>
            ))}
          </ul>
        </PanelSection>
      )}
    </div>
  );
}
