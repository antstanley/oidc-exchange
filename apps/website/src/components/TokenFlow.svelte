<script>
  let step = $state(0);
  const steps = [
    { label: 'Client', desc: 'User clicks "Sign in with Google"' },
    { label: 'Provider', desc: 'Google returns authorization code' },
    { label: 'POST /token', desc: 'Client sends code to oidc-exchange' },
    { label: 'Validate', desc: 'Service validates ID token (sig, iss, aud, exp)' },
    { label: 'User', desc: 'Look up or create user, check registration policy' },
    { label: 'Issue', desc: 'Sign JWT access token + generate refresh token' },
    { label: 'Response', desc: 'Return { access_token, refresh_token, expires_in }' },
  ];

  function next() {
    step = (step + 1) % steps.length;
  }

  function prev() {
    step = (step - 1 + steps.length) % steps.length;
  }
</script>

<div class="flow-container">
  <div class="flow-steps">
    {#each steps as s, i}
      <div class="flow-step" class:active={i === step} class:done={i < step}>
        <span class="step-num">{i + 1}</span>
        <span class="step-label">{s.label}</span>
      </div>
      {#if i < steps.length - 1}
        <div class="flow-arrow" class:active={i < step}>&rarr;</div>
      {/if}
    {/each}
  </div>
  <div class="flow-detail">
    <p><strong>Step {step + 1}:</strong> {steps[step].desc}</p>
  </div>
  <div class="flow-controls">
    <button onclick={prev}>&larr; Previous</button>
    <button onclick={next}>Next &rarr;</button>
  </div>
</div>

<style>
  .flow-container {
    border: 1px solid var(--sl-color-gray-5);
    border-radius: 0.5rem;
    padding: 1.5rem;
    margin: 1rem 0;
    background: var(--sl-color-gray-7);
  }
  .flow-steps {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    overflow-x: auto;
    padding-bottom: 0.5rem;
    flex-wrap: wrap;
    justify-content: center;
  }
  .flow-step {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.6rem;
    border-radius: 0.35rem;
    font-size: 0.85rem;
    background: var(--sl-color-gray-6);
    color: var(--sl-color-gray-2);
    transition: all 0.2s;
    white-space: nowrap;
  }
  .flow-step.active {
    background: var(--sl-color-accent);
    color: var(--sl-color-accent-high);
    font-weight: 600;
  }
  .flow-step.done {
    opacity: 0.6;
  }
  .step-num {
    font-weight: 700;
    font-size: 0.75rem;
  }
  .flow-arrow {
    color: var(--sl-color-gray-4);
    font-size: 0.8rem;
  }
  .flow-arrow.active {
    color: var(--sl-color-accent);
  }
  .flow-detail {
    margin-top: 1rem;
    padding: 0.75rem;
    background: var(--sl-color-gray-6);
    border-radius: 0.35rem;
    min-height: 2.5rem;
  }
  .flow-detail p {
    margin: 0;
  }
  .flow-controls {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.75rem;
    justify-content: center;
  }
  .flow-controls button {
    padding: 0.35rem 0.75rem;
    border: 1px solid var(--sl-color-gray-4);
    border-radius: 0.25rem;
    background: var(--sl-color-gray-6);
    color: var(--sl-color-white);
    cursor: pointer;
    font-size: 0.85rem;
  }
  .flow-controls button:hover {
    background: var(--sl-color-gray-5);
  }
</style>
