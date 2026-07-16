import { AlertTriangle, GitBranch, Squirrel } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { EmptyState } from '../../../components/ui/EmptyState';
import type { DashboardRange } from '../../../types/dashboard';
import { PipelinesTable } from '../../pipelines/components/PipelinesTable';
import { useRunners } from '../../runners/hooks/useRunners';
import { ActivityChart } from '../components/ActivityChart';
import { ActivityPanel } from '../components/ActivityPanel';
import { KpiStrip } from '../components/KpiStrip';
import { PagerControls } from '../components/PagerControls';
import { RunnerHealthPanel } from '../components/RunnerHealthPanel';
import { SuccessRateChart } from '../components/SuccessRateChart';
import { useDashboardActivity, useDashboardRecentPipelines, useDashboardSummary } from '../hooks/useDashboard';
import { useWorkspaceStream } from '../hooks/useWorkspaceStream';

const RANGES: { value: DashboardRange; label: string }[] = [
  { value: '24h', label: '24h' },
  { value: '7d', label: '7d' },
  { value: '30d', label: '30d' },
];

export function DashboardPage() {
  const { slug = '' } = useParams<{ slug: string }>();
  const [range, setRange] = useState<DashboardRange>('24h');
  const { connected } = useWorkspaceStream();

  const summary = useDashboardSummary(range, connected);
  const activity = useDashboardActivity(range, connected);
  const recentPipelines = useDashboardRecentPipelines(connected);
  const runners = useRunners(connected);

  // A query with `retry: false` (see lib/queryClient.ts) settles into
  // isError on the very first failed fetch — but only treat it as a hard
  // failure when there's genuinely nothing to show. A background refetch
  // failure that leaves stale-but-valid data in place should keep rendering
  // the dashboard, not flip the whole page to an error screen.
  const summaryFailed = summary.isError && !summary.data;
  const runnersFailed = runners.isError && !runners.data;
  const activityFailed = activity.isError && !activity.data;
  const recentPipelinesFailed = recentPipelines.isError && !recentPipelines.data;
  const criticalError = summaryFailed || runnersFailed;

  const isEmptyWorkspace =
    !summary.isLoading &&
    !runners.isLoading &&
    (summary.data?.pipelinesTotal ?? 0) === 0 &&
    (runners.data?.length ?? 0) === 0;

  // Flatten the keyset pages for the full-width recent-pipelines table,
  // then page through them five at a time — the next keyset page is fetched
  // on demand when the reader steps past what's already loaded.
  const PIPELINES_PAGE_SIZE = 5;
  const pipelines = (recentPipelines.data?.pages ?? []).flatMap((page) => page.pipelines);
  const [pipelinePage, setPipelinePage] = useState(0);
  const {
    hasNextPage: pipelinesHasNext,
    isFetchingNextPage: pipelinesFetchingNext,
    fetchNextPage: fetchNextPipelines,
  } = recentPipelines;

  const pipelineStart = pipelinePage * PIPELINES_PAGE_SIZE;
  const visiblePipelines = pipelines.slice(pipelineStart, pipelineStart + PIPELINES_PAGE_SIZE);
  const moreLoaded = pipelineStart + PIPELINES_PAGE_SIZE < pipelines.length;

  // Clamp when live updates shrink the loaded list.
  useEffect(() => {
    const maxPage = Math.max(0, Math.ceil(pipelines.length / PIPELINES_PAGE_SIZE) - 1);
    if (pipelinePage > maxPage) setPipelinePage(maxPage);
  }, [pipelinePage, pipelines.length]);

  const nextPipelinePage = () => {
    if (moreLoaded) {
      setPipelinePage((current) => current + 1);
      return;
    }
    if (pipelinesHasNext && !pipelinesFetchingNext) {
      void fetchNextPipelines().then((result) => {
        const loaded = (result.data?.pages ?? []).reduce(
          (count, page) => count + page.pipelines.length,
          0,
        );
        if (pipelineStart + PIPELINES_PAGE_SIZE < loaded) {
          setPipelinePage((current) => current + 1);
        }
      });
    }
  };

  return (
    <>
      <PageHeader
        title="Dashboard"
        description="Your workspace at a glance — recent runs, pipeline health, and runner status."
        actions={
          <div className="flex items-center gap-1 rounded border border-steel/20 bg-canvas p-0.5">
            {RANGES.map((option) => (
              <Button
                key={option.value}
                size="sm"
                variant={range === option.value ? 'primary' : 'ghost'}
                className="h-7 px-2.5 text-xs"
                onClick={() => setRange(option.value)}
              >
                {option.label}
              </Button>
            ))}
          </div>
        }
      />

      {criticalError ? (
        <EmptyState
          icon={AlertTriangle}
          title="Couldn't load your dashboard"
          description="Something went wrong fetching your workspace's pipelines and runners. Check your connection and try again."
          className="min-h-[50vh] border-0 bg-transparent"
          action={
            <Button
              size="sm"
              onClick={() => {
                void summary.refetch();
                void runners.refetch();
              }}
            >
              Try again
            </Button>
          }
        />
      ) : isEmptyWorkspace ? (
        <EmptyState
          icon={Squirrel}
          title="Your workspace is ready"
          description="Connect a repository and register a runner to start building — pipeline runs, runners, and artifacts will appear here."
          className="min-h-[50vh] border-0 bg-transparent"
        />
      ) : (
        <>
          <KpiStrip summary={summary.data} loading={summary.isLoading} error={summaryFailed} />

          <section className="mb-6">
            <h2 className="mb-3 text-sm font-semibold text-charcoal">Recent pipelines</h2>
            {recentPipelines.isLoading ? (
              <div className="h-40 animate-pulse rounded border border-steel/20 bg-canvas" />
            ) : recentPipelinesFailed ? (
              <EmptyState
                icon={AlertTriangle}
                title="Couldn't load pipelines"
                description="Something went wrong fetching recent pipeline runs."
                className="min-h-0 border-0 bg-transparent py-10"
                action={
                  <Button size="sm" variant="secondary" onClick={() => void recentPipelines.refetch()}>
                    Try again
                  </Button>
                }
              />
            ) : pipelines.length === 0 ? (
              <EmptyState
                icon={GitBranch}
                title="No pipeline runs yet"
                description="Dispatch a workflow to see its execution here."
                className="min-h-0 border-0 bg-transparent py-10"
              />
            ) : (
              <>
                <PipelinesTable slug={slug} pipelines={visiblePipelines} />
                {(pipelines.length > PIPELINES_PAGE_SIZE || pipelinesHasNext) && (
                  <PagerControls
                    label={`${pipelineStart + 1}–${Math.min(
                      pipelineStart + PIPELINES_PAGE_SIZE,
                      pipelines.length,
                    )}${pipelinesHasNext ? '' : ` of ${pipelines.length}`}`}
                    hasPrev={pipelinePage > 0}
                    hasNext={moreLoaded || pipelinesHasNext}
                    loadingNext={pipelinesFetchingNext}
                    onPrev={() => setPipelinePage((current) => Math.max(0, current - 1))}
                    onNext={nextPipelinePage}
                  />
                )}
              </>
            )}
          </section>

          <div className="mb-6 grid gap-6 lg:grid-cols-2">
            <section className="min-w-0">
              <h2 className="mb-3 text-sm font-semibold text-charcoal">Runner health</h2>
              <RunnerHealthPanel slug={slug} runners={runners.data ?? []} loading={runners.isLoading} />
            </section>

            <section className="min-w-0">
              <h2 className="mb-3 text-sm font-semibold text-charcoal">Activity</h2>
              <ActivityPanel slug={slug} connected={connected} />
            </section>
          </div>

          <div className="grid gap-6 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <h2 className="text-sm font-semibold text-charcoal">Pipeline activity</h2>
              </CardHeader>
              <CardBody>
                <ActivityChart
                  buckets={activity.data ?? []}
                  range={range}
                  loading={activity.isLoading}
                  error={activityFailed}
                />
              </CardBody>
            </Card>

            <Card>
              <CardHeader>
                <h2 className="text-sm font-semibold text-charcoal">Success rate</h2>
              </CardHeader>
              <CardBody>
                <SuccessRateChart summary={summary.data} loading={summary.isLoading} error={summaryFailed} />
              </CardBody>
            </Card>
          </div>
        </>
      )}
    </>
  );
}
