import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ cookies }) => {
  const accessToken = cookies.get('access_token');

  if (!accessToken) {
    throw redirect(302, '/');
  }

  try {
    const parts = accessToken.split('.');
    if (parts.length !== 3) throw new Error('Invalid JWT');

    const payload = JSON.parse(
      Buffer.from(parts[1], 'base64url').toString('utf-8')
    );

    return {
      user: {
        sub: payload.sub,
        email: payload.email,
        exp: payload.exp,
        iat: payload.iat,
        iss: payload.iss,
        claims: Object.fromEntries(
          Object.entries(payload).filter(
            ([k]) => !['sub', 'iss', 'aud', 'iat', 'exp'].includes(k)
          )
        )
      }
    };
  } catch {
    throw redirect(302, '/');
  }
};
