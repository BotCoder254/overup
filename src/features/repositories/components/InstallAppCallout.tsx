import { FolderGit2 } from 'lucide-react';
import { Button } from '../../../components/ui/Button';
import { EmptyState } from '../../../components/ui/EmptyState';

/**
 * Shown until a GitHub App installation is linked. The install button is a
 * full-page navigation to GitHub; the app's Setup URL brings the browser
 * back here with the installation linked.
 */
export function InstallAppCallout({ installUrl }: { installUrl: string | undefined }) {
  return (
    <EmptyState
      icon={FolderGit2}
      title="Connect your GitHub repositories"
      description="Install the overup GitHub App on your account or organization to import repositories. Access is scoped to the repositories you choose and uses short-lived installation tokens — no personal access tokens."
      className="min-h-[50vh] border-0 bg-transparent"
      action={
        installUrl ? (
          <Button
            onClick={() => {
              window.location.assign(installUrl);
            }}
          >
            Install GitHub App
          </Button>
        ) : undefined
      }
    />
  );
}
