import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  getToolchains,
  installToolchain,
  uninstallToolchain,
} from '../api/toolchainsApi';

export { useWorkspaceId };

export const toolchainsKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'toolchains'] as const;

/** Map the common failure codes to friendly copy. */
async function describeError(error: unknown, fallback: string): Promise<string> {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) {
      try {
        const body = (await error.response.clone().json()) as {
          error?: { message?: string };
        };
        if (body.error?.message === 'hosted_runner_unavailable') {
          return 'Hosted runners are unavailable — start the provisioner (RUNNER_PROVISIONER=docker) and try again.';
        }
        if (body.error?.message === 'toolchain_in_use') {
          return 'This is the default job image and cannot be uninstalled.';
        }
      } catch {
        // fall through
      }
      return 'The request conflicts with the current state.';
    }
    if (error.response.status === 403) {
      return 'You do not have permission to manage toolchains.';
    }
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
  }
  return fallback;
}

export function useToolchains() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: toolchainsKey(workspaceId ?? ''),
    queryFn: () => getToolchains(workspaceId!),
    enabled: Boolean(workspaceId),
    // Poll while any install is in flight so completion appears without a
    // manual refresh; idle otherwise.
    refetchInterval: (query) =>
      query.state.data?.toolchains.some((t) => t.installStatus === 'pending') ? 4000 : false,
  });
}

export function useInstallToolchain() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (key: string) => installToolchain(workspaceId!, key),
    onSuccess: () => {
      toast.success('Installing toolchain — this can take a minute.');
      void queryClient.invalidateQueries({ queryKey: toolchainsKey(workspaceId ?? '') });
    },
    onError: async (error) =>
      toast.error(await describeError(error, 'Could not start the install.')),
  });
}

export function useUninstallToolchain() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (key: string) => uninstallToolchain(workspaceId!, key),
    onSuccess: () => {
      toast.success('Toolchain uninstalled.');
      void queryClient.invalidateQueries({ queryKey: toolchainsKey(workspaceId ?? '') });
    },
    onError: async (error) =>
      toast.error(await describeError(error, 'Could not uninstall the toolchain.')),
  });
}
