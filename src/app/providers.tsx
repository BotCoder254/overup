import { QueryClientProvider } from '@tanstack/react-query';
import { TriangleAlert } from 'lucide-react';
import type { ReactNode } from 'react';
import { ErrorBoundary } from 'react-error-boundary';
import { Toaster } from 'sonner';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { queryClient } from '../lib/queryClient';

function ErrorFallback() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-canvas p-4 sm:p-8">
      <EmptyState
        icon={TriangleAlert}
        title="Something went wrong"
        description="An unexpected error occurred. Reload the page to continue."
        action={<Button onClick={() => window.location.reload()}>Reload</Button>}
        className="w-full max-w-lg border-0 min-h-0"
      />
    </div>
  );
}

export function Providers({ children }: { children: ReactNode }) {
  return (
    <ErrorBoundary FallbackComponent={ErrorFallback}>
      <QueryClientProvider client={queryClient}>
        {children}
        <Toaster position="top-right" />
      </QueryClientProvider>
    </ErrorBoundary>
  );
}
