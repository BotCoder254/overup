import { Navigate, createBrowserRouter } from 'react-router-dom';
import { AppShell } from '../components/layout/AppShell';
import { PlaceholderPage } from '../components/layout/PlaceholderPage';
import { CallbackPage } from '../features/auth/pages/CallbackPage';
import { OnboardingPage } from '../features/auth/pages/OnboardingPage';
import { SignInPage } from '../features/auth/pages/SignInPage';
import { useMe } from '../features/auth/hooks/useAuth';
import { DashboardPage } from '../features/dashboard/pages/DashboardPage';
import { RepositoriesPage } from '../features/repositories/pages/RepositoriesPage';
import { RepositoryDetailPage } from '../features/repositories/pages/RepositoryDetailPage';
import { WorkflowsPage } from '../features/workflows/pages/WorkflowsPage';
import { WorkflowDetailPage } from '../features/workflows/pages/WorkflowDetailPage';
import { PipelinesPage } from '../features/pipelines/pages/PipelinesPage';
import { PipelineDetailPage } from '../features/pipelines/pages/PipelineDetailPage';
import { JobDetailPage } from '../features/pipelines/pages/JobDetailPage';
import { JobQueuePage } from '../features/jobs/pages/JobQueuePage';
import { ArtifactsPage } from '../features/artifacts/pages/ArtifactsPage';
import { ArtifactDetailPage } from '../features/artifacts/pages/ArtifactDetailPage';
import { RunnersPage } from '../features/runners/pages/RunnersPage';
import { RunnerDetailPage } from '../features/runners/pages/RunnerDetailPage';
import { NAV_ITEMS } from './navigation';
import { ProtectedRoute } from './guards/ProtectedRoute';
import { PublicOnlyRoute } from './guards/PublicOnlyRoute';
import { WorkspaceRoute } from './guards/WorkspaceRoute';

/** Segments with real pages; everything else still renders a placeholder. */
const IMPLEMENTED_SEGMENTS = new Set([
  'repositories',
  'workflows',
  'pipelines',
  'jobs',
  'artifacts',
  'runners',
]);

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
        children: [
          {
            // Layout route: the shell mounts once; navigation swaps only
            // the content rendered in its canvas via <Outlet/>.
            path: '/w/:slug',
            element: <AppShell />,
            children: [
              { index: true, element: <DashboardPage /> },
              { path: 'repositories', element: <RepositoriesPage /> },
              { path: 'repositories/:repoId', element: <RepositoryDetailPage /> },
              { path: 'workflows', element: <WorkflowsPage /> },
              { path: 'workflows/:workflowId', element: <WorkflowDetailPage /> },
              { path: 'pipelines', element: <PipelinesPage /> },
              { path: 'pipelines/:pipelineId', element: <PipelineDetailPage /> },
              { path: 'pipelines/:pipelineId/jobs/:jobId', element: <JobDetailPage /> },
              { path: 'jobs', element: <JobQueuePage /> },
              { path: 'artifacts', element: <ArtifactsPage /> },
              { path: 'artifacts/:artifactId', element: <ArtifactDetailPage /> },
              { path: 'runners', element: <RunnersPage /> },
              { path: 'runners/:runnerId', element: <RunnerDetailPage /> },
              ...NAV_ITEMS.filter(
                (item) => item.segment !== '' && !IMPLEMENTED_SEGMENTS.has(item.segment),
              ).map((item) => ({
                path: item.segment,
                element: <PlaceholderPage item={item} />,
              })),
              { path: '*', element: <Navigate to="." replace /> },
            ],
          },
        ],
      },
      { path: '/dashboard', element: <DashboardRedirect /> },
    ],
  },
  { path: '*', element: <Navigate to="/" replace /> },
]);
