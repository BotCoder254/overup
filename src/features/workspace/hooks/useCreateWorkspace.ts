import { useMutation, useQueryClient } from '@tanstack/react-query';
import { HTTPError } from 'ky';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { ME_QUERY_KEY } from '../../auth/hooks/useAuth';
import type { Me } from '../../../types/user';
import { createWorkspace } from '../api/workspaceApi';

export function useCreateWorkspace() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  return useMutation({
    mutationFn: createWorkspace,
    onSuccess: (workspace) => {
      // Update the cache before navigating so the /w/:slug guard already
      // sees the workspace and doesn't bounce back to onboarding.
      queryClient.setQueryData<Me | null>(ME_QUERY_KEY, (prev) =>
        prev
          ? {
              ...prev,
              onboarded: true,
              workspace: { id: workspace.id, name: workspace.name, slug: workspace.slug },
            }
          : prev,
      );
      navigate(`/w/${workspace.slug}`, { replace: true });
    },
    onError: async (error) => {
      if (error instanceof HTTPError) {
        const { status } = error.response;
        if (status === 409) {
          // Server says a workspace already exists — refresh and let the
          // guards route to it.
          await queryClient.invalidateQueries({ queryKey: ME_QUERY_KEY });
          toast.error('You already have a workspace.');
          return;
        }
        if (status === 422) {
          const body = await error.response.json().catch(() => null);
          const message =
            (body as { error?: { message?: string } } | null)?.error?.message ??
            'Please check the form and try again.';
          toast.error(message);
          return;
        }
        if (status === 429) {
          toast.error('Too many attempts — please wait a moment and try again.');
          return;
        }
      }
      toast.error('Something went wrong. Please try again.');
    },
  });
}
