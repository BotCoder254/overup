/** One active session row from `GET /api/me/sessions`. */
export interface UserSession {
  id: string;
  createdAt: string;
  lastSeenAt: string | null;
  expiresAt: string;
  /** Masked server-side (IPv4: last octet hidden). */
  ip: string | null;
  userAgent: string | null;
  /** True for the session making the request. */
  current: boolean;
}
