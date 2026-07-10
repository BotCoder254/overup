import { Squirrel } from 'lucide-react';
import { PageHeader } from '../../../components/layout/PageHeader';
import { EmptyState } from '../../../components/ui/EmptyState';

export function DashboardPage() {
  return (
    <>
      <PageHeader title="Dashboard" />
      <EmptyState
        icon={Squirrel}
        title="Pipelines are coming soon"
        description="Your workspace is ready. Connect a repository to start building — pipeline runs, runners, and artifacts will appear here."
        className="min-h-[50vh] border-0 bg-transparent"
      />
    </>
  );
}
