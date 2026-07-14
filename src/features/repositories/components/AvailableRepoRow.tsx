import { Lock } from 'lucide-react';
import { Avatar } from '../../../components/ui/Avatar';
import { Badge } from '../../../components/ui/Badge';
import { Button } from '../../../components/ui/Button';
import type { AvailableRepo } from '../../../types/repository';

interface AvailableRepoRowProps {
  repo: AvailableRepo;
  onImport: (repo: AvailableRepo) => void;
  importing: boolean;
}

/** One importable repository exposed by a linked installation. */
export function AvailableRepoRow({ repo, onImport, importing }: AvailableRepoRowProps) {
  return (
    <li className="flex items-center gap-3 px-4 py-3">
      <Avatar size="sm" login={repo.owner} avatarUrl={repo.ownerAvatarUrl} />
      <div className="min-w-0">
        <p className="truncate text-sm text-charcoal">
          <span className="text-steel">{repo.owner}/</span>
          <span className="font-medium">{repo.name}</span>
        </p>
        {repo.description && (
          <p className="mt-0.5 truncate text-xs text-steel">{repo.description}</p>
        )}
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-2">
        {repo.private && (
          <Badge variant="outline">
            <Lock size={10} aria-hidden="true" />
            Private
          </Badge>
        )}
        {repo.language && <Badge variant="neutral">{repo.language}</Badge>}
        {repo.connected ? (
          <Badge variant="primary">Connected</Badge>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            isLoading={importing}
            onClick={() => onImport(repo)}
          >
            Import
          </Button>
        )}
      </div>
    </li>
  );
}
