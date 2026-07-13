import {
  AlertTriangle,
  Boxes,
  FolderGit2,
  KeyRound,
  Package,
  Server,
  Workflow,
  Zap,
  type LucideIcon,
} from 'lucide-react';
import type {
  NotificationCategory,
  NotificationLink,
  NotificationSeverity,
} from '../../../types/notification';

/** Category → icon, one glyph per operational surface. */
export function categoryIcon(category: NotificationCategory): LucideIcon {
  switch (category) {
    case 'pipeline':
      return Zap;
    case 'runner':
      return Server;
    case 'repository':
      return FolderGit2;
    case 'workflow':
      return Workflow;
    case 'artifact':
      return Package;
    case 'security':
      return KeyRound;
    case 'environment':
      return Boxes;
    case 'system':
    default:
      return AlertTriangle;
  }
}

/**
 * Severity → design-token text color. Strictly the solid palette: success
 * rides primary (the Badge convention), warnings stay charcoal, errors and
 * critical use danger.
 */
export function severityTextClass(severity: NotificationSeverity): string {
  switch (severity) {
    case 'success':
      return 'text-primary';
    case 'warning':
      return 'text-charcoal';
    case 'error':
    case 'critical':
      return 'text-danger';
    case 'info':
    default:
      return 'text-steel';
  }
}

/** Severity → left-accent border color for cards (palette tokens only). */
export function severityAccentClass(severity: NotificationSeverity): string {
  switch (severity) {
    case 'success':
      return 'border-l-primary';
    case 'warning':
      return 'border-l-steel';
    case 'error':
      return 'border-l-danger/70';
    case 'critical':
      return 'border-l-danger';
    case 'info':
    default:
      return 'border-l-steel/40';
  }
}

/** Severity → Badge variant (the Activity Feed convention). */
export function severityBadgeVariant(
  severity: NotificationSeverity,
): 'info' | 'success' | 'neutral' | 'danger' {
  switch (severity) {
    case 'success':
      return 'success';
    case 'warning':
      return 'neutral';
    case 'error':
    case 'critical':
      return 'danger';
    default:
      return 'info';
  }
}

export const SEVERITY_LABELS: Record<NotificationSeverity, string> = {
  info: 'Info',
  success: 'Success',
  warning: 'Warning',
  error: 'Error',
  critical: 'Critical',
};

export const CATEGORY_LABELS: Record<NotificationCategory, string> = {
  pipeline: 'Pipelines',
  runner: 'Runners',
  repository: 'Repositories',
  workflow: 'Workflows',
  artifact: 'Artifacts',
  security: 'Security',
  environment: 'Environments',
  system: 'System',
};

/**
 * Resolve a server-built link object to an in-app route. Kinds are an
 * allow-list mirroring the backend's — anything unknown lands on the
 * dashboard rather than interpolating untrusted values into a path.
 */
export function resolveNotificationLink(slug: string, link: NotificationLink): string {
  const base = `/w/${slug}`;
  switch (link.kind) {
    case 'pipeline':
      return link.pipelineId ? `${base}/pipelines/${link.pipelineId}` : `${base}/pipelines`;
    case 'repository':
      return link.repositoryId
        ? `${base}/repositories/${link.repositoryId}`
        : `${base}/repositories`;
    case 'repositories':
      return `${base}/repositories`;
    case 'workflow':
      return link.workflowId ? `${base}/workflows/${link.workflowId}` : `${base}/workflows`;
    case 'artifact':
      return link.artifactId ? `${base}/artifacts/${link.artifactId}` : `${base}/artifacts`;
    case 'secret':
      return link.secretId ? `${base}/secrets/${link.secretId}` : `${base}/secrets`;
    case 'secrets':
      return `${base}/secrets`;
    case 'environment':
      return link.environmentId
        ? `${base}/environments/${link.environmentId}`
        : `${base}/environments`;
    case 'environments':
      return `${base}/environments`;
    case 'runners':
      return `${base}/runners`;
    default:
      // Unknown kind: land on the workspace dashboard (the index route)
      // rather than interpolating anything untrusted into a path.
      return base;
  }
}
