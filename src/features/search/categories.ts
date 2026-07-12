import type { LucideIcon } from 'lucide-react';
import {
  Activity,
  Boxes,
  FolderGit2,
  Layers,
  Lock,
  Package,
  Server,
  Workflow,
} from 'lucide-react';
import type { SearchCategory, SearchResult } from '../../types/search';
import { workspacePath } from '../../app/navigation';

/** Display metadata per category — icons match the sidebar nav choices. */
export const SEARCH_CATEGORY_META: Record<
  SearchCategory,
  { label: string; plural: string; icon: LucideIcon }
> = {
  repository: { label: 'Repository', plural: 'Repositories', icon: FolderGit2 },
  workflow: { label: 'Workflow', plural: 'Workflows', icon: Workflow },
  pipeline: { label: 'Pipeline', plural: 'Pipelines', icon: Layers },
  runner: { label: 'Runner', plural: 'Runners', icon: Server },
  artifact: { label: 'Artifact', plural: 'Artifacts', icon: Package },
  environment: { label: 'Environment', plural: 'Environments', icon: Boxes },
  secret: { label: 'Secret', plural: 'Secrets', icon: Lock },
  activity: { label: 'Activity', plural: 'Activity', icon: Activity },
};

/** Detail-page href for a hit. Activity has no detail page — it links to the feed. */
export function searchResultPath(slug: string, result: SearchResult): string {
  switch (result.category) {
    case 'repository':
      return workspacePath(slug, `repositories/${result.entityId}`);
    case 'workflow':
      return workspacePath(slug, `workflows/${result.entityId}`);
    case 'pipeline':
      return workspacePath(slug, `pipelines/${result.entityId}`);
    case 'runner':
      return workspacePath(slug, `runners/${result.entityId}`);
    case 'artifact':
      return workspacePath(slug, `artifacts/${result.entityId}`);
    case 'environment':
      return workspacePath(slug, `environments/${result.entityId}`);
    case 'secret':
      return workspacePath(slug, `secrets/${result.entityId}`);
    case 'activity':
      return workspacePath(slug, 'activity');
  }
}
