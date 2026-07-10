const isDevelopment = process.env.NODE_ENV === 'development';

export const env = {
  /**
   * Absolute origin of the Rust backend, used only for full-page
   * navigations (the GitHub OAuth redirect). XHR calls stay relative and
   * go through the CRA dev proxy / same-origin reverse proxy in prod.
   */
  apiOrigin:
    process.env.REACT_APP_API_ORIGIN ?? (isDevelopment ? 'http://localhost:8080' : ''),
  /**
   * Absolute origin for browser WebSockets. Empty (default) = same origin.
   * Set it on deployments whose reverse proxy cannot forward WS upgrades
   * (e.g. Netlify): the app then mints a one-time ticket over the proxied
   * REST API and connects the socket directly to this origin.
   */
  wsOrigin: process.env.REACT_APP_WS_ORIGIN ?? '',
} as const;
