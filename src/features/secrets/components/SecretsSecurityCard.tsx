import { EyeOff, FileKey2, ScrollText, ShieldCheck, ShieldOff } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';

function PostureRow({
  icon: Icon,
  title,
  detail,
}: {
  icon: LucideIcon;
  title: string;
  detail: string;
}) {
  return (
    <li className="flex items-start gap-2.5 py-2">
      <Icon size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-primary" />
      <span className="min-w-0">
        <span className="block text-sm font-medium text-charcoal">{title}</span>
        <span className="block text-xs text-steel">{detail}</span>
      </span>
    </li>
  );
}

interface SecretsSecurityCardProps {
  /** From the summary endpoint: whether SECRETS_MASTER_KEY is configured. */
  encryptionConfigured: boolean | undefined;
  /** Values not rotated within the stale window, and that window in days. */
  stale?: number;
  staleAfterDays?: number;
}

/**
 * Security posture card: states plainly how the subsystem protects values.
 * Doubles as the warning surface when the deployment has no master key or
 * when values are overdue for rotation.
 */
export function SecretsSecurityCard({
  encryptionConfigured,
  stale,
  staleAfterDays,
}: SecretsSecurityCardProps) {
  return (
    <Card>
      <CardHeader>
        <h2 className="text-sm font-semibold text-charcoal">Security posture</h2>
      </CardHeader>
      <CardBody>
        {encryptionConfigured === false && (
          <div className="mb-2 flex items-start gap-2 rounded border border-danger/30 bg-danger/10 p-3">
            <ShieldOff size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-danger" />
            <p className="text-xs text-charcoal">
              <span className="font-medium">Encryption is not configured.</span> Set
              SECRETS_MASTER_KEY on the control plane to store secrets. Pipelines for
              repositories that already have secrets fail closed until the key returns.
            </p>
          </div>
        )}
        {typeof stale === 'number' && stale > 0 && (
          <div className="mb-2 flex items-start gap-2 rounded border border-steel/20 bg-surface p-3">
            <ShieldOff size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-steel" />
            <p className="text-xs text-charcoal">
              <span className="font-medium">
                {stale} secret{stale === 1 ? '' : 's'} not rotated in {staleAfterDays ?? 90}+ days.
              </span>{' '}
              Rotate by replacing the value — reruns pick up the new value automatically.
            </p>
          </div>
        )}
        <ul className="divide-y divide-steel/10">
          <PostureRow
            icon={FileKey2}
            title="Envelope encryption at rest"
            detail="Each value is encrypted with AES-256-GCM under its own key, which is wrapped by the deployment master key. The database only ever holds ciphertext."
          />
          <PostureRow
            icon={EyeOff}
            title="Write-only values"
            detail="No API or screen can show a value after creation — not even to owners. Replace a value to rotate it; losing the master key is unrecoverable by design."
          />
          <PostureRow
            icon={ShieldCheck}
            title="Masked in every log"
            detail="Values are decrypted only while dispatching a job, injected into the signed payload, and registered as log masks first — the browser and the database never see an unmasked byte."
          />
          <PostureRow
            icon={ScrollText}
            title="Immutable audit trail"
            detail="Every create, replace, and delete is recorded with actor and time. Audit metadata never includes the value."
          />
        </ul>
      </CardBody>
    </Card>
  );
}
