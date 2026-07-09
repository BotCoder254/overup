import { Navigate, Outlet, useParams } from 'react-router-dom';
import { useMe } from '../../features/auth/hooks/useAuth';

/**
 * Gate for /w/:slug routes. Users without a workspace go to onboarding;
 * a mismatched slug is canonicalized to the user's own workspace. The slug
 * is a routing value only — the server authorizes by immutable UUIDs.
 */
export function WorkspaceRoute() {
  const { data: me } = useMe();
  const { slug } = useParams();

  if (!me?.workspace) return <Navigate to="/onboarding" replace />;
  if (slug !== me.workspace.slug) return <Navigate to={`/w/${me.workspace.slug}`} replace />;
  return <Outlet />;
}
