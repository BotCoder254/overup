import { RotateCcw, Square, Wifi, WifiOff } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';
import { Group, Panel, Separator } from 'react-resizable-panels';
import { workspacePath } from '../../../app/navigation';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { Spinner } from '../../../components/ui/Spinner';
import { Tabs } from '../../../components/ui/Tabs';
import { ExecutionTimeline } from '../components/ExecutionTimeline';
import { LogViewer } from '../components/LogViewer';
import { PipelineGraph } from '../components/PipelineGraph';
import { PipelineStatusBadge } from '../components/PipelineStatusBadge';
import { ArtifactsPanel } from '../components/panels/ArtifactsPanel';
import { ContainerPanel } from '../components/panels/ContainerPanel';
import { EnvironmentPanel } from '../components/panels/EnvironmentPanel';
import { MetadataPanel } from '../components/panels/MetadataPanel';
import { PerformancePanel } from '../components/panels/PerformancePanel';
import {
  useCancelPipeline,
  usePipelineDetail,
  useRerunPipeline,
} from '../hooks/usePipelines';
import { usePipelineStream } from '../hooks/usePipelineStream';
import { useWorkspaceId } from '../../repositories/hooks/useRepositories';
import { useIsDesktop } from '../../../lib/useMediaQuery';
import { branchOfRef, shortSha } from '../lib/format';

type TabId = 'logs' | 'environment' | 'artifacts' | 'metadata' | 'container' | 'performance';

/**
 * The execution workspace: a live dependency graph and timeline beside
 * synchronized inspector tabs (resizable split on desktop; a natural
 * vertical stack below `lg`, where the two are conditionally rendered so
 * xterm never mounts twice). Selecting a job anywhere re-targets logs,
 * environment, container, and metadata simultaneously. State streams in
 * over the pipeline WebSocket; polling covers gaps.
 */
export function PipelineDetailPage() {
  const { slug = '', pipelineId } = useParams<{ slug: string; pipelineId: string }>();
  const workspaceId = useWorkspaceId();
  const stream = usePipelineStream(pipelineId);
  const detail = usePipelineDetail(pipelineId, stream.connected);
  const cancel = useCancelPipeline();
  const rerun = useRerunPipeline();
  const isDesktop = useIsDesktop();

  const [tab, setTab] = useState<TabId>('logs');
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [confirmCancel, setConfirmCancel] = useState(false);

  const pipeline = detail.data?.pipeline;
  const jobs = useMemo(() => detail.data?.jobs ?? [], [detail.data]);
  const events = detail.data?.events ?? [];

  // Default selection: the first running job, else the first job.
  useEffect(() => {
    if (jobs.length === 0) return;
    if (selectedKey && jobs.some((job) => job.key === selectedKey)) return;
    const running = jobs.find((job) => job.status === 'in_progress');
    setSelectedKey((running ?? jobs[0]).key);
  }, [jobs, selectedKey]);

  const selectedJob = jobs.find((job) => job.key === selectedKey) ?? null;

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }
  if (detail.isError || !pipeline || !workspaceId || !pipelineId) {
    return (
      <>
        <PageHeader
          title="Pipeline"
          parent={{ label: 'Pipelines', to: workspacePath(slug, 'pipelines') }}
        />
        <p className="text-sm text-steel">
          This pipeline could not be loaded — it may belong to a repository that was removed.
        </p>
      </>
    );
  }

  const isLive = pipeline.status !== 'completed';

  return (
    <>
      <PageHeader
        title={`${pipeline.workflowName} #${pipeline.number}`}
        parent={{ label: 'Pipelines', to: workspacePath(slug, 'pipelines') }}
        actions={
          <>
            <PipelineStatusBadge status={pipeline.status} conclusion={pipeline.conclusion} />
            {isLive ? (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setConfirmCancel(true)}
                isLoading={cancel.isPending}
              >
                <Square size={13} aria-hidden="true" />
                Cancel
              </Button>
            ) : (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => rerun.mutate(pipeline.id)}
                isLoading={rerun.isPending}
              >
                <RotateCcw size={13} aria-hidden="true" />
                Re-run
              </Button>
            )}
          </>
        }
      />
      <p className="-mt-4 mb-4 flex items-center gap-2 font-mono text-xs text-steel">
        {pipeline.repoFullName} · {branchOfRef(pipeline.gitRef)} · {shortSha(pipeline.commitSha)}
        {pipeline.commitMessage ? ` — ${pipeline.commitMessage}` : ''}
        <span
          className="inline-flex items-center gap-1"
          title={
            stream.connected
              ? 'Live: streaming execution updates'
              : 'Stream disconnected: falling back to polling'
          }
        >
          {stream.connected ? (
            <Wifi size={12} className="text-primary" aria-hidden="true" />
          ) : (
            <WifiOff size={12} aria-hidden="true" />
          )}
        </span>
      </p>

      {(() => {
        const overview = (
          <>
            <PipelineGraph
              trigger={pipeline.trigger}
              jobs={jobs}
              selected={selectedKey}
              onSelect={setSelectedKey}
            />
            <ExecutionTimeline
              pipeline={pipeline}
              jobs={jobs}
              selected={selectedKey}
              onSelect={setSelectedKey}
              jobHref={(job) => workspacePath(slug, `pipelines/${pipeline.id}/jobs/${job.id}`)}
            />
          </>
        );
        const inspectorTabs = (
          <Tabs
            ariaLabel="Execution inspector"
            active={tab}
            onChange={(id) => setTab(id as TabId)}
            className="px-2"
            tabs={[
              { id: 'logs', label: 'Logs' },
              { id: 'environment', label: 'Environment' },
              { id: 'artifacts', label: 'Artifacts' },
              { id: 'metadata', label: 'Metadata' },
              { id: 'container', label: 'Container' },
              { id: 'performance', label: 'Performance' },
            ]}
          />
        );
        const logView = selectedJob ? (
          <LogViewer
            workspaceId={workspaceId}
            pipelineId={pipelineId}
            job={selectedJob}
            onWatch={stream.watchJobLogs}
            events={events.filter((event) => event.jobId === selectedJob.id)}
          />
        ) : (
          <p className="p-3 text-sm text-steel">Select a job to view its logs.</p>
        );
        const panelView = (
          <>
            {tab === 'environment' &&
              (selectedJob ? (
                <EnvironmentPanel job={selectedJob} />
              ) : (
                <p className="text-sm text-steel">Select a job.</p>
              ))}
            {tab === 'artifacts' && <ArtifactsPanel pipelineId={pipeline.id} />}
            {tab === 'metadata' && (
              <MetadataPanel
                pipeline={pipeline}
                job={selectedJob}
                jobHref={
                  selectedJob
                    ? workspacePath(slug, `pipelines/${pipeline.id}/jobs/${selectedJob.id}`)
                    : undefined
                }
              />
            )}
            {tab === 'container' &&
              (selectedJob ? (
                <ContainerPanel job={selectedJob} />
              ) : (
                <p className="text-sm text-steel">Select a job.</p>
              ))}
            {tab === 'performance' && <PerformancePanel jobs={jobs} events={events} />}
          </>
        );

        if (!isDesktop) {
          // Below lg: no resizable panels — a natural page-scroll stack.
          return (
            <div className="space-y-4">
              <div className="space-y-3 rounded border border-steel/20 bg-canvas p-3">
                {overview}
              </div>
              <div className="flex flex-col overflow-hidden rounded border border-steel/20 bg-canvas">
                {inspectorTabs}
                {tab === 'logs' ? (
                  <div className="h-[60vh] min-h-[320px]">{logView}</div>
                ) : (
                  <div className="p-3">{panelView}</div>
                )}
              </div>
            </div>
          );
        }

        return (
          <div className="h-[72vh] min-h-[520px] overflow-hidden rounded border border-steel/20 bg-canvas">
            <Group orientation="horizontal" className="h-full">
              <Panel defaultSize="44%" minSize="30%" className="flex flex-col">
                <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">{overview}</div>
              </Panel>
              <Separator className="w-1 bg-steel/10 transition-colors hover:bg-primary/40" />
              <Panel defaultSize="56%" minSize="30%" className="flex flex-col">
                {inspectorTabs}
                {tab === 'logs' ? (
                  <div className="min-h-0 flex-1">{logView}</div>
                ) : (
                  <div className="min-h-0 flex-1 overflow-y-auto p-3">{panelView}</div>
                )}
              </Panel>
            </Group>
          </div>
        );
      })()}

      <Dialog
        open={confirmCancel}
        onClose={() => setConfirmCancel(false)}
        title="Cancel this pipeline?"
        description="Queued jobs stop immediately; running jobs are signalled to abort. This cannot be undone."
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setConfirmCancel(false)}>
              Keep running
            </Button>
            <Button
              variant="primary"
              size="sm"
              isLoading={cancel.isPending}
              onClick={() => {
                cancel.mutate(pipeline.id, { onSettled: () => setConfirmCancel(false) });
              }}
            >
              Cancel pipeline
            </Button>
          </>
        }
      />
    </>
  );
}
