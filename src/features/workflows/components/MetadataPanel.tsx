import { format } from 'date-fns';
import { Boxes, GitCommitHorizontal, KeyRound, Lock, Variable } from 'lucide-react';
import { Badge } from '../../../components/ui/Badge';
import type { WorkflowDetail } from '../../../types/workflow';

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="px-3 py-3">
      <h3 className="mb-2 text-[11px] font-medium uppercase tracking-wider text-steel">{title}</h3>
      {children}
    </section>
  );
}

/** Normalized metadata extracted during sync: triggers, permissions, env, secrets, commit. */
export function MetadataPanel({ workflow }: { workflow: WorkflowDetail }) {
  const { metadata } = workflow;
  const secretRefs = metadata.secretRefs ?? [];
  const varRefs = metadata.varRefs ?? [];
  const environments = metadata.environments ?? [];
  const envKeys = metadata.envKeys ?? [];
  // Reference detection shipped after this workflow's last parse — the
  // arrays are absent (not empty) until the repository re-syncs.
  const staleDetection = metadata.secretRefs === undefined || metadata.varRefs === undefined;

  return (
    <div className="divide-y divide-steel/10 text-sm">
      <Section title="Triggers">
        <div className="flex flex-wrap gap-1">
          {workflow.triggers.length === 0 ? (
            <span className="text-xs text-steel">none detected</span>
          ) : (
            workflow.triggers.map((trigger) => (
              <Badge key={trigger} variant="info">
                {trigger}
              </Badge>
            ))
          )}
        </div>
      </Section>

      <Section title="File">
        <dl className="space-y-1 text-xs">
          <div className="flex gap-2">
            <dt className="w-20 shrink-0 text-steel">Path</dt>
            <dd className="truncate font-mono text-charcoal">{workflow.path}</dd>
          </div>
          <div className="flex gap-2">
            <dt className="w-20 shrink-0 text-steel">Size</dt>
            <dd className="text-charcoal">{(workflow.fileSize / 1024).toFixed(1)} KB</dd>
          </div>
          <div className="flex gap-2">
            <dt className="w-20 shrink-0 text-steel">Branch</dt>
            <dd className="font-mono text-charcoal">{workflow.defaultBranch}</dd>
          </div>
        </dl>
      </Section>

      {workflow.lastCommitSha && (
        <Section title="Last commit">
          <div className="flex items-start gap-2 text-xs">
            <GitCommitHorizontal size={14} className="mt-0.5 shrink-0 text-steel" aria-hidden="true" />
            <div className="min-w-0">
              <p className="truncate text-charcoal">{workflow.lastCommitMessage ?? '—'}</p>
              <p className="mt-0.5 font-mono text-steel">
                {workflow.lastCommitSha.slice(0, 7)}
                {workflow.lastCommitAt &&
                  ` · ${format(new Date(workflow.lastCommitAt), 'PP p')}`}
              </p>
            </div>
          </div>
        </Section>
      )}

      {metadata.permissions !== undefined && metadata.permissions !== null && (
        <Section title="Permissions">
          <pre className="overflow-x-auto rounded bg-surface p-2 font-mono text-[11px] leading-relaxed text-charcoal">
            {JSON.stringify(metadata.permissions, null, 2)}
          </pre>
        </Section>
      )}

      {envKeys.length > 0 && (
        <Section title="Environment keys">
          <div className="flex flex-wrap gap-1">
            {envKeys.map((key) => (
              <Badge key={key} variant="neutral">
                <KeyRound size={10} aria-hidden="true" />
                {key}
              </Badge>
            ))}
          </div>
        </Section>
      )}

      <Section title="Secrets referenced">
        {secretRefs.length === 0 ? (
          <p className="text-xs text-steel">No secrets referenced.</p>
        ) : (
          <div className="flex flex-wrap gap-1">
            {secretRefs.map((name) => (
              <Badge key={name} variant="outline">
                <Lock size={10} aria-hidden="true" />
                {name}
              </Badge>
            ))}
          </div>
        )}
        <p className="mt-2 text-[11px] leading-relaxed text-steel">
          Names only — configure the values on the Secrets page; they resolve at dispatch.
        </p>
      </Section>

      <Section title="Variables referenced">
        {varRefs.length === 0 ? (
          <p className="text-xs text-steel">No variables referenced.</p>
        ) : (
          <div className="flex flex-wrap gap-1">
            {varRefs.map((name) => (
              <Badge key={name} variant="outline">
                <Variable size={10} aria-hidden="true" />
                {name}
              </Badge>
            ))}
          </div>
        )}
      </Section>

      <Section title="Environments">
        {environments.length === 0 ? (
          <p className="text-xs text-steel">No environment bindings.</p>
        ) : (
          <div className="flex flex-wrap gap-1">
            {environments.map((name) => (
              <Badge key={name} variant="primary">
                <Boxes size={10} aria-hidden="true" />
                {name}
              </Badge>
            ))}
          </div>
        )}
        {staleDetection && (
          <p className="mt-2 text-[11px] leading-relaxed text-steel">
            Detection data comes from the last sync — re-sync the repository to refresh it.
          </p>
        )}
      </Section>
    </div>
  );
}
