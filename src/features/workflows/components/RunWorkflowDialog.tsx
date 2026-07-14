import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { workspacePath } from '../../../app/navigation';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import type { WorkflowDetail, WorkflowDispatchInput } from '../../../types/workflow';
import { useRepositoryDetail } from '../../repositories/hooks/useRepositories';
import { useDispatchWorkflow } from '../../pipelines/hooks/usePipelines';

const SHA_PATTERN = /^[0-9a-fA-F]{7,40}$/;

const controlClasses =
  'h-9 rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal ' +
  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

type InputValues = Record<string, string | boolean>;

function defaultsFor(inputs: WorkflowDispatchInput[]): InputValues {
  const values: InputValues = {};
  for (const input of inputs) {
    if (input.type === 'boolean') {
      values[input.name] = input.default === 'true';
    } else if (input.default !== undefined) {
      values[input.name] = input.default;
    } else if (input.type === 'choice' && input.options?.length) {
      values[input.name] = input.options[0];
    } else {
      values[input.name] = '';
    }
  }
  return values;
}

interface RunWorkflowDialogProps {
  open: boolean;
  onClose: () => void;
  workflow: WorkflowDetail;
  workspaceSlug: string;
}

/**
 * Manual trigger ("Run workflow"): branch + optional commit pin + a form
 * generated from the workflow's parsed `workflow_dispatch` inputs. Nothing is
 * scheduled until the user confirms; on success the browser lands on the new
 * pipeline's live detail page. The server re-validates everything — this
 * dialog only mirrors its rules for immediate feedback.
 */
export function RunWorkflowDialog({
  open,
  onClose,
  workflow,
  workspaceSlug,
}: RunWorkflowDialogProps) {
  const navigate = useNavigate();
  const dispatch = useDispatchWorkflow();
  const repository = useRepositoryDetail(open ? workflow.repositoryId : undefined);

  const dispatchInputs = useMemo(
    () => workflow.metadata.dispatchInputs ?? [],
    [workflow.metadata.dispatchInputs],
  );
  const hasDispatchTrigger = workflow.triggers.includes('workflow_dispatch');

  const [branch, setBranch] = useState(workflow.defaultBranch);
  const [commitSha, setCommitSha] = useState('');
  const [inputValues, setInputValues] = useState<InputValues>({});

  useEffect(() => {
    if (open) {
      setBranch(workflow.defaultBranch);
      setCommitSha('');
      setInputValues(defaultsFor(dispatchInputs));
    }
  }, [open, workflow.defaultBranch, dispatchInputs]);

  const close = useCallback(() => {
    if (dispatch.isPending) return;
    onClose();
  }, [dispatch.isPending, onClose]);

  const branches = repository.data?.branches ?? [];
  const shaProblem =
    commitSha && !SHA_PATTERN.test(commitSha.trim())
      ? 'A commit is 7-40 hexadecimal characters.'
      : undefined;
  const missingRequired = dispatchInputs.filter((input) => {
    if (!input.required) return false;
    const value = inputValues[input.name];
    return input.type === 'boolean' ? false : !String(value ?? '').trim();
  });
  const invalid = Boolean(shaProblem) || missingRequired.length > 0 || !branch;
  // Inputs that will actually be submitted (booleans always; text-likes only
  // when non-empty — empty optionals defer to server-side defaults).
  const effectiveInputCount = dispatchInputs.filter((input) => {
    if (input.type === 'boolean') return true;
    return Boolean(String(inputValues[input.name] ?? '').trim());
  }).length;

  const submit = () => {
    if (invalid || dispatch.isPending) return;
    const inputs: Record<string, string | number | boolean> = {};
    for (const input of dispatchInputs) {
      const value = inputValues[input.name];
      if (input.type === 'boolean') {
        inputs[input.name] = Boolean(value);
      } else {
        const text = String(value ?? '').trim();
        if (!text) continue; // optional + empty: let the server apply defaults
        inputs[input.name] = input.type === 'number' ? Number(text) : text;
      }
    }
    dispatch.mutate(
      {
        workflowId: workflow.id,
        branch,
        commitSha: commitSha.trim() || undefined,
        inputs: dispatchInputs.length > 0 ? inputs : undefined,
      },
      {
        onSuccess: (pipeline) => {
          onClose();
          navigate(workspacePath(workspaceSlug, `pipelines/${pipeline.id}`));
        },
      },
    );
  };

  const setInput = (name: string, value: string | boolean) =>
    setInputValues((current) => ({ ...current, [name]: value }));

  return (
    <Dialog
      open={open}
      onClose={close}
      title="Run workflow"
      description="Review the execution options below — nothing runs until you confirm. The run executes the stored revision of this workflow through the workspace scheduler and runners."
      className="max-w-xl"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={close} disabled={dispatch.isPending}>
            Cancel
          </Button>
          <Button size="sm" disabled={invalid} isLoading={dispatch.isPending} onClick={submit}>
            Run workflow
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <FormField
          id="run-workflow-branch"
          label="Branch"
          hint="The run checks out this branch's head unless a commit is pinned below."
        >
          {(aria) => (
            <select
              {...aria}
              className={`${controlClasses} w-full`}
              value={branch}
              onChange={(event) => setBranch(event.target.value)}
              disabled={repository.isLoading}
            >
              {branches.length === 0 && <option value={branch}>{branch}</option>}
              {branches.map((item) => (
                <option key={item.name} value={item.name}>
                  {item.name}
                  {item.isDefault ? ' (default)' : ''}
                </option>
              ))}
            </select>
          )}
        </FormField>

        <FormField
          id="run-workflow-sha"
          label="Commit"
          optional
          hint="Pin an exact commit SHA instead of the branch head."
          error={shaProblem}
        >
          {(aria) => (
            <Input
              {...aria}
              autoComplete="off"
              spellCheck={false}
              placeholder="Branch head"
              maxLength={40}
              className="font-mono"
              value={commitSha}
              onChange={(event) => setCommitSha(event.target.value)}
            />
          )}
        </FormField>

        {dispatchInputs.length > 0 ? (
          <fieldset className="space-y-4 rounded border border-steel/20 p-3">
            <legend className="px-1 text-xs font-semibold uppercase tracking-wide text-steel">
              Workflow inputs
            </legend>
            {dispatchInputs.map((input) => {
              const id = `run-input-${input.name}`;
              if (input.type === 'boolean') {
                return (
                  <label
                    key={input.name}
                    htmlFor={id}
                    className="flex items-start gap-2 text-sm text-charcoal"
                  >
                    <input
                      id={id}
                      type="checkbox"
                      className="mt-0.5 h-4 w-4 rounded border-steel/30 accent-primary"
                      checked={Boolean(inputValues[input.name])}
                      onChange={(event) => setInput(input.name, event.target.checked)}
                    />
                    <span>
                      <span className="font-mono text-xs">{input.name}</span>
                      {input.description && (
                        <span className="block text-xs text-steel">{input.description}</span>
                      )}
                    </span>
                  </label>
                );
              }
              return (
                <FormField
                  key={input.name}
                  id={id}
                  label={input.name}
                  optional={!input.required}
                  hint={input.description}
                >
                  {(aria) =>
                    input.type === 'choice' ? (
                      <select
                        {...aria}
                        className={`${controlClasses} w-full`}
                        value={String(inputValues[input.name] ?? '')}
                        onChange={(event) => setInput(input.name, event.target.value)}
                      >
                        {(input.options ?? []).map((option) => (
                          <option key={option} value={option}>
                            {option}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <Input
                        {...aria}
                        type={input.type === 'number' ? 'number' : 'text'}
                        autoComplete="off"
                        spellCheck={false}
                        value={String(inputValues[input.name] ?? '')}
                        onChange={(event) => setInput(input.name, event.target.value)}
                      />
                    )
                  }
                </FormField>
              );
            })}
          </fieldset>
        ) : hasDispatchTrigger ? (
          <p className="rounded border border-steel/20 bg-surface px-3 py-2 text-xs text-steel">
            This workflow declares <span className="font-mono">workflow_dispatch</span> without
            inputs. If inputs were added recently, re-sync the repository to pick up their
            definitions.
          </p>
        ) : null}

        <div className="rounded border border-steel/20 bg-surface px-3 py-2 text-xs text-steel">
          <p className="mb-1 font-semibold uppercase tracking-wide">Execution summary</p>
          <p>
            <span className="font-mono text-charcoal">{workflow.name}</span> will run on{' '}
            <span className="font-mono text-charcoal">{branch}</span>
            {commitSha.trim() ? (
              <>
                {' '}
                at commit{' '}
                <span className="font-mono text-charcoal">
                  {commitSha.trim().slice(0, 12)}
                </span>
              </>
            ) : (
              ' at the branch head'
            )}
            {effectiveInputCount > 0 && (
              <>
                {' '}
                with {effectiveInputCount} input{effectiveInputCount === 1 ? '' : 's'}
              </>
            )}
            . The trigger is recorded as <span className="font-mono text-charcoal">manual</span>{' '}
            with you as the actor.
          </p>
          {missingRequired.length > 0 && (
            <p className="mt-1 text-danger">
              Required: {missingRequired.map((input) => input.name).join(', ')}
            </p>
          )}
        </div>
      </div>
    </Dialog>
  );
}
