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
        <div className="space-y-3">
          <h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">
            A modern CI/CD platform for developers
          </h1>
          <p className="leading-relaxed text-steel">
            Overup uses GitHub as its exclusive identity provider. Connect
            your account to build, test, and ship straight from your
            repositories.
          </p>
        </div>

        <div className="space-y-4">
          <Button
            size="lg"
            className="w-full"
            onClick={continueWithGitHub}
            isLoading={isRedirecting}
          >
            {!isRedirecting && <GitHubMark className="h-5 w-5" />}
            Continue with GitHub
          </Button>
          <p className="text-sm leading-relaxed text-steel">
            We never see or store your GitHub password — authentication
            happens directly with GitHub.
          </p>
        </div>

        <p className="text-sm leading-relaxed text-steel">
          First time here? A workspace is created for you automatically after
          your first successful sign-in.
        </p>
      </div>
    </AuthSplitLayout>
  );
}
