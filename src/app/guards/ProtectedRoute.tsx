import { Navigate, Outlet } from 'react-router-dom';
import { FullScreenLoader } from '../../components/ui/Spinner';
import { useMe } from '../../features/auth/hooks/useAuth';

/** Only renders children for a live session; otherwise back to sign-in. */
export function ProtectedRoute() {
  const { data: me, isLoading } = useMe();

  if (isLoading) return <FullScreenLoader />;
  if (!me) return <Navigate to="/" replace />;
  return <Outlet />;
}
