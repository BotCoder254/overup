import { Squirrel } from 'lucide-react';
import { AppShell } from '../../../components/layout/AppShell';
import { EmptyState } from '../../../components/ui/EmptyState';

export function DashboardPage() {
  return (
    <AppShell>
      <EmptyState
        icon={Squirrel}
        title="Pipelines are coming soon"
        description="Your workspace is ready. Connect a repository to start building — pipeline runs, runners, and artifacts will appear here."
      />
    </AppShell>
  );
}
