import { useCallback, useState } from 'react';
import { Dialog } from '../../../components/ui/Dialog';
import { useBootstrapRunner } from '../hooks/useRunners';
import { InstallCommandStep } from './wizard/InstallCommandStep';
import { OsArchStep } from './wizard/OsArchStep';
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

/** Guided registration: name/labels -> install command -> live connection check. */
export function RunnerRegistrationDialog({ open, onClose }: RunnerRegistrationDialogProps) {
  const bootstrap = useBootstrapRunner();
  const [step, setStep] = useState<Step>('details');
  const [name, setName] = useState('');
  const [labels, setLabels] = useState('');
  const [os, setOs] = useState('');
  const [issued, setIssued] = useState<{ runnerId: string; token: string } | null>(null);

  // Stable across re-renders (typing updates name/labels state every
  // keystroke) so the Dialog never sees a changing onClose reference.
  const close = useCallback(() => {
    onClose();
    setStep('details');
    setName('');
    setLabels('');
    setOs('');
    setIssued(null);
  }, [onClose]);

  const onDetailsNext = () => {
    bootstrap.mutate(
      { name: name.trim(), labels: parseLabels(labels) },
      {
        onSuccess: (result) => {
          setIssued({ runnerId: result.runner.id, token: result.token });
          setStep('install');
        },
      },
    );
  };

  return (
    <Dialog open={open} onClose={close} title={TITLES[step]} className="max-w-lg">
      {step === 'details' && (
        <OsArchStep
          name={name}
          onNameChange={setName}
          labels={labels}
          onLabelsChange={setLabels}
          os={os}
          onOsChange={setOs}
          onNext={onDetailsNext}
          isLoading={bootstrap.isPending}
        />
      )}
      {step === 'install' && issued && (
        <InstallCommandStep
          token={issued.token}
          labels={parseLabels(labels)}
          onNext={() => setStep('waiting')}
        />
      )}
      {step === 'waiting' && issued && (
        <WaitForConnectionStep runnerId={issued.runnerId} onDone={close} />
      )}
    </Dialog>
  );
}
