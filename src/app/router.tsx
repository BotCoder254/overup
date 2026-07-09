import { Navigate, createBrowserRouter } from 'react-router-dom';
import { CallbackPage } from '../features/auth/pages/CallbackPage';
import { OnboardingPage } from '../features/auth/pages/OnboardingPage';
import { SignInPage } from '../features/auth/pages/SignInPage';
import { useMe } from '../features/auth/hooks/useAuth';
import { DashboardPage } from '../features/dashboard/pages/DashboardPage';
import { ProtectedRoute } from './guards/ProtectedRoute';
import { PublicOnlyRoute } from './guards/PublicOnlyRoute';
import { WorkspaceRoute } from './guards/WorkspaceRoute';

/** Legacy /dashboard entry point: forward to the slug-routed workspace. */
function DashboardRedirect() {
  const { data: me } = useMe();
  return <Navigate to={me?.workspace ? `/w/${me.workspace.slug}` : '/onboarding'} replace />;
}

export const router = createBrowserRouter([
  {
    element: <PublicOnlyRoute />,
    children: [{ path: '/', element: <SignInPage /> }],
  },
  {
    // Deliberately unguarded: it resolves the fresh session itself.
    path: '/auth/callback',
    element: <CallbackPage />,
  },
  {
    element: <ProtectedRoute />,
    children: [
      { path: '/onboarding', element: <OnboardingPage /> },
      {
        element: <WorkspaceRoute />,
        children: [{ path: '/w/:slug', element: <DashboardPage /> }],
      },
      { path: '/dashboard', element: <DashboardRedirect /> },
    ],
  },
  { path: '*', element: <Navigate to="/" replace /> },
]);
