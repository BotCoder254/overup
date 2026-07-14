/** Shapes returned by the workflow management API. */

export type ValidationStatus = 'valid' | 'warnings' | 'errors';

export interface WorkflowSummary {
  id: string;
  repositoryId: string;
  repoFullName: string;
  repoOwner: string;
  repoOwnerAvatarUrl: string | null;
  path: string;
  name: string;
  triggers: string[];
  validationStatus: ValidationStatus;
  jobCount: number;
  lastCommitSha: string | null;
  lastCommitAt: string | null;
  updatedAt: string;
}

export interface WorkflowJob {
  key: string;
  name: string | null;
  runsOn: string[];
  needs: string[];
  uses: string | null;
  strategy: unknown;
  stepCount: number;
}

export interface Diagnostic {
  severity: 'error' | 'warning';
  message: string;
  path?: string;
  line?: number;
}

/** One `on.workflow_dispatch.inputs` entry parsed server-side. */
export interface WorkflowDispatchInput {
  name: string;
  type: 'string' | 'number' | 'boolean' | 'choice' | 'environment';
  required: boolean;
  default?: string;
  description?: string;
  options?: string[];
}

export interface WorkflowDetail extends WorkflowSummary {
  defaultBranch: string;
  fileSize: number;
  rawContent: string;
  metadata: {
    permissions?: unknown;
    concurrency?: unknown;
    envKeys?: string[];
    secretRefs?: string[];
    /** Absent on workflows synced before dispatch-input parsing shipped. */
    dispatchInputs?: WorkflowDispatchInput[];
  };
  validationErrors: Diagnostic[];
  lastCommitMessage: string | null;
  jobs: WorkflowJob[];
}

/** Response of the offline validation endpoint backing the editor. */
export interface ValidateResponse {
  status: ValidationStatus;
  diagnostics: Diagnostic[];
  triggers: string[];
  jobs: Array<{
    key: string;
    name: string | null;
    needs: string[];
    runsOn: string[];
    uses: string | null;
    stepCount: number;
  }>;
}
