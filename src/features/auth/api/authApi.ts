import { HTTPError } from 'ky';
import { api } from '../../../lib/api';
import type { Me } from '../../../types/user';

/** Resolve the current session, or null when signed out (401). */
export async function getMe(): Promise<Me | null> {
  try {
    return await api.get('/api/me').json<Me>();
  } catch (error) {
    if (error instanceof HTTPError && error.response.status === 401) {
      return null;
    }
    throw error;
  }
}

export async function logout(): Promise<void> {
  await api.post('/auth/logout');
}
