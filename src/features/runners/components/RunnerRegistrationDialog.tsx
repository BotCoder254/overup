import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useState } from 'react';
import { Dialog } from '../../../components/ui/Dialog';
import { revokeRunner } from '../api/runnersApi';
import {
  runnersKey,
  useBootstrapRunner,
  useCreateHostedRunner,
  useHostedRunnerAvailable,
  useWorkspaceId,
} from '../hooks/useRunners';
import { InstallCommandStep } from './wizard/InstallCommandStep';
import { OsArchStep, type RunnerMode } from './wizard/OsArchStep';
import { WaitForConnectionStep } from './wizard/WaitForConnectionStep';

interface RunnerRegistrationDialogProps {
  open: boolean;
  onClose: () => void;
}

type Step = 'details' | 'install' | 'waiting';

const TITLES: Record<Step, string> = {
  details: 'Register a runner',
  install: 'Install the runner',
  waiting: 'Connecting',
};

function parseLabels(value: string): string[] {
  return Array.from(
    new Set(
      value
        .split(',')
        .map((label) => label.trim())
        .filter(Boolean),
    ),
  );
}

/**
 * Guided registration. Self-hosted: name/labels -> install command -> live
 * connection check. Hosted (when the deployment supports it): name/labels ->
 * the server provisions a runner container itself -> live connection check —
 * no install step, no token ever shown.
 */
export function RunnerRegistrationDialog({ open, onClose }: RunnerRegistrationDialogProps) {
  const bootstrap = useBootstrapRunner();
  const createHosted = useCreateHostedRunner();
  const hostedAvailable = useHostedRunnerAvailable();
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  const [step, setStep] = useState<Step>('details');
  const [mode, setMode] = useState<RunnerMode>('self-hosted');
  const [name, setName] = useState('');
  const [labels, setLabels] = useState('');
  const [os, setOs] = useState('');
  const [issued, setIssued] = useState<{ runnerId: string; token: string | null } | null>(null);

  // Stable across re-renders (typing updates name/labels state every
  // keystroke) so the Dialog never sees a changing onClose reference.
  const close = useCallback(() => {
    onClose();
    setStep('details');
    setMode('self-hosted');
    setName('');
    setLabels('');
    setOs('');
    setIssued(null);
  }, [onClose]);

  const onDetailsNext = () => {
    const input = { name: name.trim(), labels: parseLabels(labels) };
    if (mode === 'hosted') {
      createHosted.mutate(input, {
        onSuccess: (runner) => {
          setIssued({ runnerId: runner.id, token: null });
          setStep('waiting');
        },
      });
    } else {
      bootstrap.mutate(input, {
        onSuccess: (result) => {
          setIssued({ runnerId: result.runner.id, token: result.token });
          setStep('install');
        },
      });
    }
  };

  // A failed hosted provision leaves a dead runner row behind; discard it
  // (best-effort — the janitor purges stragglers) and restart from the
  // details step with the typed name/labels intact.
  const retryHosted = useCallback(() => {
    if (issued && workspaceId) {
      void revokeRunner(workspaceId, issued.runnerId)
        .then(() => queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) }))
        .catch(() => {});
    }
    setIssued(null);
    setStep('details');
  }, [issued, workspaceId, queryClient]);

  const waitingMessage =
    mode === 'hosted'
      ? 'Provisioning the hosted runner…'
      : 'Waiting for the runner to connect…';

  return (
    <Dialog
      open={open}
      onClose={close}
      title={step === 'waiting' && mode === 'hosted' ? 'Provisioning' : TITLES[step]}
      className="max-w-lg"
    >
      {step === 'details' && (
        <OsArchStep
          name={name}
          onNameChange={setName}
          labels={labels}
          onLabelsChange={setLabels}
          os={os}
          onOsChange={setOs}
          mode={mode}
          onModeChange={setMode}
          hostedAvailable={hostedAvailable.data === true}
          onNext={onDetailsNext}
          isLoading={bootstrap.isPending || createHosted.isPending}
        />
      )}
      {step === 'install' && issued?.token && (
        <InstallCommandStep
          token={issued.token}
          name={name.trim()}
          labels={parseLabels(labels)}
          onNext={() => setStep('waiting')}
        />
      )}
      {step === 'waiting' && issued && (
        <WaitForConnectionStep
          runnerId={issued.runnerId}
          message={waitingMessage}
          onDone={close}
          onRetry={mode === 'hosted' ? retryHosted : undefined}
        />
      )}
    </Dialog>
  );
}
