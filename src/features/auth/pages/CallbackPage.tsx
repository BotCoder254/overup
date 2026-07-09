import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { toast } from 'sonner';
import { Spinner } from '../../../components/ui/Spinner';
import { getMe } from '../api/authApi';
import { AuthSplitLayout } from '../components/AuthSplitLayout';
import { ME_QUERY_KEY } from '../hooks/useAuth';

/**
 * Landing spot after the backend finishes the OAuth flow. The URL carries at
 * most a non-sensitive failure marker (`?error=auth_failed`); the session
 * itself lives in the HttpOnly cookie, so we simply refetch /api/me and
 * route by workspace membership.
 */
export function CallbackPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const queryClient = useQueryClient();
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;

    if (searchParams.get('error')) {
      toast.error('GitHub sign-in failed. Please try again.');
      navigate('/', { replace: true });
      return;
    }

    void (async () => {
      const me = await queryClient
        .fetchQuery({ queryKey: ME_QUERY_KEY, queryFn: getMe })
        .catch(() => null);

      if (!me) {
        toast.error('Sign-in could not be completed. Please try again.');
        navigate('/', { replace: true });
        return;
      }

      navigate(me.workspace ? `/w/${me.workspace.slug}` : '/onboarding', {
        replace: true,
      });
    })();
  }, [navigate, queryClient, searchParams]);

  return (
    <AuthSplitLayout>
      <div className="flex animate-fade-in items-center gap-4">
        <Spinner />
        <p className="text-steel">Completing sign-in…</p>
      </div>
    </AuthSplitLayout>
  );
}
