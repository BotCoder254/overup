import { FileSearch, Variable } from 'lucide-react';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import type { DetectedRequirement } from '../../../types/requirements';
import type { SecretsRequirements } from '../../../types/secret';

/** `repo · path`, plus how many further references exist beyond the first. */
function referenceHint(requirement: DetectedRequirement): string {
  const first = requirement.references[0];
  if (!first) return '';
  const extra = requirement.referenceCount - 1;
  return `${first.repositoryName} · ${first.workflowPath}${
    extra > 0 ? ` · +${extra} more` : ''
  }`;
}

/**
 * When every reference comes from one repository, "Add" pre-scopes the new
 * secret to it; mixed references leave the scope choice to the user.
 */
function soleRepository(
  requirement: DetectedRequirement,
): { id: string; name: string } | undefined {
  const [first] = requirement.references;
  if (!first) return undefined;
  const shared = requirement.references.every(
    (ref) => ref.repositoryId === first.repositoryId,
  );
  return shared ? { id: first.repositoryId, name: first.repositoryName } : undefined;
}

interface DetectedRequirementsCardProps {
  requirements: SecretsRequirements | undefined;
  onAdd: (name: string, presetRepository?: { id: string; name: string }) => void;
}

/**
 * Workflow-declared requirements detected at sync time: secret names the
 * YAML references with nothing configured to satisfy them (one-click Add),
 * plus `${{ vars.* }}` references shown for visibility. Renders nothing
 * while loading or when there is nothing to report — the rail stays quiet.
 */
export function DetectedRequirementsCard({
  requirements,
  onAdd,
}: DetectedRequirementsCardProps) {
  const missing = requirements?.secrets ?? [];
  const vars = requirements?.vars ?? [];
  if (missing.length === 0 && vars.length === 0) return null;

  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Detected requirements</h2>
      </CardHeader>
      <CardBody>
        {missing.length > 0 && (
          <>
            <div className="mb-2 flex items-start gap-2 rounded border border-steel/20 bg-surface p-3">
              <FileSearch size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
              <p className="text-xs text-charcoal">
                <span className="font-medium">
                  {missing.length} workflow-referenced secret
                  {missing.length === 1 ? ' isn’t' : 's aren’t'} configured yet.
                </span>{' '}
                Jobs that reference them run with empty values until they exist. Based on the
                latest workflow sync.
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
                    onClick={() => onAdd(requirement.name, soleRepository(requirement))}
                  >
                    Add
                  </Button>
                </li>
              ))}
            </ul>
          </>
        )}
        {vars.length > 0 && (
          <div className={missing.length > 0 ? 'mt-3' : undefined}>
            <h3 className="text-xs font-medium uppercase tracking-wider text-steel">
              Variables referenced in YAML
            </h3>
            <ul className="divide-y divide-steel/10">
              {vars.map((requirement) => (
                <li key={requirement.name} className="flex items-start gap-2.5 py-2">
                  <Variable
                    size={16}
                    aria-hidden="true"
                    className="mt-0.5 shrink-0 text-steel"
                  />
                  <span className="min-w-0">
                    <span className="block truncate font-mono text-sm text-charcoal">
                      {requirement.name}
                    </span>
                    <span className="block truncate text-xs text-steel">
                      {referenceHint(requirement)}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
            <p className="mt-1 text-xs text-steel">
              Variables aren’t managed by overup yet — shown for visibility.
            </p>
          </div>
        )}
      </CardBody>
    </Card>
  );
}
