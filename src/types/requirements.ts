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

export interface DetectedRequirement {
  name: string;
  /** True total — `references` is capped server-side (20 per name). */
  referenceCount: number;
  references: RequirementReference[];
}
