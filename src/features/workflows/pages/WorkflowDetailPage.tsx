import { ExternalLink, Info, Play } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';
import { Group, Panel, Separator } from 'react-resizable-panels';
import { PageHeader } from '../../../components/layout/PageHeader';
import { Button } from '../../../components/ui/Button';
import { Spinner } from '../../../components/ui/Spinner';
import { Tabs } from '../../../components/ui/Tabs';
import { workspacePath } from '../../../app/navigation';
import type { ValidateResponse } from '../../../types/workflow';
import { JobsPanel } from '../components/JobsPanel';
import { MetadataPanel } from '../components/MetadataPanel';
import { RunWorkflowDialog } from '../components/RunWorkflowDialog';
import { ValidationBadge } from '../components/ValidationBadge';
import { WorkflowMetaStrip } from '../components/WorkflowMetaStrip';
import { ValidationPanel } from '../components/ValidationPanel';
import { WorkflowEditor } from '../components/WorkflowEditor';
import { WorkflowGraph, type GraphJob } from '../components/WorkflowGraph';
import { useValidateWorkflow, useWorkflowDetail } from '../hooks/useWorkflows';

type TabId = 'graph' | 'jobs' | 'metadata';

const VALIDATE_DEBOUNCE_MS = 500;

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** Best-effort line of a job's definition in the current editor content. */
function findJobLine(content: string, jobKey: string): number | null {
  const pattern = new RegExp(`^\\s{1,6}(?:"${escapeRegExp(jobKey)}"|${escapeRegExp(jobKey)})\\s*:`, 'm');
  const match = pattern.exec(content);
  if (!match) return null;
  return content.slice(0, match.index).split('\n').length;
}

export function WorkflowDetailPage() {
  const { slug = '', workflowId } = useParams<{ slug: string; workflowId: string }>();
  const detail = useWorkflowDetail(workflowId);
  const validate = useValidateWorkflow();

  const [content, setContent] = useState<string | null>(null);
  const [live, setLive] = useState<ValidateResponse | null>(null);
  const [tab, setTab] = useState<TabId>('graph');
  const [selectedJob, setSelectedJob] = useState<string | null>(null);
  const [revealLine, setRevealLine] = useState<number | null>(null);
  const [runOpen, setRunOpen] = useState(false);

  const workflow = detail.data;

  // Seed the editor once the workflow loads.
  useEffect(() => {
    if (workflow && content === null) setContent(workflow.rawContent);
  }, [workflow, content]);

  // Debounced live validation: every edited snapshot goes through the same
  // parser the sync pipeline uses. The pristine document keeps its stored
  // diagnostics without a round-trip.
  useEffect(() => {
    if (content === null || !workflow) return undefined;
    if (content === workflow.rawContent && live === null) return undefined;
    const timer = setTimeout(() => {
      validate.mutate(content, { onSuccess: setLive });
    }, VALIDATE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [content]);

  const diagnostics = live?.diagnostics ?? workflow?.validationErrors ?? [];
  const status = live?.status ?? workflow?.validationStatus ?? 'valid';
  const triggers = live?.triggers ?? workflow?.triggers ?? [];
  const graphJobs: GraphJob[] = useMemo(() => {
    if (live) return live.jobs;
    return (workflow?.jobs ?? []).map((job) => ({
      key: job.key,
      name: job.name,
      needs: job.needs,
      runsOn: job.runsOn,
      uses: job.uses,
      stepCount: job.stepCount,
    }));
  }, [live, workflow]);

  const onSelectJob = (jobKey: string) => {
    setSelectedJob(jobKey);
    if (content) {
      const line = findJobLine(content, jobKey);
      if (line) setRevealLine(line);
    }
  };

  if (detail.isLoading) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <Spinner className="h-6 w-6 text-steel" />
      </div>
    );
  }
  if (detail.isError || !workflow) {
    return (
      <>
        <PageHeader
          title="Workflow"
          parent={{ label: 'Workflows', to: workspacePath(slug, 'workflows') }}
        />
        <p className="text-sm text-steel">
          This workflow could not be loaded — it may have been removed in a recent sync.
        </p>
      </>
    );
  }

  const githubUrl = `https://github.com/${workflow.repoFullName}/blob/${workflow.defaultBranch}/${workflow.path}`;

  return (
    <>
      <PageHeader
        title={workflow.name}
        parent={{ label: 'Workflows', to: workspacePath(slug, 'workflows') }}
        actions={
          <>
            <ValidationBadge status={status} />
            <a
              href={githubUrl}
              target="_blank"
              rel="noreferrer noopener"
              className="inline-flex h-9 items-center gap-1.5 rounded px-3 text-sm font-medium text-charcoal transition-colors hover:bg-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <ExternalLink size={14} aria-hidden="true" />
              View on GitHub
            </a>
            <Button
              size="sm"
              onClick={() => setRunOpen(true)}
              disabled={workflow.validationStatus === 'errors'}
              title={
                workflow.validationStatus === 'errors'
                  ? 'This workflow has validation errors and cannot run.'
                  : undefined
              }
            >
              <Play size={14} aria-hidden="true" />
              Run workflow
            </Button>
          </>
        }
      />
      <RunWorkflowDialog
        open={runOpen}
        onClose={() => setRunOpen(false)}
        workflow={workflow}
        workspaceSlug={slug}
      />
      <p className="-mt-4 mb-4 font-mono text-xs text-steel">
        {workflow.repoFullName} · {workflow.path}
      </p>

      <WorkflowMetaStrip workflow={workflow} slug={slug} />

      <div className="mb-3 flex items-center gap-2 rounded border border-steel/20 bg-surface px-3 py-2 text-xs text-steel">
        <Info size={13} className="shrink-0" aria-hidden="true" />
        Read-only workspace: edits validate live against the platform parser but are not saved —
        GitHub remains the source of truth for this file.
      </div>

      <div className="h-[70vh] min-h-[480px] overflow-hidden rounded border border-steel/20 bg-canvas">
        <Group orientation="horizontal" className="h-full">
          <Panel defaultSize="58%" minSize="35%" className="flex flex-col">
            <div className="min-h-0 flex-1">
              <WorkflowEditor
                value={content ?? ''}
                onChange={setContent}
                diagnostics={diagnostics}
                revealLine={revealLine}
              />
            </div>
            <div className="border-t border-steel/10">
              <ValidationPanel
                diagnostics={diagnostics}
                validating={validate.isPending}
                onSelectLine={(line) => setRevealLine(line)}
              />
            </div>
          </Panel>
          <Separator className="w-1 bg-steel/10 transition-colors hover:bg-primary/40" />
          <Panel defaultSize="42%" minSize="25%" className="flex flex-col">
            <Tabs
              ariaLabel="Workflow inspector"
              active={tab}
              onChange={(id) => setTab(id as TabId)}
              className="px-2"
              tabs={[
                { id: 'graph', label: 'Graph' },
                { id: 'jobs', label: `Jobs (${graphJobs.length})` },
                { id: 'metadata', label: 'Metadata' },
              ]}
            />
            <div className="min-h-0 flex-1 overflow-y-auto p-3">
              {tab === 'graph' && (
                <WorkflowGraph
                  triggers={triggers}
                  jobs={graphJobs}
                  selected={selectedJob}
                  onSelect={onSelectJob}
                />
              )}
              {tab === 'jobs' && (
                <JobsPanel jobs={graphJobs} selected={selectedJob} onSelect={onSelectJob} />
              )}
              {tab === 'metadata' && <MetadataPanel workflow={workflow} />}
            </div>
          </Panel>
        </Group>
      </div>
    </>
  );
}
