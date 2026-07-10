import type { NavItem } from '../../app/navigation';
import { EmptyState } from '../ui/EmptyState';
import { PageHeader } from './PageHeader';

/**
 * Generic page for nav destinations that don't have a real feature yet.
 * EmptyState draws its own border/background — inside the content canvas that
 * would double-frame, so both are stripped here.
 */
export function PlaceholderPage({ item }: { item: NavItem }) {
  return (
    <>
      <PageHeader title={item.label} />
      <EmptyState
        icon={item.icon}
        title={`${item.label} is coming soon`}
        description={item.description}
        className="min-h-[50vh] border-0 bg-transparent"
      />
    </>
  );
}
