<script lang="ts">
    import { enhance } from '$app/forms';
    let { form } = $props();
</script>

<div class="min-h-screen flex items-center justify-center bg-gray-950">
  <div class="bg-gray-900 rounded-xl p-8 shadow-2xl max-w-md w-full mx-4">
    <h1 class="text-2xl font-bold text-white text-center mb-2">oidc-exchange</h1>
    <p class="text-gray-400 text-center mb-8">Admin Console</p>

    {#if form?.error}
      <div class="bg-red-900/50 border border-red-700 text-red-300 rounded-lg p-3 mb-4 text-sm">
        {form.error}
      </div>
    {/if}

    <form method="POST" use:enhance>
      <label class="block mb-4">
        <span class="text-gray-400 text-sm">Access Token</span>
        <textarea
          name="token"
          rows="4"
          class="mt-1 w-full bg-gray-800 border border-gray-700 rounded-lg p-3 text-white text-sm font-mono focus:outline-none focus:border-blue-500"
          placeholder="Paste your JWT access token..."
        ></textarea>
      </label>
      <p class="text-gray-500 text-xs mb-4">
        Obtain a token via POST /token with an account that has the admin role claim.
      </p>
      <button type="submit" class="w-full bg-blue-600 hover:bg-blue-700 text-white font-medium py-3 px-4 rounded-lg transition-colors">
        Sign in
      </button>
    </form>
  </div>
</div>
