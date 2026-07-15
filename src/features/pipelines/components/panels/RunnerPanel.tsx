import { formatDistanceToNow } from 'date-fns';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../../app/navigation';
import { Badge } from '../../../../components/ui/Badge';
import type { Runner } from '../../../../types/runner';
import { HealthPanel } from '../../../runners/components/HealthPanel';
import { RunnerStatusBadge } from '../../../runners/components/RunnerStatusBadge';
import { Field, FieldList, PanelSection } from './fields';

/** Known architecture label values runners advertise. */
const ARCH_LABELS = new Set(['x64', 'x86_64', 'amd64', 'arm64', 'aarch64', 'arm']);

function archFromLabels(labels: string[]): string | null {
  return labels.find((label) => ARCH_LABELS.has(label.toLowerCase())) ?? null;
}

/**
 * The assigned runner's identity and live host telemetry (heartbeat-fed;
 * the job detail poll keeps it fresh while the job runs). Only sanitized
 * fields — never tokens, never raw host paths.
 */
export function RunnerPanel({ slug, runner }: { slug: string; runner: Runner | null }) {
  if (!runner) {
    return (
      <PanelSection title="Runner">
        <p className="text-sm text-steel">
          No runner assigned yet — the job is still waiting in the queue.
        </p>
      </PanelSection>
    );
  }

  return (
    <div>
      <PanelSection title="Runner">
        <FieldList>
          <Field label="Name">
            <span className="inline-flex items-center gap-2">
              <Link
                to={workspacePath(slug, `runners/${runner.id}`)}
                className="text-link hover:underline"
              >
                {runner.name}
              </Link>
              <RunnerStatusBadge status={runner.status} draining={runner.draining} />
            </span>
          </Field>
          <Field label="Identifier">
            <span className="font-mono text-xs">{runner.id}</span>
          </Field>
          <Field label="Labels">
            {runner.labels.length > 0 ? (
              <span className="flex flex-wrap gap-1">
                {runner.labels.map((label) => (
                  <Badge key={label} variant="outline">
                    {label}
                  </Badge>
                ))}
              </span>
            ) : (
              '—'
            )}
          </Field>
          <Field label="Version">{runner.version ?? '—'}</Field>
          <Field label="OS">{runner.lastHealth?.os ?? '—'}</Field>
          <Field label="Arch">{archFromLabels(runner.labels) ?? '—'}</Field>
          <Field label="Docker">{runner.lastHealth?.dockerVersion ?? '—'}</Field>
          <Field label="Last heartbeat">
            {runner.lastSeenAt
              ? formatDistanceToNow(new Date(runner.lastSeenAt), { addSuffix: true })
              : '—'}
          </Field>
        </FieldList>
      </PanelSection>

      <PanelSection title="Host utilization">
        <HealthPanel health={runner.lastHealth} />
      </PanelSection>
    </div>
  );
}
