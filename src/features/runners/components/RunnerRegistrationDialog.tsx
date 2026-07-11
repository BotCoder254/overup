import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useState } from 'react';
import { Dialog } from '../../../components/ui/Dialog';
import type { RunnerResourceProfile } from '../../../types/runner';
import { revokeRunner } from '../api/runnersApi';
import {
  runnersKey,
  useBootstrapRunner,
  useCreateHostedRunner,
  useHostedRunnerInfo,
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
  details: 'Create a runner',
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
 * Guided creation. Hosted (the default whenever the deployment supports it):
 * name/labels/size/instances -> the server provisions runner container(s)
 * itself -> live connection check — no install step, no token ever shown.
 * Self-hosted (behind "Advanced"): name/labels -> install command -> live
 * connection check.
 */
export function RunnerRegistrationDialog({ open, onClose }: RunnerRegistrationDialogProps) {
  const bootstrap = useBootstrapRunner();
  const createHosted = useCreateHostedRunner();
  const hostedInfo = useHostedRunnerInfo();
  const workspaceId = useWorkspaceId();
  const queryClient = useQueryClient();

  const [step, setStep] = useState<Step>('details');
  const [mode, setMode] = useState<RunnerMode>('hosted');
  const [name, setName] = useState('');
  const [labels, setLabels] = useState('');
  const [os, setOs] = useState('');
  const [profile, setProfile] = useState<RunnerResourceProfile>('standard');
  const [instances, setInstances] = useState(1);
  const [issued, setIssued] = useState<{ runnerIds: string[]; token: string | null } | null>(null);

  // Hosted is the default path, but only when the deployment can actually
  // provision and has quota headroom — otherwise the classic self-hosted
  // flow is the only one.
  const hostedUsable =
    hostedInfo.data?.hostedAvailable === true && hostedInfo.data.hostedRemaining !== 0;
  const effectiveMode: RunnerMode = hostedUsable ? mode : 'self-hosted';

  // Stable across re-renders (typing updates name/labels state every
  // keystroke) so the Dialog never sees a changing onClose reference.
  const close = useCallback(() => {
    onClose();
    setStep('details');
    setMode('hosted');
    setName('');
    setLabels('');
    setOs('');
    setProfile('standard');
    setInstances(1);
    setIssued(null);
  }, [onClose]);

  const onDetailsNext = () => {
    const input = { name: name.trim(), labels: parseLabels(labels) };
    if (effectiveMode === 'hosted') {
      createHosted.mutate(
        { ...input, resourceProfile: profile, instances },
        {
          onSuccess: (runners) => {
            setIssued({ runnerIds: runners.map((runner) => runner.id), token: null });
            setStep('waiting');
          },
        },
      );
    } else {
      bootstrap.mutate(input, {
        onSuccess: (result) => {
          setIssued({ runnerIds: [result.runner.id], token: result.token });
          setStep('install');
        },
      });
    }
  };

  // A failed hosted provision leaves dead runner rows behind; discard them
  // (best-effort — the janitor purges stragglers) and restart from the
  // details step with the typed name/labels intact.
  const retryHosted = useCallback(() => {
    if (issued && workspaceId) {
      void Promise.allSettled(
        issued.runnerIds.map((runnerId) => revokeRunner(workspaceId, runnerId)),
      ).then(() => queryClient.invalidateQueries({ queryKey: runnersKey(workspaceId) }));
    }
    setIssued(null);
    setStep('details');
  }, [issued, workspaceId, queryClient]);

  const waitingMessage =
    effectiveMode === 'hosted'
      ? issued && issued.runnerIds.length > 1
        ? 'Provisioning the hosted runners…'
        : 'Provisioning the hosted runner…'
      : 'Waiting for the runner to connect…';

  return (
    <Dialog
      open={open}
      onClose={close}
      title={step === 'waiting' && effectiveMode === 'hosted' ? 'Provisioning' : TITLES[step]}
      className="max-w-xl"
    >
      {step === 'details' && (
        <OsArchStep
          name={name}
          onNameChange={setName}
          labels={labels}
          onLabelsChange={setLabels}
          os={os}
          onOsChange={setOs}
          mode={effectiveMode}
          onModeChange={setMode}
          profile={profile}
          onProfileChange={setProfile}
          instances={instances}
          onInstancesChange={setInstances}
          hostedAvailable={hostedInfo.data?.hostedAvailable === true}
          hostedRemaining={hostedInfo.data?.hostedRemaining ?? null}
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
          runnerIds={issued.runnerIds}
          message={waitingMessage}
          onDone={close}
          onRetry={effectiveMode === 'hosted' ? retryHosted : undefined}
        />
      )}
    </Dialog>
  );
}
