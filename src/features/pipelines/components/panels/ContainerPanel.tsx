import { Badge } from '../../../../components/ui/Badge';
import type { PipelineJob } from '../../../../types/pipeline';
import { Field, FieldList, PanelSection } from './fields';

/** Container-level execution facts — no host details beyond what debugging
 * needs. The container id appears in the job's system log lines. */
export function ContainerPanel({ job }: { job: PipelineJob }) {
  return (
    <div>
      <PanelSection title="Container">
        <FieldList>
          <Field label="Image">
            <span className="font-mono text-xs">{job.plan.image}</span>
          </Field>
          <Field label="Working directory">
            <span className="font-mono text-xs">/workspace</span>
          </Field>
          <Field label="Mounts">
            <span className="font-mono text-xs">workspace → /workspace (bind, per-job temp)</span>
          </Field>
          <Field label="Network">default bridge, isolated per container</Field>
          <Field label="Steps">{job.plan.steps.length}</Field>
        </FieldList>
      </PanelSection>

      <PanelSection title="Result">
        <FieldList>
          <Field label="Exit code">
            {job.exitCode !== null ? (
              <span className="font-mono text-xs">{job.exitCode}</span>
            ) : (
              '—'
            )}
          </Field>
          <Field label="Error category">
            {job.errorCategory ? (
              <Badge variant="danger">{job.errorCategory.replaceAll('_', ' ')}</Badge>
            ) : (
              '—'
            )}
          </Field>
          <Field label="Cleanup">
            {job.status === 'completed'
              ? 'container removed, workspace deleted'
              : 'runs after the job finishes'}
          </Field>
        </FieldList>
      </PanelSection>

      <PanelSection title={`Steps (${job.plan.steps.length})`}>
        {job.plan.steps.length === 0 ? (
          <p className="text-sm text-steel">This job has no runnable steps.</p>
        ) : (
          <ol className="space-y-1">
            {job.plan.steps.map((step, index) => (
              <li
                key={`${index}-${step.name}`}
                className="rounded border border-steel/20 px-2 py-1.5"
              >
                <div className="text-xs font-medium text-charcoal">
                  {index + 1}. {step.name}
                  <span className="ml-2 font-mono text-[10px] text-steel">{step.shell}</span>
                </div>
                <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-[11px] text-steel">
                  {step.run}
                </pre>
              </li>
            ))}
          </ol>
        )}
      </PanelSection>
    </div>
  );
}
