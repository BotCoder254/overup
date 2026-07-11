import { useQueries } from '@tanstack/react-query';
import { CheckCircle2, CircleAlert } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '../../../../components/ui/Button';
import { Spinner } from '../../../../components/ui/Spinner';
import { getRunnerDetail } from '../../api/runnersApi';
import { runnerKey, useWorkspaceId } from '../../hooks/useRunners';
import { PROVISION_FAILURE_COPY } from '../../lib/provisionCopy';
import type { Runner } from '../../../../types/runner';

/** Give a cold image pull a real chance before calling the wait stuck. */
const WAIT_DEADLINE_MS = 4 * 60 * 1000;

interface WaitForConnectionStepProps {
  /** One id for self-hosted; hosted batches poll every instance. */
  runnerIds: string[];
  /** Progress copy while polling, e.g. "Provisioning the hosted runner…". */
  message?: string;
  onDone: () => void;
  /** Hosted mode: discard the failed runner(s) and restart from the details step. */
  onRetry?: () => void;
}

type InstanceState = 'waiting' | 'connected' | 'failed';

interface InstanceView {
  runnerId: string;
  runner: Runner | undefined;
  state: InstanceState;
  failureCategory: string | null;
}

/**
 * Polls each runner until it connects. Hosted provisioning happens in a
 * backend background task, so this step also owns the failure surface:
 * `provisionError` on a polled row means that attempt failed, and a
 * ~4-minute deadline catches the silent "provisioned but never connected"
 * case (typically a wrong RUNNER_PROVISIONER_OVERUP_URL). Multi-instance
 * batches settle independently — done when ALL connect, failed when ANY
 * fails.
 */
export function WaitForConnectionStep({
  runnerIds,
  message,
  onDone,
  onRetry,
}: WaitForConnectionStepProps) {
  const workspaceId = useWorkspaceId();
  const [timedOut, setTimedOut] = useState(false);
  // Bumped by "Keep waiting" to restart the deadline clock.
  const [waitEpoch, setWaitEpoch] = useState(0);
  const timedOutRef = useRef(false);
  timedOutRef.current = timedOut;

  const details = useQueries({
    queries: runnerIds.map((runnerId) => ({
      queryKey: runnerKey(workspaceId ?? '', runnerId),
      queryFn: () => getRunnerDetail(workspaceId!, runnerId),
      enabled: Boolean(workspaceId),
      refetchInterval: (query: { state: { data?: Runner } }) => {
        const data = query.state.data;
        const settled =
          timedOutRef.current ||
          (data && (data.status !== 'offline' || data.provisionError || data.revoked));
        return settled ? false : 2000;
      },
    })),
  });

  const instances: InstanceView[] = runnerIds.map((runnerId, index) => {
    const runner = details[index]?.data;
    const connected = Boolean(runner && runner.status !== 'offline');
    const failureCategory = connected
      ? null
      : (runner?.provisionError ?? (runner?.revoked ? 'revoked' : null));
    return {
      runnerId,
      runner,
      state: connected ? 'connected' : failureCategory ? 'failed' : 'waiting',
      failureCategory,
    };
  });

  const allConnected =
    instances.length > 0 && instances.every((instance) => instance.state === 'connected');
  const failedInstances = instances.filter((instance) => instance.state === 'failed');
  const anyFailed = failedInstances.length > 0;
  const multi = runnerIds.length > 1;

  useEffect(() => {
    if (allConnected) onDone();
    // Only fire once, right when full connectivity is first observed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allConnected]);

  useEffect(() => {
    if (allConnected || anyFailed) return;
    const timer = window.setTimeout(() => setTimedOut(true), WAIT_DEADLINE_MS);
    return () => window.clearTimeout(timer);
  }, [allConnected, anyFailed, waitEpoch]);

  const keepWaiting = () => {
    setTimedOut(false);
    setWaitEpoch((epoch) => epoch + 1);
    for (const detail of details) void detail.refetch();
  };

  const instanceList = multi && (
    <ul className="w-full max-w-sm space-y-1.5 text-left">
      {instances.map((instance) => (
        <li
          key={instance.runnerId}
          className="flex items-center gap-2 rounded border border-steel/20 px-3 py-2"
        >
          {instance.state === 'connected' && (
            <CheckCircle2 size={15} className="shrink-0 text-primary" aria-hidden="true" />
          )}
          {instance.state === 'failed' && (
            <CircleAlert size={15} className="shrink-0 text-danger" aria-hidden="true" />
          )}
          {instance.state === 'waiting' && <Spinner className="h-3.5 w-3.5 shrink-0 text-steel" />}
          <span className="truncate text-sm text-charcoal">
            {instance.runner?.name ?? 'Runner'}
          </span>
          <span className="ml-auto text-xs text-steel">
            {instance.state === 'connected'
              ? 'Connected'
              : instance.state === 'failed'
                ? 'Failed'
                : 'Provisioning…'}
          </span>
        </li>
      ))}
    </ul>
  );

  if (allConnected) {
    return (
      <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
        <CheckCircle2 size={40} className="text-primary" aria-hidden="true" />
        <p className="text-sm font-medium text-charcoal">
          {multi ? 'All runners connected.' : 'Runner connected.'}
        </p>
      </div>
    );
  }

  if (anyFailed) {
    const firstFailure = failedInstances[0].failureCategory!;
    return (
      <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
        <CircleAlert size={40} className="text-danger" aria-hidden="true" />
        <p className="text-sm font-medium text-charcoal">
          {multi ? 'Provisioning failed for some runners.' : 'Provisioning failed.'}
        </p>
        {instanceList}
        <p className="max-w-sm text-sm text-steel">
          {PROVISION_FAILURE_COPY[firstFailure] ??
            'The runner could not be provisioned. Try again.'}
        </p>
        <div className="flex flex-col gap-2 sm:flex-row">
          {onRetry && (
            <Button size="sm" onClick={onRetry}>
              {multi ? 'Discard these runners and start over' : 'Try again'}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onDone}>
            Close
          </Button>
        </div>
      </div>
    );
  }

  if (timedOut) {
    return (
      <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
        <CircleAlert size={40} className="text-steel" aria-hidden="true" />
        <p className="text-sm font-medium text-charcoal">
          {multi ? 'Still waiting for the runners.' : 'Still waiting for the runner.'}
        </p>
        {instanceList}
        <p className="max-w-sm text-sm text-steel">
          The runner has not connected yet. For hosted runners, verify the server&apos;s
          RUNNER_PROVISIONER_OVERUP_URL — the container must be able to reach the API on that
          URL. For self-hosted runners, check the install command output.
        </p>
        <div className="flex flex-col gap-2 sm:flex-row">
          <Button size="sm" variant="secondary" onClick={keepWaiting}>
            Keep waiting
          </Button>
          <Button size="sm" variant="ghost" onClick={onDone}>
            Close and check later
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
      <Spinner className="h-8 w-8 text-steel" />
      <p className="text-sm text-steel">{message ?? 'Waiting for the runner to connect…'}</p>
      {instanceList}
      <Button size="sm" variant="ghost" onClick={onDone}>
        Close and check later
      </Button>
    </div>
  );
}
