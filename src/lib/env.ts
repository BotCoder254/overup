const isDevelopment = process.env.NODE_ENV === 'development';

export const env = {
  /**
   * Absolute origin of the Rust backend, used only for full-page
   * navigations (the GitHub OAuth redirect). XHR calls stay relative and
   * go through the CRA dev proxy / same-origin reverse proxy in prod.
   */
  apiOrigin:
    process.env.REACT_APP_API_ORIGIN ?? (isDevelopment ? 'http://localhost:8080' : ''),
} as const;
