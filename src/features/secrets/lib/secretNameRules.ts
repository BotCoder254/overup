/**
 * Client-side mirror of the server's secret-name validation (the server
 * remains the authority). Names are UPPER_SNAKE_CASE; platform prefixes and
 * well-known environment variables are reserved. Shared by the create
 * dialog and the detected-requirements card (which uses it to mark
 * YAML-referenced names that can never become overup secrets).
 */

export const SECRET_NAME_PATTERN = /^[A-Z_][A-Z0-9_]*$/;
export const RESERVED_PREFIXES = ['OVERUP_', 'GITHUB_', 'RUNNER_', 'DOCKER_'];
export const RESERVED_NAMES = [
  'CI',
  'PATH',
  'HOME',
  'SHELL',
  'HOSTNAME',
  'LANG',
  'PWD',
  'USER',
  'TMPDIR',
  'LD_PRELOAD',
  'LD_LIBRARY_PATH',
];

/** Why `name` can't be an overup secret, or undefined when it can (or is empty). */
export function secretNameProblem(name: string): string | undefined {
  if (!name) return undefined;
  if (name.length > 200) return 'At most 200 characters.';
  if (!SECRET_NAME_PATTERN.test(name)) {
    return 'UPPER_SNAKE_CASE only: letters A-Z, digits, underscores; not starting with a digit.';
  }
  const prefix = RESERVED_PREFIXES.find((p) => name.startsWith(p));
  if (prefix) return `The ${prefix} prefix is reserved for the platform.`;
  if (RESERVED_NAMES.includes(name)) return `${name} is a reserved environment variable name.`;
  return undefined;
}

export function isConfigurableSecretName(name: string): boolean {
  return name.length > 0 && secretNameProblem(name) === undefined;
}
