import type { RequestHandler } from './$types';

const AUTH_ENDPOINT = process.env.AUTH_ENDPOINT || 'http://localhost:8080/auth';

export const POST: RequestHandler = async ({ request, cookies }) => {
  const { credential } = await request.json();

  if (!credential) {
    return new Response(JSON.stringify({ error: 'missing credential' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' }
    });
  }

  const tokenResponse = await fetch(`${AUTH_ENDPOINT}/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'id_token',
      id_token: credential,
      provider: 'google'
    })
  });

  if (!tokenResponse.ok) {
    const error = await tokenResponse.text();
    return new Response(error, {
      status: tokenResponse.status >= 500 ? 500 : 401,
      headers: { 'Content-Type': 'application/json' }
    });
  }

  const tokens = await tokenResponse.json();

  cookies.set('access_token', tokens.access_token, {
    path: '/',
    httpOnly: true,
    secure: true,
    sameSite: 'strict',
    maxAge: tokens.expires_in
  });

  if (tokens.refresh_token) {
    cookies.set('refresh_token', tokens.refresh_token, {
      path: '/',
      httpOnly: true,
      secure: true,
      sameSite: 'strict',
      maxAge: 60 * 60 * 24 * 30
    });
  }

  return new Response(JSON.stringify({ ok: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
  });
};
