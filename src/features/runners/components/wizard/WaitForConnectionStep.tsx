import { useQuery } from '@tanstack/react-query';
import { CheckCircle2, CircleAlert } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '../../../../components/ui/Button';
import { Spinner } from '../../../../components/ui/Spinner';
import { getRunnerDetail } from '../../api/runnersApi';
import { runnerKey, useWorkspaceId } from '../../hooks/useRunners';

/** Give a cold image pull a real chance before calling the wait stuck. */
const WAIT_DEADLINE_MS = 4 * 60 * 1000;

/** Static copy per provisioning failure category (the backend only ever
 *  sends these fixed strings — never raw Docker output). */
const FAILURE_COPY: Record<string, string> = {
  image_pull_failed:
    'The server could not pull the runner image. Check RUNNER_IMAGE and registry access on the server, then try again.',
  container_create_failed:
    'The server could not create the runner container. Check the Docker daemon on the server, then try again.',
  container_start_failed:
    'The runner container failed to start. Check its logs on the server, then try again.',
  provision_timeout:
    'Provisioning did not finish within the 10-minute budget — usually a very slow image pull. Try again.',
  revoked: 'This runner was revoked before it connected.',
};

interface WaitForConnectionStepProps {
  runnerId: string;
  /** Progress copy while polling, e.g. "Provisioning the hosted runner…". */
  message?: string;
  onDone: () => void;
  /** Hosted mode: discard the failed runner and restart from the details step. */
  onRetry?: () => void;
}

/**
 * Polls the runner until it connects. Hosted provisioning happens in a
 * backend background task, so this step also owns the failure surface:
 * `provisionError` on the polled row means the attempt failed, and a
 * ~4-minute deadline catches the silent "provisioned but never connected"
 * case (typically a wrong RUNNER_PROVISIONER_OVERUP_URL).
 */
export function WaitForConnectionStep({
  runnerId,
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

  const detail = useQuery({
    queryKey: runnerKey(workspaceId ?? '', runnerId),
    queryFn: () => getRunnerDetail(workspaceId!, runnerId),
    enabled: Boolean(workspaceId),
    refetchInterval: (query) => {
      const data = query.state.data;
      const settled =
        timedOutRef.current ||
        (data && (data.status !== 'offline' || data.provisionError || data.revoked));
      return settled ? false : 2000;
    },
  });

  const connected = Boolean(detail.data && detail.data.status !== 'offline');
  const failureCategory = connected
    ? null
    : (detail.data?.provisionError ?? (detail.data?.revoked ? 'revoked' : null));
  const failed = failureCategory !== null;

  useEffect(() => {
    if (connected) onDone();
    // Only fire once, right when connectivity is first observed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected]);

  useEffect(() => {
    if (connected || failed) return;
    const timer = window.setTimeout(() => setTimedOut(true), WAIT_DEADLINE_MS);
    return () => window.clearTimeout(timer);
  }, [connected, failed, waitEpoch]);

  const keepWaiting = () => {
    setTimedOut(false);
    setWaitEpoch((epoch) => epoch + 1);
    void detail.refetch();
  };

  if (connected) {
    return (
      <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
        <CheckCircle2 size={40} className="text-primary" aria-hidden="true" />
        <p className="text-sm font-medium text-charcoal">Runner connected.</p>
      </div>
    );
  }

  if (failed) {
    return (
      <div className="mt-4 flex flex-col items-center gap-4 py-6 text-center">
        <CircleAlert size={40} className="text-danger" aria-hidden="true" />
        <p className="text-sm font-medium text-charcoal">Provisioning failed.</p>
        <p className="max-w-sm text-sm text-steel">
          {FAILURE_COPY[failureCategory] ?? 'The runner could not be provisioned. Try again.'}
        </p>
        <div className="flex flex-col gap-2 sm:flex-row">
          {onRetry && (
            <Button size="sm" onClick={onRetry}>
              Try again
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
        <p className="text-sm font-medium text-charcoal">Still waiting for the runner.</p>
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
      <Button size="sm" variant="ghost" onClick={onDone}>
        Close and check later
      </Button>
    </div>
  );
}
