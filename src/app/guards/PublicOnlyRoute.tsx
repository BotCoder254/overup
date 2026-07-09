import { Navigate, Outlet } from 'react-router-dom';
import { FullScreenLoader } from '../../components/ui/Spinner';
import { useMe } from '../../features/auth/hooks/useAuth';

/** Keeps signed-in users out of the sign-in flow. */
export function PublicOnlyRoute() {
  const { data: me, isLoading } = useMe();

  if (isLoading) return <FullScreenLoader />;
  if (me) {
    return <Navigate to={me.workspace ? `/w/${me.workspace.slug}` : '/onboarding'} replace />;
  }
  return <Outlet />;
}
