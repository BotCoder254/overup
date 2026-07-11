import { useCallback, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import { Textarea } from '../../../components/ui/Textarea';
import { useRepositories } from '../../repositories/hooks/useRepositories';
import type { Secret } from '../../../types/secret';
import { useCreateSecret, useReplaceSecretValue } from '../hooks/useSecrets';

/**
 * Client-side mirror of the server's validation (the server remains the
 * authority). Names are UPPER_SNAKE_CASE; platform prefixes and well-known
 * environment variables are reserved.
 */
const NAME_PATTERN = /^[A-Z_][A-Z0-9_]*$/;
const RESERVED_PREFIXES = ['OVERUP_', 'GITHUB_', 'RUNNER_', 'DOCKER_'];
const RESERVED_NAMES = [
  'CI',
  'PATH',
  'HOME',
  'SHELL',
  'HOSTNAME',
  'LANG',
  'PWD',
  'USER',
  'TMPDIR',
  'LD_PRELOAD',
  'LD_LIBRARY_PATH',
];
const VALUE_MIN = 8;
const VALUE_MAX = 32 * 1024;

function nameError(name: string): string | undefined {
  if (!name) return undefined;
  if (name.length > 200) return 'At most 200 characters.';
  if (!NAME_PATTERN.test(name)) {
    return 'UPPER_SNAKE_CASE only: letters A-Z, digits, underscores; not starting with a digit.';
  }
  const prefix = RESERVED_PREFIXES.find((p) => name.startsWith(p));
  if (prefix) return `The ${prefix} prefix is reserved for the platform.`;
  if (RESERVED_NAMES.includes(name)) return `${name} is a reserved environment variable name.`;
  return undefined;
}

function valueError(value: string): string | undefined {
  if (!value) return undefined;
  const bytes = new TextEncoder().encode(value).length;
  if (bytes < VALUE_MIN) return `At least ${VALUE_MIN} bytes (shorter values cannot be masked).`;
  if (bytes > VALUE_MAX) return 'At most 32 KB.';
  if (value.includes('\0')) return 'NUL bytes are not allowed.';
  return undefined;
}

interface SecretFormDialogProps {
  open: boolean;
  onClose: () => void;
  /** When set, the dialog replaces this secret's value instead of creating. */
  replaceTarget?: Pick<Secret, 'id' | 'name' | 'scope' | 'repositoryName'> | null;
}

/**
 * Create a secret, or replace an existing secret's value. The value field
 * is the only place plaintext ever exists in the browser: it is sent once
 * over TLS, encrypted server-side, and can never be viewed again — the
 * dialog confirms success without echoing anything back.
 */
export function SecretFormDialog({ open, onClose, replaceTarget }: SecretFormDialogProps) {
  const repositories = useRepositories();
  const create = useCreateSecret();
  const replace = useReplaceSecretValue();
  const replacing = Boolean(replaceTarget);

  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [description, setDescription] = useState('');
  const [scope, setScope] = useState<'workspace' | 'repository'>('workspace');
  const [repositoryId, setRepositoryId] = useState('');

  // Stable close that also resets state, read via the Dialog's onClose ref
  // so keystroke re-renders never disturb its focus effect.
  const close = useCallback(() => {
    setName('');
    setValue('');
    setDescription('');
    setScope('workspace');
    setRepositoryId('');
    onClose();
  }, [onClose]);

  const nameProblem = nameError(name);
  const valueProblem = valueError(value);
  const missingRepo = !replacing && scope === 'repository' && !repositoryId;
  const invalid =
    Boolean(valueProblem) ||
    !value ||
    (!replacing && (Boolean(nameProblem) || !name || missingRepo || description.length > 500));
  const pending = create.isPending || replace.isPending;

  const submit = () => {
    if (invalid || pending) return;
    if (replacing && replaceTarget) {
      replace.mutate({ secretId: replaceTarget.id, value }, { onSuccess: close });
      return;
    }
    create.mutate(
      {
        name,
        value,
        description: description.trim() || undefined,
        repositoryId: scope === 'repository' ? repositoryId : undefined,
      },
      { onSuccess: close },
    );
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      title={replacing ? `Replace value of ${replaceTarget?.name}` : 'New secret'}
      description={
        replacing
          ? 'The current value can never be shown. Entering a new value replaces it permanently for every future pipeline run.'
          : 'The value is encrypted before it is stored and can never be viewed again — only replaced. It is injected into matching pipeline jobs as an environment variable.'
      }
      className="max-w-lg"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button size="sm" disabled={invalid} isLoading={pending} onClick={submit}>
            {replacing ? 'Replace value' : 'Create secret'}
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        {!replacing && (
          <>
            <FormField
              id="secret-name"
              label="Name"
              hint="UPPER_SNAKE_CASE — this becomes the environment variable name in job containers."
              error={nameProblem}
            >
              {(aria) => (
                <Input
                  {...aria}
                  autoFocus
                  autoComplete="off"
                  spellCheck={false}
                  placeholder="DEPLOY_TOKEN"
                  maxLength={200}
                  className="font-mono"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              )}
            </FormField>

            <fieldset>
              <legend className="text-sm font-medium text-charcoal">Scope</legend>
              <div className="mt-2 space-y-2">
                <label className="flex items-start gap-2 text-sm text-charcoal">
                  <input
                    type="radio"
                    name="secret-scope"
                    className="mt-0.5 accent-primary"
                    checked={scope === 'workspace'}
                    onChange={() => setScope('workspace')}
                  />
                  <span>
                    Workspace
                    <span className="block text-xs text-steel">
                      Available to pipelines in every repository.
                    </span>
                  </span>
                </label>
                <label className="flex items-start gap-2 text-sm text-charcoal">
                  <input
                    type="radio"
                    name="secret-scope"
                    className="mt-0.5 accent-primary"
                    checked={scope === 'repository'}
                    onChange={() => setScope('repository')}
                  />
                  <span>
                    Repository
                    <span className="block text-xs text-steel">
                      Limited to one repository; overrides a workspace secret of the same name.
                    </span>
                  </span>
                </label>
              </div>
            </fieldset>

            {scope === 'repository' && (
              <FormField
                id="secret-repository"
                label="Repository"
                error={missingRepo ? 'Choose the repository this secret belongs to.' : undefined}
              >
                {(aria) => (
                  <select
                    {...aria}
                    className="h-9 w-full rounded border border-steel/30 bg-canvas px-2 text-sm text-charcoal focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                    value={repositoryId}
                    onChange={(event) => setRepositoryId(event.target.value)}
                  >
                    <option value="">Select a repository…</option>
                    {(repositories.data ?? []).map((repo) => (
                      <option key={repo.id} value={repo.id}>
                        {repo.fullName}
                      </option>
                    ))}
                  </select>
                )}
              </FormField>
            )}
          </>
        )}

        {replacing && replaceTarget && (
          <p className="rounded border border-steel/20 bg-surface p-3 text-xs text-steel">
            <span className="font-mono text-charcoal">{replaceTarget.name}</span> ·{' '}
            {replaceTarget.scope === 'repository'
              ? `repository secret (${replaceTarget.repositoryName ?? 'unknown'})`
              : 'workspace secret'}{' '}
            — name and scope are immutable.
          </p>
        )}

        <FormField
          id="secret-value"
          label={replacing ? 'New value' : 'Value'}
          hint="Sent once over an encrypted connection, then stored as ciphertext. Never appears in logs — it is masked automatically."
          error={valueProblem}
        >
          {(aria) => (
            <Textarea
              {...aria}
              autoFocus={replacing}
              autoComplete="off"
              spellCheck={false}
              rows={3}
              className="font-mono"
              placeholder="Paste the secret value…"
              value={value}
              onChange={(event) => setValue(event.target.value)}
            />
          )}
        </FormField>

        {!replacing && (
          <FormField
            id="secret-description"
            label="Description"
            optional
            hint="What this credential is for and where it came from."
            error={description.length > 500 ? 'At most 500 characters.' : undefined}
          >
            {(aria) => (
              <Textarea
                {...aria}
                rows={2}
                maxLength={600}
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
            )}
          </FormField>
        )}
      </div>
    </Dialog>
  );
}
