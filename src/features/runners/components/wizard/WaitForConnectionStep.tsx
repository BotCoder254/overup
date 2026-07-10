import { useQuery } from '@tanstack/react-query';
import { CheckCircle2 } from 'lucide-react';
import { useEffect } from 'react';
import { Button } from '../../../../components/ui/Button';
import { Spinner } from '../../../../components/ui/Spinner';
import { getRunnerDetail } from '../../api/runnersApi';
import { runnerKey, useWorkspaceId } from '../../hooks/useRunners';

interface WaitForConnectionStepProps {
  runnerId: string;
  onDone: () => void;
}

/** Polls unconditionally — the runner is expected to read `offline` while this step is active. */
export function WaitForConnectionStep({ runnerId, onDone }: WaitForConnectionStepProps) {
  const workspaceId = useWorkspaceId();
  const detail = useQuery({
    queryKey: runnerKey(workspaceId ?? '', runnerId),
    queryFn: () => getRunnerDetail(workspaceId!, runnerId),
    enabled: Boolean(workspaceId),
    refetchInterval: (query) => (query.state.data?.status === 'offline' ? 2000 : false),
  });

  const connected = detail.data && detail.data.status !== 'offline';

  useEffect(() => {
    if (connected) onDone();
    // Only fire once, right when connectivity is first observed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected]);

  return (
    <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
      {connected ? (
        <>
          <CheckCircle2 size={40} className="text-primary" aria-hidden="true" />
          <p className="text-sm font-medium text-charcoal">Runner connected.</p>
        </>
      ) : (
        <>
          <Spinner className="h-8 w-8 text-steel" />
          <p className="text-sm text-steel">Waiting for the runner to connect…</p>
        </>
      )}
      {!connected && (
        <Button size="sm" variant="ghost" onClick={onDone}>
          Close and check later
        </Button>
      )}
    </div>
  );
}
