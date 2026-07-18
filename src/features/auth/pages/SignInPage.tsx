import { Lock, Server, ShieldCheck, Workflow } from 'lucide-react';
import { useState } from 'react';
import { GitHubMark } from '../../../components/brand/GitHubMark';
import { Button } from '../../../components/ui/Button';
import { env } from '../../../lib/env';
import { AuthSplitLayout } from '../components/AuthSplitLayout';

export function SignInPage() {
  const [isRedirecting, setIsRedirecting] = useState(false);

  const continueWithGitHub = () => {
    setIsRedirecting(true);
    // Full-page navigation: the entire OAuth flow happens on the backend.
    // No tokens, secrets, or OAuth parameters ever exist in React.
    window.location.assign(`${env.apiOrigin}/auth/github/login`);
  };

  return (
    <AuthSplitLayout>
      <div className="animate-fade-in space-y-8">
        <div className="space-y-4">
          <h1 className="text-4xl font-semibold leading-[1.1] tracking-tight sm:text-5xl">
            Ship faster with
            <span className="block text-primary">open-source CI/CD</span>
          </h1>
          <p className="text-steel">
            Pipelines, runners, and secrets — wired straight into GitHub.
          </p>
        </div>

        <div className="space-y-3">
          <Button
            size="lg"
            className="w-full"
            onClick={continueWithGitHub}
            isLoading={isRedirecting}
          >
            {!isRedirecting && <GitHubMark className="h-5 w-5" />}
            Continue with GitHub
          </Button>
          <p className="flex items-center gap-1.5 text-xs text-steel">
            <ShieldCheck size={13} aria-hidden="true" className="shrink-0" />
            Secure OAuth — we never see your password.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-steel/10 pt-5 text-xs text-steel">
          <span className="flex items-center gap-1.5">
            <Workflow size={13} aria-hidden="true" className="text-primary" />
            Pipelines
          </span>
          <span className="flex items-center gap-1.5">
            <Server size={13} aria-hidden="true" className="text-primary" />
            Self-hosted runners
          </span>
          <span className="flex items-center gap-1.5">
            <Lock size={13} aria-hidden="true" className="text-primary" />
            Encrypted secrets
          </span>
        </div>
      </div>
    </AuthSplitLayout>
  );
}
