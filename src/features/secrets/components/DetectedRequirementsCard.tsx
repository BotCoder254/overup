import { CheckCircle2, FileSearch, Variable } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import type { DetectedRequirement } from '../../../types/requirements';
import type { SecretsRequirements } from '../../../types/secret';
import { isConfigurableSecretName, secretNameProblem } from '../lib/secretNameRules';

/** First referencing workflow, plus how many further references exist. */
function referenceHint(requirement: DetectedRequirement): string {
  const first = requirement.references[0];
  if (!first) return '';
  const extra = requirement.referenceCount - 1;
  return `${first.workflowPath}${extra > 0 ? ` · +${extra} more` : ''}`;
}

/** Fold entries into repository groups, preserving the server's sort order. */
function groupByRepository(
  entries: DetectedRequirement[],
): Array<{ repositoryName: string; items: DetectedRequirement[] }> {
  const groups: Array<{ repositoryName: string; items: DetectedRequirement[] }> = [];
  for (const entry of entries) {
    const last = groups[groups.length - 1];
    if (last && last.repositoryName === entry.repositoryName) {
      last.items.push(entry);
    } else {
      groups.push({ repositoryName: entry.repositoryName, items: [entry] });
    }
  }
  return groups;
}

interface DetectedRequirementsCardProps {
  slug: string;
  requirements: SecretsRequirements | undefined;
  onAdd: (name: string, presetRepository?: { id: string; name: string }) => void;
}

/**
 * Everything workflow YAML was detected referencing at sync time, grouped by
 * repository: configured secrets link to their detail page for editing,
 * missing ones get a one-click Add, reserved names explain why they can
 * never be overup secrets, and `${{ vars.* }}` references are listed for
 * visibility. Renders nothing only when no references were detected at all.
 */
export function DetectedRequirementsCard({
  slug,
  requirements,
  onAdd,
}: DetectedRequirementsCardProps) {
  const secrets = requirements?.secrets ?? [];
  const vars = requirements?.vars ?? [];
  if (secrets.length === 0 && vars.length === 0) return null;

  const missing = secrets.filter(
    (entry) => !entry.configured && isConfigurableSecretName(entry.name),
  ).length;

  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Detected in workflows</h2>
      </CardHeader>
      <CardBody>
        {missing > 0 && (
          <div className="mb-2 flex items-start gap-2 rounded border border-steel/20 bg-surface p-3">
            <FileSearch size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
            <p className="text-xs text-charcoal">
              <span className="font-medium">
                {missing} workflow-referenced secret{missing === 1 ? ' isn’t' : 's aren’t'}{' '}
                configured yet.
              </span>{' '}
              Jobs that reference them run with empty values until they exist. Based on the
              latest workflow sync.
            </p>
          </div>
        )}
        {secrets.length > 0 && (
          <ul className="divide-y divide-steel/10">
            {groupByRepository(secrets).map((group) => (
              <li key={group.repositoryName} className="py-2">
                <h3 className="mb-1 truncate text-xs font-medium uppercase tracking-wider text-steel">
                  {group.repositoryName}
                </h3>
                <ul className="space-y-1">
                  {group.items.map((entry) => {
                    const reserved = !entry.configured && !isConfigurableSecretName(entry.name);
                    return (
                      <li
                        key={entry.name}
                        className="flex items-center gap-2.5"
                        title={reserved ? secretNameProblem(entry.name) : undefined}
                      >
                        <span className="min-w-0 flex-1">
                          <span
                            className={`block truncate font-mono text-sm ${
                              reserved ? 'text-steel' : 'text-charcoal'
                            }`}
                          >
                            {entry.name}
                          </span>
                          <span className="block truncate text-xs text-steel">
                            {reserved
                              ? 'Reserved name — rename it in the workflow YAML.'
                              : referenceHint(entry)}
                          </span>
                        </span>
                        {entry.configured && entry.configuredId ? (
                          <Link
                            to={workspacePath(slug, `secrets/${entry.configuredId}`)}
                            className="shrink-0"
                          >
                            <Badge variant="success">
                              <CheckCircle2 size={10} aria-hidden="true" />
                              Configured
                            </Badge>
                          </Link>
                        ) : reserved ? null : (
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() =>
                              onAdd(entry.name, {
                                id: entry.repositoryId,
                                name: entry.repositoryName,
                              })
                            }
                          >
                            Add
                          </Button>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ul>
        )}
        {vars.length > 0 && (
          <div className={secrets.length > 0 ? 'mt-3' : undefined}>
            <h3 className="text-xs font-medium uppercase tracking-wider text-steel">
              Variables referenced in YAML
            </h3>
            <ul className="divide-y divide-steel/10">
              {vars.map((entry) => (
                <li
                  key={`${entry.repositoryId}:${entry.name}`}
                  className="flex items-start gap-2.5 py-2"
                >
                  <Variable size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
                  <span className="min-w-0">
                    <span className="block truncate font-mono text-sm text-charcoal">
                      {entry.name}
                    </span>
                    <span className="block truncate text-xs text-steel">
                      {entry.repositoryName} · {referenceHint(entry)}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
            <p className="mt-1 text-xs text-steel">
              Variables aren’t managed by overup yet — these resolve to empty values at run
              time.
            </p>
          </div>
        )}
      </CardBody>
    </Card>
  );
}
