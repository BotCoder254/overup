import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { toast } from 'sonner';
import { ME_QUERY_KEY } from '../../auth/hooks/useAuth';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import {
  deleteAccount,
  getWorkspaceLogoUrl,
  listMembers,
  listSessions,
  removeWorkspaceLogo,
  revokeAllSessions,
  revokeSession,
  updateMe,
  updateWorkspace,
  uploadWorkspaceLogo,
  type UpdateMeInput,
} from '../api/settingsApi';

export const sessionsKey = ['sessions'] as const;
export const membersKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'members'] as const;
export const logoKey = (workspaceId: string) =>
  ['workspaces', workspaceId, 'logo-url'] as const;

async function describeError(error: unknown, fallback: string): Promise<string> {
  if (error instanceof HTTPError) {
    if (error.response.status === 409) return 'That action conflicts with the current state.';
    if (error.response.status === 403) return 'You do not have permission for that.';
    if (error.response.status === 413) return 'The image is too large (2 MiB max).';
    if (error.response.status === 429) return 'Too many attempts — please wait a moment.';
    if (error.response.status === 422) {
      try {
        const body = (await error.response.clone().json()) as {
          error?: { message?: string };
        };
        if (body.error?.message) return body.error.message;
      } catch {
        // fall through to the generic copy
      }
      return 'The request was rejected as invalid.';
    }
  }
  return fallback;
}

export function useUpdateMe() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (patch: UpdateMeInput) => updateMe(patch),
    onSuccess: (me) => {
      queryClient.setQueryData(ME_QUERY_KEY, me);
      toast.success('Profile updated.');
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not update your profile.'));
    },
  });
}

export function useDeleteAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (confirmUsername: string) => deleteAccount(confirmUsername),
    onSuccess: () => {
      // Full reset + hard navigation: SPA-only routing would race the now
      // cleared cookie against cached guards.
      queryClient.clear();
      window.location.assign('/');
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not delete the account.'));
    },
  });
}

export function useSessions() {
  return useQuery({
    queryKey: sessionsKey,
    queryFn: listSessions,
    staleTime: 30_000,
  });
}

export function useRevokeSession() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) => revokeSession(sessionId),
    onSuccess: () => {
      toast.success('Session revoked.');
      void queryClient.invalidateQueries({ queryKey: sessionsKey });
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not revoke the session.'));
    },
  });
}

export function useRevokeAllSessions() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: revokeAllSessions,
    onSuccess: (revoked) => {
      toast.success(
        revoked === 0
          ? 'No other sessions to revoke.'
          : `Signed out ${revoked} other session${revoked === 1 ? '' : 's'}.`,
      );
      void queryClient.invalidateQueries({ queryKey: sessionsKey });
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not revoke other sessions.'));
    },
  });
}

export function useUpdateWorkspace() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (patch: { name: string }) => updateWorkspace(workspaceId!, patch),
    onSuccess: () => {
      toast.success('Workspace updated.');
      // The sidebar renders the name from /api/me — refresh it.
      void queryClient.invalidateQueries({ queryKey: ME_QUERY_KEY });
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not update the workspace.'));
    },
  });
}

export function useWorkspaceMembers() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: membersKey(workspaceId ?? ''),
    queryFn: () => listMembers(workspaceId!),
    enabled: Boolean(workspaceId),
  });
}

export function useWorkspaceLogoUrl() {
  const workspaceId = useWorkspaceId();
  return useQuery({
    queryKey: logoKey(workspaceId ?? ''),
    queryFn: () => getWorkspaceLogoUrl(workspaceId!),
    enabled: Boolean(workspaceId),
    // Presigned URLs live 10 minutes; refresh comfortably inside that.
    staleTime: 5 * 60_000,
  });
}

export function useUploadWorkspaceLogo() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (file: File) => uploadWorkspaceLogo(workspaceId!, file),
    onSuccess: (logoUrl) => {
      toast.success('Workspace logo updated.');
      if (workspaceId) queryClient.setQueryData(logoKey(workspaceId), logoUrl);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not upload the logo.'));
    },
  });
}

export function useRemoveWorkspaceLogo() {
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => removeWorkspaceLogo(workspaceId!),
    onSuccess: () => {
      toast.success('Workspace logo removed.');
      if (workspaceId) queryClient.setQueryData(logoKey(workspaceId), null);
    },
    onError: async (error) => {
      toast.error(await describeError(error, 'Could not remove the logo.'));
    },
  });
}
