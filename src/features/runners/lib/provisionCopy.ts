/**
 * Static copy per provisioning failure category. The backend only ever
 * sends these fixed strings — never raw Docker output — so the map is the
 * single translation point for the wizard and the runner detail surfaces.
 */
export const PROVISION_FAILURE_COPY: Record<string, string> = {
  image_pull_failed:
    'The server could not pull the runner image. Check RUNNER_IMAGE and registry access on the server, then try again.',
  container_create_failed:
    'The server could not create the runner container. Check the Docker daemon on the server, then try again.',
  container_start_failed:
    'The runner container failed to start. Check its logs on the server, then try again.',
  provision_timeout:
    'Provisioning did not finish within the 10-minute budget — usually a very slow image pull. Try again.',
  bootstrap_arm_failed:
    'The server could not prepare the runner credential. Try again.',
  container_missing:
    'The runner container disappeared from the Docker host. Revoke this runner and create a new one.',
  docker_unavailable:
    "The server lost its Docker connection while provisioning. It reconnects automatically — try again shortly.",
  revoked: 'This runner was revoked before it connected.',
};

/** Quota-refusal copy shared by the wizard pre-check and the 409 handler. */
export const HOSTED_QUOTA_COPY =
  'Hosted runner limit reached for this deployment. Revoke an existing hosted runner to free a slot.';

/** 409 `hosted_runner_unavailable`: Docker is down server-side right now. */
export const HOSTED_UNAVAILABLE_COPY =
  "The server's Docker daemon is unreachable right now, so hosted runners are temporarily unavailable. It reconnects automatically — try again shortly.";
