<script lang="ts">
  import { goto } from '$app/navigation';
  import { exchangeToken } from '$lib/auth';

  let { data } = $props();
  let buttonContainer: HTMLElement;

  $effect(() => {
    if (typeof google !== 'undefined' && buttonContainer) {
      google.accounts.id.initialize({
        client_id: data.googleClientId,
        callback: handleCredentialResponse
      });
      google.accounts.id.renderButton(buttonContainer, {
        theme: 'outline',
        size: 'large',
        width: 300
      });
    }
  });

  async function handleCredentialResponse(response: { credential: string }) {
    const result = await exchangeToken(response.credential);
    if (result.ok) {
      goto('/authenticated');
    } else {
      goto('/denied');
    }
  }
</script>

<h1>OIDC Exchange Demo</h1>
<p>Sign in to test the OIDC token exchange service.</p>

<div bind:this={buttonContainer}></div>

<noscript>JavaScript is required for Google Sign-In.</noscript>
