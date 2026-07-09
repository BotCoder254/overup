import { zodResolver } from '@hookform/resolvers/zod';
import { ArrowRight } from 'lucide-react';
import { useForm } from 'react-hook-form';
import { Navigate } from 'react-router-dom';
import { Button } from '../../../components/ui/Button';
import { FormField } from '../../../components/ui/FormField';
import { Input } from '../../../components/ui/Input';
import { Textarea } from '../../../components/ui/Textarea';
import { slugPreview } from '../../../lib/slug';
import { SlugPreview } from '../../workspace/components/SlugPreview';
import { useCreateWorkspace } from '../../workspace/hooks/useCreateWorkspace';
import { createWorkspaceSchema, type CreateWorkspaceInput } from '../../workspace/schema';
import { AuthSplitLayout } from '../components/AuthSplitLayout';
import { useMe } from '../hooks/useAuth';

/**
 * First-time onboarding: every resource on the platform lives inside a
 * workspace, so a freshly signed-in user creates one before anything else.
 * Only the name and optional description are submitted — the backend
 * validates everything and generates the canonical slug.
 */
export function OnboardingPage() {
  const { data: me } = useMe();
  const create = useCreateWorkspace();
  const {
    register,
    handleSubmit,
    watch,
    formState: { errors },
  } = useForm<CreateWorkspaceInput>({
    resolver: zodResolver(createWorkspaceSchema),
    defaultValues: { name: '', description: '' },
  });

  // Already provisioned (e.g. deep link back to /onboarding) — skip ahead.
  if (me?.workspace) return <Navigate to={`/w/${me.workspace.slug}`} replace />;

  const slug = slugPreview(watch('name') ?? '');

  const onSubmit = (values: CreateWorkspaceInput) =>
    create.mutate({ name: values.name, description: values.description || undefined });

  return (
    <AuthSplitLayout>
      <div className="animate-fade-in space-y-8">
        <div className="space-y-3">
          <h1 className="text-2xl font-semibold tracking-tight sm:text-3xl">
            Create your workspace
          </h1>
          <p className="leading-relaxed text-steel">
            A workspace is where your repositories, runners, and pipelines live. Every automation
            project needs one before anything can be connected.
          </p>
        </div>

        <form onSubmit={handleSubmit(onSubmit)} noValidate className="space-y-6">
          <div className="space-y-2">
            <FormField id="workspace-name" label="Workspace name" error={errors.name?.message}>
              {(aria) => (
                <Input
                  {...aria}
                  autoFocus
                  autoComplete="organization"
                  placeholder="My Automation Platform"
                  {...register('name')}
                />
              )}
            </FormField>
            <SlugPreview slug={slug} />
          </div>

          <FormField
            id="workspace-description"
            label="Description"
            optional
            error={errors.description?.message}
          >
            {(aria) => (
              <Textarea
                {...aria}
                placeholder="What will this workspace be used for?"
                {...register('description')}
              />
            )}
          </FormField>

          <Button
            type="submit"
            size="lg"
            className="w-full"
            isLoading={create.isPending}
            disabled={create.isPending}
          >
            Create workspace
            {!create.isPending && <ArrowRight className="h-5 w-5" aria-hidden="true" />}
          </Button>
        </form>
      </div>
    </AuthSplitLayout>
  );
}
