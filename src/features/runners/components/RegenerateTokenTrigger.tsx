import { useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import type { Runner } from '../../../types/runner';
import { useRegenerateRunnerToken } from '../hooks/useRunners';
import { TokenRevealDialog } from './TokenRevealDialog';

interface RegenerateTokenTriggerProps {
  runner: Runner | null;
  onClose: () => void;
}

/** Confirm, then mint and reveal a new token for an existing runner. */
export function RegenerateTokenTrigger({ runner, onClose }: RegenerateTokenTriggerProps) {
  const [token, setToken] = useState<string | null>(null);
  const regenerate = useRegenerateRunnerToken();

  if (!runner && !token) return null;

  return (
    <>
      <Dialog
        open={runner !== null}
        onClose={onClose}
        title={runner ? `Regenerate token for ${runner.name}?` : ''}
        description="The runner's current token stops working immediately and it is disconnected until it reconnects with the new one."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              size="sm"
              isLoading={regenerate.isPending}
              onClick={() =>
                runner &&
                regenerate.mutate(runner.id, {
                  onSuccess: (newToken) => {
                    setToken(newToken);
                    onClose();
                  },
                })
              }
            >
              Regenerate
            </Button>
          </>
        }
      />

      <TokenRevealDialog
        open={token !== null}
        onClose={() => setToken(null)}
        title="New token issued"
        description="Copy this token into the runner's RUNNER_TOKEN environment variable now — it will not be shown again."
        token={token}
      />
    </>
  );
}
