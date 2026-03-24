<script lang="ts">
  import { goto } from '$app/navigation';

  let { data } = $props();

  async function logout() {
    await fetch('/api/logout', { method: 'POST' });
    goto('/');
  }
</script>

<h1>Authenticated</h1>
<p>You are signed in.</p>

<dl>
  <dt>Subject</dt>
  <dd><code>{data.user.sub}</code></dd>

  {#if data.user.email}
    <dt>Email</dt>
    <dd>{data.user.email}</dd>
  {/if}

  <dt>Issued At</dt>
  <dd>{new Date(data.user.iat * 1000).toLocaleString()}</dd>

  <dt>Expires</dt>
  <dd>{new Date(data.user.exp * 1000).toLocaleString()}</dd>

  {#if Object.keys(data.user.claims).length > 0}
    <dt>Custom Claims</dt>
    <dd><pre>{JSON.stringify(data.user.claims, null, 2)}</pre></dd>
  {/if}
</dl>

<button onclick={logout}>Sign Out</button>
