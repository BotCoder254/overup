import ky from 'ky';

/**
 * Single HTTP client for the overup API.
 *
 * - `credentials: 'include'` sends the HttpOnly session cookie; tokens are
 *   never readable (or stored) in JavaScript.
 * - `X-Requested-With` satisfies the backend's CSRF defense-in-depth check
 *   on state-changing requests.
 */
export const api = ky.create({
  credentials: 'include',
  retry: 0,
  headers: {
    'X-Requested-With': 'XMLHttpRequest',
  },
});
