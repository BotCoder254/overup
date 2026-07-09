import type { ReactNode } from 'react';
import { Logo } from '../../../components/brand/Logo';
import { ParticleField } from './ParticleField';

interface AuthSplitLayoutProps {
  children: ReactNode;
}

/**
 * The single layout shared by every authentication and onboarding page: a
 * full-height 50/50 split with no floating card and no divider between the
 * halves. Left half is the interaction panel with its content centered;
 * right half is a solid navy branding panel (hidden below `lg`) with an
 * interactive particle backdrop, the logo top-right, and the copyright
 * bottom-right.
 */
export function AuthSplitLayout({ children }: AuthSplitLayoutProps) {
  const year = new Date().getFullYear();

  return (
    <div className="grid min-h-screen grid-cols-1 lg:grid-cols-2">
      <section className="flex flex-col bg-canvas">
        <header className="p-6 sm:p-10">
          <Logo size="md" className="text-charcoal" />
        </header>
        <main className="flex flex-1 items-center justify-center">
          <div className="w-full max-w-md px-6 py-10 sm:px-10">{children}</div>
        </main>
      </section>

      <aside className="relative hidden overflow-hidden bg-navy text-white lg:block">
        <ParticleField className="absolute inset-0 h-full w-full" />

        {/* Overlay chrome must not swallow the canvas hover interaction. */}
        <div className="pointer-events-none relative z-10 flex h-full flex-col justify-between">
          <header className="flex justify-end p-10">
            <Logo size="md" />
          </header>
          <footer className="flex justify-end p-10">
            <p className="text-sm text-white/60">&copy; {year} overup. All rights reserved.</p>
          </footer>
        </div>
      </aside>
    </div>
  );
}
