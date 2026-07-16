import { CheckCircle2, FileSearch } from 'lucide-react';
import { Link } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import type { DetectedRequirement } from '../../../types/requirements';
import type { EnvironmentsRequirements } from '../../../types/environment';

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

interface DetectedEnvironmentsCardProps {
  slug: string;
  requirements: EnvironmentsRequirements | undefined;
  onCreate: (name: string) => void;
}

/**
 * Every environment name bound by workflow YAML, grouped by repository:
 * existing environments link to their detail page for editing, missing ones
 * get a one-click Create (through the ordinary RBAC'd endpoint — YAML never
 * mints resources by itself). Renders nothing only when no `environment:`
 * bindings were detected at all.
 */
export function DetectedEnvironmentsCard({
  slug,
  requirements,
  onCreate,
}: DetectedEnvironmentsCardProps) {
  const detected = requirements?.environments ?? [];
  if (detected.length === 0) return null;

  const missing = detected.filter((entry) => !entry.configured).length;

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
                {missing} environment{missing === 1 ? '' : 's'} referenced in workflow YAML{' '}
                {missing === 1 ? "doesn't" : "don't"} exist yet.
              </span>{' '}
              Jobs still run, but without environment secrets. Based on the latest workflow
              sync.
            </p>
          </div>
        )}
        <ul className="divide-y divide-steel/10">
          {groupByRepository(detected).map((group) => (
            <li key={group.repositoryName} className="py-2">
              <h3 className="mb-1 truncate text-xs font-medium uppercase tracking-wider text-steel">
                {group.repositoryName}
              </h3>
              <ul className="space-y-1">
                {group.items.map((entry) => (
                  <li key={entry.name} className="flex items-center gap-2.5">
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-mono text-sm text-charcoal">
                        {entry.name}
                      </span>
                      <span className="block truncate text-xs text-steel">
                        {referenceHint(entry)}
                      </span>
                    </span>
                    {entry.configured && entry.configuredId ? (
                      <Link
                        to={workspacePath(slug, `environments/${entry.configuredId}`)}
                        className="shrink-0"
                      >
                        <Badge variant="success">
                          <CheckCircle2 size={10} aria-hidden="true" />
                          Configured
                        </Badge>
                      </Link>
                    ) : (
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={() => onCreate(entry.name)}
                      >
                        Create
                      </Button>
                    )}
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </CardBody>
    </Card>
  );
}
