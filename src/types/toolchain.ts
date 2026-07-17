/** A language-toolchain image family from the backend catalog. */
export interface Toolchain {
  /** Short alias an author types as `container: <key>`. */
  key: string;
  label: string;
  language: string;
  description: string;
  tools: string[];
  imageLatest: string;
  image2204: string;
  image2404: string;
  /** Very large image (`full-*`) — prewarming is discouraged. */
  large: boolean;
  /** The `-latest` image is warmed on runner connect (env prepull OR install). */
  prewarmed: boolean;
  /** Install lifecycle on the hosted-runner daemon. */
  installStatus: 'none' | 'pending' | 'installed' | 'failed';
  /** Static failure category when installStatus === 'failed'. */
  installError?: string;
}

export interface ToolchainsResponse {
  toolchains: Toolchain[];
  /** Image a job falls back to when it declares no container/known label. */
  defaultImage: string;
  /** Whether RUNNER_IMAGE_ALLOWLIST restricts which images may run. */
  allowlistEnabled: boolean;
  /** Whether install/uninstall is possible now (hosted provisioner reachable). */
  installSupported: boolean;
}
