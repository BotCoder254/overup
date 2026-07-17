import { Square, Wifi, WifiOff } from 'lucide-react';
import { useCallback, useState } from 'react';
import { useParams } from 'react-router-dom';
import { Group, Panel, Separator } from 'react-resizable-panels';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import { useIsDesktop } from '../../../lib/useMediaQuery';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { JobIdentityBar } from '../components/JobIdentityBar';
import { LogViewer } from '../components/LogViewer';
import { PipelineStatusBadge } from '../components/PipelineStatusBadge';
import { StepTimeline } from '../components/StepTimeline';
import { ArtifactsPanel } from '../components/panels/ArtifactsPanel';
import { ContainerPanel } from '../components/panels/ContainerPanel';
import { EnvironmentPanel } from '../components/panels/EnvironmentPanel';
import { JobMetricsPanel } from '../components/panels/JobMetricsPanel';
import { RunnerPanel } from '../components/panels/RunnerPanel';
import { useCancelJob, useJobDetail, usePipelineDetail } from '../hooks/usePipelines';
import { usePipelineStream } from '../hooks/usePipelineStream';

/**
 * The Job Execution workspace: the smallest schedulable unit, observed end
 * to end, in a three-column operational layout. Live state rides the
 * existing per-pipeline WebSocket (job picked out client-side); the
 * job-detail endpoint adds what the socket does not carry — the assigned
 * runner's identity and heartbeat health. Left: the step timeline (live
 * states, exit codes, per-step log collapse + jump). Center: the streaming
 * terminal. Right: runner, container, masked environment, job artifacts,
 * and metrics. Desktop gets a resizable three-panel split; below `lg` it
 * stacks, and the two layouts are exclusive so xterm mounts once.
 */
export function JobDetailPage() {
  const { slug = '', pipelineId, jobId } = useParams<{
    slug: string;
    pipelineId: string;
    jobId: string;
  }>();
  const workspaceId = useWorkspaceId();
  const stream = usePipelineStream(pipelineId);
  const detail = usePipelineDetail(pipelineId, stream.connected);
  const jobDetail = useJobDetail(pipelineId, jobId);
  const cancelJob = useCancelJob();
  const isDesktop = useIsDesktop();
  const [confirmCancel, setConfirmCancel] = useState(false);
  // Log-section visibility shared between the step list and the terminal.
  const [hiddenSections, setHiddenSections] = useState<Set<string>>(new Set());
  const toggleSection = useCallback((key: string) => {
    setHiddenSections((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);
  // "Scroll the terminal to this section" requests from the step list.
  const [jumpRequest, setJumpRequest] = useState<{ key: string; nonce: number } | null>(null);
  const jumpToSection = useCallback((key: string) => {
    setJumpRequest((previous) => ({ key, nonce: (previous?.nonce ?? 0) + 1 }));
  }, []);

  const pipeline = detail.data?.pipeline ?? jobDetail.data?.pipeline;
  // The stream-patched pipeline cache is the live source of truth for the
  // job row; the dedicated payload is the fallback for direct deep links.
  const job =
    detail.data?.jobs.find((candidate) => candidate.id === jobId) ?? jobDetail.data?.job;
  const runner = jobDetail.data?.runner ?? null;
  const jobEvents = (detail.data?.events ?? jobDetail.data?.events ?? []).filter(
    (event) => event.jobId === jobId,
  );

  if (detail.isLoading && jobDetail.isLoading) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }
  if (!pipeline || !job || !workspaceId || !pipelineId || !jobId) {
    return (
      <>
        <PageHeader
          title="Job"
          parent={{ label: 'Pipelines', to: workspacePath(slug, 'pipelines') }}
        />
        <p className="text-sm text-steel">
          This job could not be loaded — it may belong to a pipeline that was removed.
        </p>
      </>
    );
  }

  const isLive = job.status !== 'completed';

  const logView = (
    <LogViewer
      workspaceId={workspaceId}
      pipelineId={pipelineId}
      job={job}
      onWatch={stream.watchJobLogs}
      events={jobEvents}
      hiddenSections={hiddenSections}
      onToggleSection={toggleSection}
      jumpToSection={jumpRequest}
    />
  );
  const stepList = (
    <StepTimeline
      job={job}
      events={jobEvents}
      hiddenSections={hiddenSections}
      onToggleSection={toggleSection}
      onJumpToSection={jumpToSection}
    />
  );
  const operationalPanels = (
    <>
      <RunnerPanel slug={slug} runner={runner} />
      <ContainerPanel job={job} events={jobEvents} />
      <EnvironmentPanel job={job} />
      <PanelBlock title="Artifacts">
        <ArtifactsPanel pipelineId={pipelineId} jobId={job.id} compact />
      </PanelBlock>
      <JobMetricsPanel job={job} />
    </>
  );

  return (
    <>
      <PageHeader
        title={job.name ?? job.key}
        parent={{
          label: `${pipeline.workflowName} #${pipeline.number}`,
          to: workspacePath(slug, `pipelines/${pipeline.id}`),
        }}
        actions={
          <>
            <span
              className="inline-flex items-center"
              title={
                stream.connected
                  ? 'Live: streaming execution updates'
                  : 'Stream disconnected: falling back to polling'
              }
            >
              {stream.connected ? (
                <Wifi size={13} className="text-primary" aria-hidden="true" />
              ) : (
                <WifiOff size={13} className="text-steel" aria-hidden="true" />
              )}
            </span>
            <PipelineStatusBadge status={job.status} conclusion={job.conclusion} />
            {isLive && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setConfirmCancel(true)}
                isLoading={cancelJob.isPending}
              >
                <Square size={13} aria-hidden="true" />
                Cancel job
              </Button>
            )}
          </>
        }
      />

      <JobIdentityBar slug={slug} pipeline={pipeline} job={job} runner={runner} />

      {isDesktop ? (
        <div className="h-[80vh] min-h-[620px] overflow-hidden rounded border border-steel/20 bg-canvas">
          <Group orientation="horizontal" className="h-full">
            <Panel defaultSize="24%" minSize="16%" className="flex flex-col">
              <div className="min-h-0 flex-1 overflow-y-auto p-3">{stepList}</div>
            </Panel>
            <Separator className="w-1 bg-steel/10 transition-colors hover:bg-primary/40" />
            <Panel defaultSize="48%" minSize="30%" className="flex flex-col">
              <div className="min-h-0 flex-1">{logView}</div>
            </Panel>
            <Separator className="w-1 bg-steel/10 transition-colors hover:bg-primary/40" />
            <Panel defaultSize="28%" minSize="20%" className="flex flex-col">
              <div className="min-h-0 flex-1 overflow-y-auto p-3">{operationalPanels}</div>
            </Panel>
          </Group>
        </div>
      ) : (
        // Below lg: no resizable panels — a natural page-scroll stack.
        <div className="space-y-4">
          <div className="rounded border border-steel/20 bg-canvas p-3">{stepList}</div>
          <div className="h-[68vh] min-h-[420px] overflow-hidden rounded border border-steel/20 bg-canvas">
            {logView}
          </div>
          <div className="rounded border border-steel/20 bg-canvas p-3">
            {operationalPanels}
          </div>
        </div>
      )}

      <Dialog
        open={confirmCancel}
        onClose={() => setConfirmCancel(false)}
        title="Cancel this job?"
        description="A queued job stops immediately; a running job is signalled to abort. Jobs that depend on it will be skipped. This cannot be undone."
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setConfirmCancel(false)}>
              Keep running
            </Button>
            <Button
              variant="primary"
              size="sm"
              isLoading={cancelJob.isPending}
              onClick={() => {
                cancelJob.mutate(
                  { pipelineId, jobId: job.id },
                  { onSettled: () => setConfirmCancel(false) },
                );
              }}
            >
              Cancel job
            </Button>
          </>
        }
      />
    </>
  );
}

/** Small titled wrapper matching the PanelSection heading treatment. */
function PanelBlock({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-4">
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-steel">
        {title}
      </h3>
      {children}
    </section>
  );
}
