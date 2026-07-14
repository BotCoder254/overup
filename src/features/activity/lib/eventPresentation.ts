import type { ActivityCategory, ActivityEvent, ActivitySeverity } from '../../../types/activity';

/**
 * Presentation layer for ledger entries: human-readable sentences, badge
 * variants, and deep links. Metadata is read defensively — every value is
 * untrusted JSON, so anything that isn't the expected primitive is simply
 * omitted and the copy still reads correctly.
 */

const str = (metadata: Record<string, unknown>, key: string): string | null =>
  typeof metadata[key] === 'string' ? (metadata[key] as string) : null;

const num = (metadata: Record<string, unknown>, key: string): number | null =>
  typeof metadata[key] === 'number' ? (metadata[key] as number) : null;

export interface EventDescription {
  /** Verb phrase completing "{actor} …", e.g. "created secret". */
  verb: string;
  /** Mono-rendered subject label (secret name, repo full name, …). */
  subjectLabel: string | null;
}

export function describeEvent(event: ActivityEvent): EventDescription {
  const m = event.metadata;
  const name = str(m, 'name');
  const number = num(m, 'number');
  switch (event.action) {
    case 'workspace.created':
      return { verb: 'created the workspace', subjectLabel: name };
    case 'workspace.updated':
      return { verb: 'renamed the workspace to', subjectLabel: name };
    case 'workspace.logo_updated':
      return { verb: 'updated the workspace logo', subjectLabel: null };
    case 'workspace.logo_removed':
      return { verb: 'removed the workspace logo', subjectLabel: null };
    case 'user.profile_updated':
      return { verb: 'updated their profile', subjectLabel: null };
    case 'session.revoked':
      return { verb: 'revoked a session', subjectLabel: null };
    case 'sessions.revoked_all': {
      const count = num(m, 'count');
      return {
        verb:
          count !== null
            ? `signed out ${count} other session${count === 1 ? '' : 's'}`
            : 'signed out their other sessions',
        subjectLabel: null,
      };
    }
    case 'installation.linked':
      return { verb: 'linked the GitHub App installation', subjectLabel: str(m, 'accountLogin') };
    case 'installation.unlinked':
      return { verb: 'unlinked the GitHub App installation', subjectLabel: null };
    case 'repository.imported':
      return { verb: 'imported repository', subjectLabel: str(m, 'fullName') };
    case 'repository.removed':
      return { verb: 'removed repository', subjectLabel: str(m, 'fullName') };
    case 'repository.synced': {
      const workflows = num(m, 'workflows');
      return {
        verb:
          workflows !== null
            ? `synced repository metadata (${workflows} workflow${workflows === 1 ? '' : 's'})`
            : 'synced repository metadata',
        subjectLabel: null,
      };
    }
    case 'pipeline.created':
      return {
        verb: number !== null ? `started pipeline #${number}` : 'started a pipeline',
        subjectLabel: str(m, 'workflow'),
      };
    case 'pipeline.completed': {
      const conclusion = str(m, 'conclusion') ?? 'unknown';
      return {
        verb:
          number !== null
            ? `pipeline #${number} completed — ${conclusion}`
            : `pipeline completed — ${conclusion}`,
        subjectLabel: null,
      };
    }
    case 'pipeline.cancelled':
      return {
        verb: number !== null ? `cancelled pipeline #${number}` : 'cancelled a pipeline',
        subjectLabel: null,
      };
    case 'job.cancelled':
      return {
        verb: number !== null ? `cancelled a job in pipeline #${number}` : 'cancelled a job',
        subjectLabel: str(m, 'job'),
      };
    case 'runner.created':
      return { verb: 'registered runner', subjectLabel: name };
    case 'runner.revoked':
      return { verb: 'revoked runner', subjectLabel: name };
    case 'runner.updated':
      return { verb: 'updated runner', subjectLabel: name };
    case 'runner.token_regenerated':
      return { verb: 'regenerated the token for runner', subjectLabel: name };
    case 'runner.drained':
      return { verb: 'drained runner', subjectLabel: name };
    case 'runner.disabled':
      return { verb: 'disabled runner', subjectLabel: name };
    case 'runner.resumed':
      return { verb: 'resumed runner', subjectLabel: name };
    case 'runner.provisioned':
      return { verb: 'provisioned hosted runner', subjectLabel: name };
    case 'runner.provision_failed':
      return { verb: 'failed to provision hosted runner', subjectLabel: name };
    case 'secret.created':
      return { verb: 'created secret', subjectLabel: name };
    case 'secret.updated': {
      const field = str(m, 'field');
      return {
        verb: field === 'value' ? 'rotated the value of secret' : 'updated secret',
        subjectLabel: name,
      };
    }
    case 'secret.deleted':
      return { verb: 'deleted secret', subjectLabel: name };
    case 'environment.created':
      return { verb: 'created environment', subjectLabel: name };
    case 'environment.updated':
      return { verb: 'updated environment', subjectLabel: name };
    case 'environment.deleted': {
      const cascaded = num(m, 'deletedSecrets');
      return {
        verb:
          cascaded !== null && cascaded > 0
            ? `deleted environment (and ${cascaded} scoped secret${cascaded === 1 ? '' : 's'})`
            : 'deleted environment',
        subjectLabel: name,
      };
    }
    case 'artifact.uploaded':
      return { verb: 'uploaded artifact', subjectLabel: name };
    case 'artifact.downloaded':
      return { verb: 'downloaded artifact', subjectLabel: name };
    case 'artifact.deleted':
      return { verb: 'deleted artifact', subjectLabel: name };
    case 'artifact.retention_updated':
      return { verb: 'updated the artifact retention policies', subjectLabel: null };
    default:
      // Unknown action (newer backend): show it verbatim rather than hiding it.
      return { verb: event.action, subjectLabel: null };
  }
}

export function severityBadgeVariant(
  severity: ActivitySeverity,
): 'info' | 'success' | 'neutral' | 'danger' {
  switch (severity) {
    case 'success':
      return 'success';
    case 'warning':
      return 'neutral';
    case 'danger':
      return 'danger';
    default:
      return 'info';
  }
}

export const CATEGORY_LABELS: Record<ActivityCategory, string> = {
  workspace: 'Workspace',
  integration: 'Integration',
  repository: 'Repository',
  pipeline: 'Pipeline',
  runner: 'Runner',
  artifact: 'Artifact',
  secret: 'Secret',
  environment: 'Environment',
};

export const CATEGORY_ORDER: ActivityCategory[] = [
  'pipeline',
  'runner',
  'repository',
  'artifact',
  'secret',
  'environment',
  'integration',
  'workspace',
];

/** Actions selectable in the filter bar, grouped for the <optgroup> UI. */
export const ACTION_GROUPS: { category: ActivityCategory; actions: string[] }[] = [
  {
    category: 'pipeline',
    actions: ['pipeline.created', 'pipeline.completed', 'pipeline.cancelled', 'job.cancelled'],
  },
  {
    category: 'runner',
    actions: [
      'runner.created',
      'runner.provisioned',
      'runner.provision_failed',
      'runner.updated',
      'runner.drained',
      'runner.disabled',
      'runner.resumed',
      'runner.token_regenerated',
      'runner.revoked',
    ],
  },
  {
    category: 'repository',
    actions: ['repository.imported', 'repository.synced', 'repository.removed'],
  },
  {
    category: 'artifact',
    actions: [
      'artifact.uploaded',
      'artifact.downloaded',
      'artifact.deleted',
      'artifact.retention_updated',
    ],
  },
  { category: 'secret', actions: ['secret.created', 'secret.updated', 'secret.deleted'] },
  {
    category: 'environment',
    actions: ['environment.created', 'environment.updated', 'environment.deleted'],
  },
  { category: 'integration', actions: ['installation.linked', 'installation.unlinked'] },
  {
    category: 'workspace',
    actions: [
      'workspace.created',
      'workspace.updated',
      'workspace.logo_updated',
      'workspace.logo_removed',
      'user.profile_updated',
      'session.revoked',
      'sessions.revoked_all',
    ],
  },
];

/** Actions whose subject row no longer exists — no page to link to. */
const TOMBSTONE_ACTIONS = new Set([
  'repository.removed',
  'runner.revoked',
  'secret.deleted',
  'environment.deleted',
  'artifact.deleted',
  'installation.unlinked',
]);

/**
 * Deep link from a ledger entry to its subject's page, or null when there
 * is nowhere sensible to go (workspace/installation subjects, deleted rows).
 */
export function eventLink(event: ActivityEvent, slug: string): string | null {
  if (!event.subjectId || TOMBSTONE_ACTIONS.has(event.action)) return null;
  const base = `/w/${slug}`;
  switch (event.subjectType) {
    case 'repository':
      return `${base}/repositories/${event.subjectId}`;
    case 'pipeline':
      return `${base}/pipelines/${event.subjectId}`;
    case 'pipeline_job': {
      const pipelineId = str(event.metadata, 'pipeline');
      return pipelineId ? `${base}/pipelines/${pipelineId}/jobs/${event.subjectId}` : null;
    }
    case 'runner':
      return `${base}/runners/${event.subjectId}`;
    case 'secret':
      return `${base}/secrets/${event.subjectId}`;
    case 'environment':
      return `${base}/environments/${event.subjectId}`;
    case 'artifact':
      return `${base}/artifacts/${event.subjectId}`;
    default:
      return null;
  }
}
