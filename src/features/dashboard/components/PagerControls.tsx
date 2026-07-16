import { ChevronLeft, ChevronRight } from 'lucide-react';
import { Button } from '../../../components/ui/Button';

interface PagerControlsProps {
  /** e.g. "1–5 of 12" — rendered in the muted steel caption style. */
  label: string;
  hasPrev: boolean;
  hasNext: boolean;
  onPrev: () => void;
  onNext: () => void;
  /** True while the next keyset page is being fetched. */
  loadingNext?: boolean;
}

/**
 * Compact page-by-page controls for the dashboard panels (5 rows per page).
 * Same control language as the rest of the shell: small secondary buttons,
 * steel caption text.
 */
export function PagerControls({
  label,
  hasPrev,
  hasNext,
  onPrev,
  onNext,
  loadingNext,
}: PagerControlsProps) {
  return (
    <div className="mt-3 flex items-center justify-between gap-2">
      <span className="text-xs text-steel">{label}</span>
      <div className="flex items-center gap-1">
        <Button size="sm" variant="secondary" disabled={!hasPrev} onClick={onPrev}>
          <ChevronLeft size={14} aria-hidden="true" />
          Prev
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={!hasNext}
          isLoading={loadingNext}
          onClick={onNext}
        >
          Next
          <ChevronRight size={14} aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
