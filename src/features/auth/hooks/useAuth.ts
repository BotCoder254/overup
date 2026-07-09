import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from 'react-router-dom';
import type { Me } from '../../../types/user';
import { getMe, logout } from '../api/authApi';

export const ME_QUERY_KEY = ['me'] as const;

/** Session bootstrap: resolves the HttpOnly cookie to a profile via /api/me. */
export function useMe() {
  return useQuery<Me | null>({
    queryKey: ME_QUERY_KEY,
    queryFn: getMe,
  });
}

export function useLogout() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  return useMutation({
    mutationFn: logout,
    onSettled: () => {
      // Whatever the server said, drop all client state and start over.
      queryClient.clear();
      queryClient.setQueryData(ME_QUERY_KEY, null);
      navigate('/', { replace: true });
    },
  });
}
