import type { LucideIcon } from 'lucide-react';
import {
  Activity,
  Boxes,
  FolderGit2,
  KeyRound,
  LayoutDashboard,
  Layers,
  ListChecks,
  Lock,
  Package,
  ScrollText,
  Server,
  Settings,
  Workflow,
} from 'lucide-react';

export interface NavItem {
  label: string;
  /** URL segment under /w/:slug — '' is the dashboard index route. */
  segment: string;
  icon: LucideIcon;
  /** Short description of the feature, shown on its placeholder page. */
  description: string;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

/**
 * Single source of truth for workspace navigation: the sidebar, the router,
 * the placeholder pages, and the command palette are all generated from it.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: 'General',
    items: [
      {
        label: 'Dashboard',
        segment: '',
        icon: LayoutDashboard,
        description:
          'Your workspace at a glance — recent runs, pipeline health, and runner status will land here.',
      },
      {
        label: 'Activity',
        segment: 'activity',
        icon: Activity,
        description:
          'A live feed of everything happening in this workspace — pushes, runs, deploys, and configuration changes.',
      },
    ],
  },
  {
    label: 'Build',
    items: [
      {
        label: 'Repositories',
        segment: 'repositories',
        icon: FolderGit2,
        description:
          'Connect GitHub repositories to the workspace and keep their branches, workflows, and webhooks in sync.',
      },
      {
        label: 'Workflows',
        segment: 'workflows',
        icon: Workflow,
        description:
          'Discover, validate, and edit the automation workflows defined in your repositories.',
      },
      {
        label: 'Pipelines',
        segment: 'pipelines',
        icon: Layers,
        description:
          'Every pipeline execution across the workspace — status, duration, triggers, and full run history.',
      },
      {
        label: 'Jobs',
        segment: 'jobs',
        icon: ListChecks,
        description:
          'Individual jobs inside pipeline runs, with live step output and per-job timing.',
      },
    ],
  },
  {
    label: 'Infrastructure',
    items: [
      {
        label: 'Runners',
        segment: 'runners',
        icon: Server,
        description:
          'Register self-hosted runners, watch their health, and control which pipelines they pick up.',
      },
      {
        label: 'Artifacts',
        segment: 'artifacts',
        icon: Package,
        description:
          'Build outputs produced by pipeline runs — browse, download, and manage retention.',
      },
      {
        label: 'Environments',
        segment: 'environments',
        icon: Boxes,
        description:
          'Deployment targets with protection rules, required reviewers, and environment-scoped configuration.',
      },
    ],
  },
  {
    label: 'Configuration',
    items: [
      {
        label: 'Secrets',
        segment: 'secrets',
        icon: Lock,
        description:
          'Encrypted key-value secrets scoped to the workspace, injected into pipeline runs at execution time.',
      },
      {
        label: 'API Keys',
        segment: 'api-keys',
        icon: KeyRound,
        description:
          'Scoped tokens for automating the overup API from scripts, runners, and external systems.',
      },
      {
        label: 'Logs',
        segment: 'logs',
        icon: ScrollText,
        description:
          'The workspace audit trail — who changed what, when, and from where.',
      },
      {
        label: 'Settings',
        segment: 'settings',
        icon: Settings,
        description:
          'Workspace profile, members and roles, and danger-zone controls.',
      },
    ],
  },
];

/** Flattened list for the router and command palette. */
export const NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((group) => group.items);

export function workspacePath(slug: string, segment = ''): string {
  return segment ? `/w/${slug}/${segment}` : `/w/${slug}`;
}
