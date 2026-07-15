import { useQuery } from '@tanstack/react-query';
import { useDebouncedValue } from '../../../lib/useDebouncedValue';
import { checkWorkspaceAvailability } from '../api/workspaceApi';

export type AvailabilityStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'taken'
  | 'invalid';

export interface WorkspaceAvailabilityState {
  status: AvailabilityStatus;
  /** Clean slug the backend derived (empty until a check resolves). */
  slug: string;
  /** When taken, the slug that would actually be assigned. */
  adjustedSlug: string | null;
}

/** Names shorter than this never hit the backend (matches NAME_MIN_CHARS). */
const MIN_CHARS = 2;
const DEBOUNCE_MS = 400;

/**
 * Live, debounced workspace-name availability. Purely advisory: the create
 * flow never blocks on it, and Settings keeps the slug immutable — this only
 * drives the spinner + "Available / will be saved as …" hint. `checking` is
 * true while the user is still typing (raw value ahead of the debounced one)
 * OR the query is in flight, so the spinner appears immediately.
 */
export function useWorkspaceAvailability(rawName: string): WorkspaceAvailabilityState {
  const trimmed = rawName.trim();
  const debounced = useDebouncedValue(trimmed, DEBOUNCE_MS);
  const enabled = debounced.length >= MIN_CHARS;

  const query = useQuery({
    queryKey: ['workspaceAvailability', debounced],
    queryFn: () => checkWorkspaceAvailability(debounced),
    enabled,
    staleTime: 30_000,
    retry: false,
  });

  if (trimmed.length === 0) {
    return { status: 'idle', slug: '', adjustedSlug: null };
  }
  if (trimmed.length < MIN_CHARS) {
    return { status: 'invalid', slug: '', adjustedSlug: null };
  }
  // Still settling the debounce, or a fetch is in flight.
  if (trimmed !== debounced || query.isFetching) {
    return { status: 'checking', slug: query.data?.slug ?? '', adjustedSlug: null };
  }
  if (query.isError) {
    // A 422 means the name isn't valid server-side; anything else we just
    // fall silent rather than block the form.
    return { status: 'invalid', slug: '', adjustedSlug: null };
  }
  if (query.data) {
    return {
      status: query.data.available ? 'available' : 'taken',
      slug: query.data.slug,
      adjustedSlug: query.data.adjustedSlug,
    };
  }
  return { status: 'idle', slug: '', adjustedSlug: null };
}
