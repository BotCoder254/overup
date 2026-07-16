import { FileSearch } from 'lucide-react';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import type { DetectedRequirement } from '../../../types/requirements';
import type { EnvironmentsRequirements } from '../../../types/environment';

/** `repo · path`, plus how many further references exist beyond the first. */
function referenceHint(requirement: DetectedRequirement): string {
  const first = requirement.references[0];
  if (!first) return '';
  const extra = requirement.referenceCount - 1;
  return `${first.repositoryName} · ${first.workflowPath}${
    extra > 0 ? ` · +${extra} more` : ''
  }`;
}

interface DetectedEnvironmentsCardProps {
  requirements: EnvironmentsRequirements | undefined;
  onCreate: (name: string) => void;
}

/**
 * Environment names bound by workflow YAML with no matching environment —
 * detected at sync time and offered as a one-click create (through the
 * ordinary RBAC'd endpoint; YAML never mints resources by itself). Renders
 * nothing while loading or when every reference is satisfied.
 */
export function DetectedEnvironmentsCard({
  requirements,
  onCreate,
}: DetectedEnvironmentsCardProps) {
  const missing = requirements?.environments ?? [];
  if (missing.length === 0) return null;

  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Detected environments</h2>
      </CardHeader>
      <CardBody>
        <div className="mb-2 flex items-start gap-2 rounded border border-steel/20 bg-surface p-3">
          <FileSearch size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
          <p className="text-xs text-charcoal">
            <span className="font-medium">
              {missing.length} environment{missing.length === 1 ? '' : 's'} referenced in
              workflow YAML {missing.length === 1 ? "doesn't" : "don't"} exist yet.
            </span>{' '}
            Jobs still run, but without environment secrets. Based on the latest workflow
            sync.
          </p>
        </div>
        <ul className="divide-y divide-steel/10">
          {missing.map((requirement) => (
            <li key={requirement.name} className="flex items-center gap-2.5 py-2">
              <span className="min-w-0 flex-1">
                <span className="block truncate font-mono text-sm text-charcoal">
                  {requirement.name}
                </span>
                <span className="block truncate text-xs text-steel">
                  {referenceHint(requirement)}
                </span>
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onCreate(requirement.name)}
              >
                Create
              </Button>
            </li>
          ))}
        </ul>
      </CardBody>
    </Card>
  );
}
