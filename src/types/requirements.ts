/**
 * Detected-requirements types — workflow-YAML references to secrets, vars,
 * and environments captured at sync time. Names only, never values.
 */

export interface RequirementReference {
  repositoryId: string;
  repositoryName: string;
  workflowId: string;
  workflowPath: string;
}

/** A workflow reference within the entry's repository. */
export interface RequirementWorkflowRef {
  workflowId: string;
  workflowPath: string;
}

/**
 * One detected (name, repository) pair with its configured state —
 * `configuredId` is the covering secret's / matching environment's id.
 */
export interface DetectedRequirement {
  name: string;
  repositoryId: string;
  repositoryName: string;
  configured: boolean;
  configuredId: string | null;
  /** True total — `references` is capped server-side (20 per entry). */
  referenceCount: number;
  references: RequirementWorkflowRef[];
}
