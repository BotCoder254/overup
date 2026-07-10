import { api } from '../../../lib/api';
import type { ActivityBucket, DashboardRange, DashboardSummary } from '../../../types/dashboard';

export async function getDashboardSummary(
  workspaceId: string,
  range: DashboardRange,
): Promise<DashboardSummary> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/dashboard/summary`, { searchParams: { range } })
    .json<{ summary: DashboardSummary }>();
  return body.summary;
}

export async function getDashboardActivity(
  workspaceId: string,
  range: DashboardRange,
): Promise<ActivityBucket[]> {
  const body = await api
    .get(`/api/workspaces/${workspaceId}/dashboard/activity`, { searchParams: { range } })
    .json<{ buckets: ActivityBucket[] }>();
  return body.buckets;
}
