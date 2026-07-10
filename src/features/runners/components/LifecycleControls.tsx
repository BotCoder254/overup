import { PauseCircle, PlayCircle, StopCircle } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import type { Runner } from '../../../types/runner';
import { useDisableRunner, useDrainRunner, useResumeRunner } from '../hooks/useRunners';

interface LifecycleControlsProps {
  runner: Runner;
}

/** Disable/drain/resume — gated by current status, confirmed for the disruptive ones. */
export function LifecycleControls({ runner }: LifecycleControlsProps) {
  const drain = useDrainRunner();
  const disable = useDisableRunner();
  const resume = useResumeRunner();
  const [confirmDisable, setConfirmDisable] = useState(false);
  const [confirmDrain, setConfirmDrain] = useState(false);

  if (runner.status === 'disabled') {
    return (
      <Button size="sm" variant="secondary" isLoading={resume.isPending} onClick={() => resume.mutate(runner.id)}>
        <PlayCircle size={14} aria-hidden="true" />
        Resume
      </Button>
    );
  }

  if (runner.draining) {
    return (
      <Button size="sm" variant="ghost" disabled>
        <StopCircle size={14} aria-hidden="true" />
        Draining…
      </Button>
    );
  }

  return (
    <>
      {runner.status === 'busy' && (
        <Button size="sm" variant="secondary" onClick={() => setConfirmDrain(true)}>
          <StopCircle size={14} aria-hidden="true" />
          Drain
        </Button>
      )}
      <Button size="sm" variant="ghost" onClick={() => setConfirmDisable(true)}>
        <PauseCircle size={14} aria-hidden="true" />
        Disable
      </Button>

      <Dialog
        open={confirmDrain}
        onClose={() => setConfirmDrain(false)}
        title={`Drain ${runner.name}?`}
        description="It finishes the job it's currently running, then stops taking new work and goes offline. This cannot be reversed once started."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDrain(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              isLoading={drain.isPending}
              onClick={() => drain.mutate(runner.id, { onSuccess: () => setConfirmDrain(false) })}
            >
              Drain runner
            </Button>
          </>
        }
      />

      <Dialog
        open={confirmDisable}
        onClose={() => setConfirmDisable(false)}
        title={`Disable ${runner.name}?`}
        description="It immediately stops being scheduled new work. Any job already running is left untouched. You can resume it at any time."
        footer={
          <>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDisable(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              isLoading={disable.isPending}
              onClick={() => disable.mutate(runner.id, { onSuccess: () => setConfirmDisable(false) })}
            >
              Disable runner
            </Button>
          </>
        }
      />
    </>
  );
}
