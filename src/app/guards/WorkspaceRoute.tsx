import { Navigate, Outlet, useLocation, useParams } from 'react-router-dom';
import { useMe } from '../../features/auth/hooks/useAuth';

/**
 * Gate for /w/:slug routes. Users without a workspace go to onboarding;
 * a mismatched slug is canonicalized to the user's own workspace, keeping
 * the rest of the path (/w/wrong/runners → /w/right/runners). The slug is
 * a routing value only — the server authorizes by immutable UUIDs.
 */
export function WorkspaceRoute() {
  const { data: me } = useMe();
  const { slug } = useParams();
  const location = useLocation();

  if (!me?.workspace) return <Navigate to="/onboarding" replace />;
  if (slug !== me.workspace.slug) {
    const rest = location.pathname.replace(/^\/w\/[^/]+/, '');
    return <Navigate to={`/w/${me.workspace.slug}${rest}`} replace />;
  }
  return <Outlet />;
}
