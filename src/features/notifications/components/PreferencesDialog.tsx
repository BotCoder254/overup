import { useEffect, useState } from 'react';
import { Button } from '../../../components/ui/Button';
import { Dialog } from '../../../components/ui/Dialog';
import type {
  NotificationCategory,
  NotificationSeverity,
} from '../../../types/notification';
import {
  useNotificationPreferences,
  useSaveNotificationPreferences,
} from '../hooks/useNotifications';
import { CATEGORY_LABELS, SEVERITY_LABELS } from '../lib/notificationPresentation';

const ALL_CATEGORIES = Object.keys(CATEGORY_LABELS) as NotificationCategory[];
const ALL_SEVERITIES = Object.keys(SEVERITY_LABELS) as NotificationSeverity[];

/** Mute presets → hours; the server caps mutedUntil at 30 days. */
const MUTE_OPTIONS: { label: string; hours: number }[] = [
  { label: 'Not muted', hours: 0 },
  { label: 'For 1 hour', hours: 1 },
  { label: 'For 8 hours', hours: 8 },
  { label: 'For 24 hours', hours: 24 },
  { label: 'For 7 days', hours: 24 * 7 },
];

const controlClasses =
  'w-full rounded border border-steel/20 bg-canvas px-2.5 py-1.5 text-sm text-charcoal transition-colors hover:border-steel/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary';

interface PreferencesDialogProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Per-user delivery preferences: category toggles, minimum severity, and a
 * temporary global mute. Enforced server-side at WRITE time (fan-out), so
 * these are real filters, not display settings. Critical alerts always
 * deliver regardless — stated in the dialog so nobody is surprised.
 */
export function PreferencesDialog({ open, onClose }: PreferencesDialogProps) {
  const { data: preferences } = useNotificationPreferences();
  const save = useSaveNotificationPreferences();

  const [disabled, setDisabled] = useState<Set<NotificationCategory>>(new Set());
  const [minSeverity, setMinSeverity] = useState<NotificationSeverity>('info');
  const [muteHours, setMuteHours] = useState(0);

  // Seed local state from the server copy each time the dialog opens.
  useEffect(() => {
    if (!open || !preferences) return;
    setDisabled(new Set(preferences.disabledCategories));
    setMinSeverity(preferences.minSeverity);
    setMuteHours(0);
  }, [open, preferences]);

  const currentlyMuted =
    preferences?.mutedUntil && new Date(preferences.mutedUntil) > new Date();

  const toggleCategory = (category: NotificationCategory) => {
    setDisabled((old) => {
      const next = new Set(old);
      if (next.has(category)) next.delete(category);
      else next.add(category);
      return next;
    });
  };

  const submit = () => {
    const mutedUntil =
      muteHours > 0
        ? new Date(Date.now() + muteHours * 3600_000).toISOString()
        : null;
    save.mutate(
      {
        mutedUntil,
        disabledCategories: Array.from(disabled),
        minSeverity,
      },
      { onSuccess: onClose },
    );
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Notification settings"
      description="Choose which operational events reach you. Critical alerts always deliver."
      footer={
        <>
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" isLoading={save.isPending} onClick={submit}>
            Save preferences
          </Button>
        </>
      }
    >
      <div className="mt-4 space-y-5">
        <fieldset>
          <legend className="text-xs font-medium uppercase tracking-wider text-steel">
            Categories
          </legend>
          <div className="mt-2 grid grid-cols-2 gap-1.5">
            {ALL_CATEGORIES.map((category) => {
              const enabled = !disabled.has(category);
              return (
                <label
                  key={category}
                  className="flex cursor-pointer items-center gap-2 rounded border border-steel/20 px-2.5 py-1.5 text-sm text-charcoal transition-colors hover:bg-surface"
                >
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={() => toggleCategory(category)}
                    className="h-3.5 w-3.5 accent-primary"
                  />
                  {CATEGORY_LABELS[category]}
                </label>
              );
            })}
          </div>
        </fieldset>

        <div>
          <label
            htmlFor="notification-min-severity"
            className="text-xs font-medium uppercase tracking-wider text-steel"
          >
            Minimum severity
          </label>
          <select
            id="notification-min-severity"
            value={minSeverity}
            onChange={(event) => setMinSeverity(event.target.value as NotificationSeverity)}
            className={`mt-2 ${controlClasses}`}
          >
            {ALL_SEVERITIES.map((severity) => (
              <option key={severity} value={severity}>
                {SEVERITY_LABELS[severity]} and above
              </option>
            ))}
          </select>
        </div>

        <div>
          <label
            htmlFor="notification-mute"
            className="text-xs font-medium uppercase tracking-wider text-steel"
          >
            Mute everything
          </label>
          {currentlyMuted && preferences?.mutedUntil && (
            <p className="mt-1 text-xs text-steel">
              Currently muted until {new Date(preferences.mutedUntil).toLocaleString()}. Saving
              with “Not muted” lifts it.
            </p>
          )}
          <select
            id="notification-mute"
            value={muteHours}
            onChange={(event) => setMuteHours(Number(event.target.value))}
            className={`mt-2 ${controlClasses}`}
          >
            {MUTE_OPTIONS.map((option) => (
              <option key={option.hours} value={option.hours}>
                {option.label}
              </option>
            ))}
          </select>
        </div>
      </div>
    </Dialog>
  );
}
