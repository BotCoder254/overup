import { formatDistanceToNow } from 'date-fns';
import { GitBranch, Lock, Workflow } from 'lucide-react';
import { Link, useParams } from 'react-router-dom';
import { Badge } from '../../../components/ui/Badge';
import type { Repository } from '../../../types/repository';
import { SyncStatusBadge } from './SyncStatusBadge';

/** One connected repository in the workspace grid. The whole card links to its detail page. */
export function RepoCard({ repository }: { repository: Repository }) {
  const { slug } = useParams<{ slug: string }>();

  return (
    <Link
      to={`/w/${slug}/repositories/${repository.id}`}
      className="group flex flex-col gap-3 rounded border border-steel/20 bg-canvas p-4 transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0">
          <p className="truncate text-xs text-steel">{repository.owner}/</p>
          <h3 className="truncate text-sm font-semibold text-charcoal group-hover:text-primary">
            {repository.name}
          </h3>
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {repository.private && (
            <Badge variant="outline">
              <Lock size={10} aria-hidden="true" />
              Private
            </Badge>
          )}
          <SyncStatusBadge status={repository.syncStatus} />
        </div>
      </div>

      {repository.description && (
        <p className="line-clamp-2 text-xs leading-relaxed text-steel">{repository.description}</p>
      )}

      <div className="mt-auto flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-steel">
        <span className="inline-flex items-center gap-1">
          <GitBranch size={12} aria-hidden="true" />
          {repository.defaultBranch}
        </span>
        <span className="inline-flex items-center gap-1">
          <Workflow size={12} aria-hidden="true" />
          {repository.workflowCount} workflow{repository.workflowCount === 1 ? '' : 's'}
        </span>
        {repository.language && <span>{repository.language}</span>}
        {repository.lastSyncedAt && (
          <span className="ml-auto">
            synced {formatDistanceToNow(new Date(repository.lastSyncedAt), { addSuffix: true })}
          </span>
        )}
      </div>
    </Link>
  );
}
