import { z } from 'zod';

/**
 * UX mirror of the server rules only — the Rust backend re-validates
 * everything with Unicode normalization and character allow-lists and is
 * the sole authority.
 */
export const createWorkspaceSchema = z.object({
  name: z
    .string()
    .trim()
    .min(2, 'Workspace name must be at least 2 characters')
    .max(80, 'Workspace name must be at most 80 characters'),
  description: z
    .string()
    .trim()
    .max(500, 'Description must be at most 500 characters')
    .optional()
    .or(z.literal('')),
});

export type CreateWorkspaceInput = z.infer<typeof createWorkspaceSchema>;
