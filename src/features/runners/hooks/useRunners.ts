import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { HOSTED_QUOTA_COPY } from '../lib/provisionCopy';
import {
  bootstrapRunner,
  createHostedRunner,
  type CreateHostedInput,
  disableRunner,
  drainRunner,
  getRunnerDetail,
  getRunners,
  getRunnersList,
  regenerateRunnerToken,
  resumeRunner,
  revokeRunner,
  updateRunner,
} from '../api/runnersApi';

export { useWorkspaceId };

export const runnersKey = (workspaceId: string) => ['workspaces', workspaceId, 'runners'] as const;
export const runnerKey = (workspaceId: string, runnerId: string) =>
  ['workspaces', workspaceId, 'runners', 'detail', runnerId] as const;

/** Fallback poll while the live workspace stream is disconnected. */
const pollWhileDisconnected = 5000;

function describeError(error: unknown, fallback: string): string {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) return 'A runner with this name already exists.';
    if (error.response.status === 422) return 'That value is not valid.';
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
  }
  return fallback;
}

/**
 * Runner list. The workspace stream patches this cache live (connect/
 * disconnect, health, lifecycle changes); polling is only the fallback
 * while the stream is down.
 */
export function useRunners(streamConnected: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: runnersKey(workspaceId ?? ''),
    queryFn: () => getRunners(workspaceId!),
    enabled: Boolean(workspaceId),
    refetchInterval: streamConnected ? false : pollWhileDisconnected,
  });
}

export function useRunnerDetail(runnerId: string | undefined, streamConnected: boolean) {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: runnerKey(workspaceId ?? '', runnerId ?? ''),
    queryFn: () => getRunnerDetail(workspaceId!, runnerId!),
    enabled: Boolean(workspaceId && runnerId),
    refetchInterval: (query) => {
      if (streamConnected) return false;
      const status = query.state.data?.status;
      return status === 'idle' || status === 'busy' ? pollWhileDisconnected : false;
    },
  });
}

/**
 * Hosted-runner capability + remaining quota. Availability changes only on
 * a server redeploy, but the remaining quota moves with every hosted
 * create/revoke — so this refetches on mount (no infinite staleTime) and is
 * invalidated alongside the runner list.
 */
export function useHostedRunnerInfo() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: ['workspaces', workspaceId ?? '', 'runners', 'hosted-info'] as const,
    queryFn: async () => {
      const list = await getRunnersList(workspaceId!);
      return { hostedAvailable: list.hostedAvailable, hostedRemaining: list.hostedRemaining };
    },
    enabled: Boolean(workspaceId),
  });
}

export function useCreateHostedRunner() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateHostedInput) => createHostedRunner(workspaceId!, input),
    onSuccess: () => {
      if (workspaceId) void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
    },
    onError: async (error) => {
      // Safety net behind the wizard's pre-check: the quota can fill between
      // opening the dialog and submitting.
      if (error instanceof HTTPError && error.response.status === 409) {
        try {
          const body = (await error.response.clone().json()) as {
            error?: { message?: string };
          };
          if (body.error?.message === 'hosted_runner_quota') {
            toast.error(HOSTED_QUOTA_COPY);
            return;
          }
        } catch {
          // Fall through to the generic 409 copy.
        }
      }
      toast.error(describeError(error, 'Could not provision the hosted runner.'));
    },
  });
}

export function useBootstrapRunner() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: { name: string; labels: string[] }) => bootstrapRunner(workspaceId!, input),
    onSuccess: () => {
      if (workspaceId) void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not start runner registration.'));
    },
  });
}

export function useUpdateRunner() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: { runnerId: string; name?: string; labels?: string[] }) =>
      updateRunner(workspaceId!, input.runnerId, { name: input.name, labels: input.labels }),
    onSuccess: (_runner, input) => {
      toast.success('Runner updated.');
      if (!workspaceId) return;
      void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
      void queryClient.invalidateQueries({ queryKey: runnerKey(workspaceId, input.runnerId) });
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not update the runner.'));
    },
  });
}

export function useRegenerateRunnerToken() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runnerId: string) => regenerateRunnerToken(workspaceId!, runnerId),
    onSuccess: (_token, runnerId) => {
      toast.success('New token issued — the runner must reconnect with it.');
      if (!workspaceId) return;
      void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
      void queryClient.invalidateQueries({ queryKey: runnerKey(workspaceId, runnerId) });
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not regenerate the token.'));
    },
  });
}

export function useRevokeRunner() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runnerId: string) => revokeRunner(workspaceId!, runnerId),
    onSuccess: () => {
      toast.success('Runner revoked.');
      if (workspaceId) void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
    },
    onError: (error) => {
      toast.error(describeError(error, 'Could not revoke the runner.'));
    },
  });
}

function useLifecycleMutation(
  action: (workspaceId: string, runnerId: string) => Promise<void>,
  successMessage: string,
  errorFallback: string,
) {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (runnerId: string) => action(workspaceId!, runnerId),
    onSuccess: (_data, runnerId) => {
      toast.success(successMessage);
      if (!workspaceId) return;
      void queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) });
      void queryClient.invalidateQueries({ queryKey: runnerKey(workspaceId, runnerId) });
    },
    onError: (error) => {
      toast.error(describeError(error, errorFallback));
    },
  });
}

export function useDrainRunner() {
  return useLifecycleMutation(
    drainRunner,
    'Runner draining — it will finish its current job, then stop taking new work.',
    'Could not drain the runner.',
  );
}

export function useDisableRunner() {
  return useLifecycleMutation(
    disableRunner,
    'Runner disabled — it will not be scheduled new work.',
    'Could not disable the runner.',
  );
}

export function useResumeRunner() {
  return useLifecycleMutation(resumeRunner, 'Runner resumed.', 'Could not resume the runner.');
}
