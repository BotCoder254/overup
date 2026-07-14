import { useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Card, CardBody, CardHeader } from '../../../components/ui/Card';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import type { Me } from '../../../types/user';
import { useUpdateMe } from '../hooks/useSettings';

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/**
 * Profile settings. The avatar is read-only by design — GitHub is the source
 * of truth for identity and refreshes it on every sign-in. Display name and
 * email are user-editable; once edited they stop mirroring GitHub.
 */
export function ProfileTab({ me }: { me: Me }) {
  const updateMe = useUpdateMe();
  const [displayName, setDisplayName] = useState(me.displayName ?? '');
  const [email, setEmail] = useState(me.email ?? '');

  useEffect(() => {
    setDisplayName(me.displayName ?? '');
    setEmail(me.email ?? '');
  }, [me.displayName, me.email]);

  const name = me.displayName ?? me.username;
  const trimmedName = displayName.trim();
  const trimmedEmail = email.trim();
  const nameProblem =
    trimmedName.length > 80 ? 'At most 80 characters.' : undefined;
  const emailProblem =
    trimmedEmail && !EMAIL_PATTERN.test(trimmedEmail)
      ? 'Enter a valid email address.'
      : undefined;

  const dirty =
    trimmedName !== (me.displayName ?? '') || trimmedEmail !== (me.email ?? '');
  const invalid = Boolean(nameProblem || emailProblem) || !trimmedName;

  const save = () => {
    if (invalid || !dirty || updateMe.isPending) return;
    updateMe.mutate({
      ...(trimmedName !== (me.displayName ?? '') ? { displayName: trimmedName } : {}),
      ...(trimmedEmail && trimmedEmail !== (me.email ?? '') ? { email: trimmedEmail } : {}),
    });
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <h2 className="text-sm font-semibold text-charcoal">Profile image</h2>
        </CardHeader>
        <CardBody>
          <div className="flex items-start gap-4">
            {me.avatarUrl ? (
              <img
                src={me.avatarUrl}
                alt=""
                referrerPolicy="no-referrer"
                className="h-20 w-20 shrink-0 rounded border border-steel/20 object-cover"
              />
            ) : (
              <span
                aria-hidden="true"
                className="flex h-20 w-20 shrink-0 items-center justify-center rounded border border-steel/20 bg-primary/10 text-2xl font-semibold text-primary"
              >
                {name.charAt(0).toUpperCase()}
              </span>
            )}
            <div className="min-w-0 text-sm text-steel">
              <p className="font-medium text-charcoal">Managed by GitHub</p>
              <p className="mt-1">
                Your profile image mirrors your GitHub account and refreshes on every
                sign-in. Change it on GitHub and it follows here — it cannot be uploaded
                or removed on this side.
              </p>
            </div>
          </div>
        </CardBody>
      </Card>

      <Card>
        <CardHeader>
          <h2 className="text-sm font-semibold text-charcoal">Profile information</h2>
        </CardHeader>
        <CardBody>
          <div className="space-y-4">
            <FormField
              id="profile-username"
              label="Username"
              hint="Your GitHub login — it identifies you across the workspace and cannot be changed here."
            >
              {(aria) => (
                <Input {...aria} value={`@${me.username}`} disabled className="font-mono" />
              )}
            </FormField>

            <FormField id="profile-display-name" label="Display name" error={nameProblem}>
              {(aria) => (
                <Input
                  {...aria}
                  autoComplete="name"
                  maxLength={120}
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                />
              )}
            </FormField>

            <FormField
              id="profile-email"
              label="Email"
              hint="Once you set an email here it stops mirroring GitHub."
              error={emailProblem}
            >
              {(aria) => (
                <Input
                  {...aria}
                  type="email"
                  autoComplete="email"
                  maxLength={254}
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                />
              )}
            </FormField>

            <div className="flex justify-end">
              <Button
                size="sm"
                onClick={save}
                disabled={invalid || !dirty}
                isLoading={updateMe.isPending}
              >
                Save changes
              </Button>
            </div>
          </div>
        </CardBody>
      </Card>
    </div>
  );
}
