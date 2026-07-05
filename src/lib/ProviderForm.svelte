<script lang="ts">
  type Provider = "digitalocean" | "hetzner" | "oracle";

  let selectedProvider = $state<Provider>("digitalocean");
  let token = $state("");
  let region = $state("");
  let size = $state("");
  let error = $state<string | null>(null);

  const providers: { value: Provider; label: string }[] = [
    { value: "digitalocean", label: "DigitalOcean" },
    { value: "hetzner", label: "Hetzner" },
    { value: "oracle", label: "Oracle Cloud" },
  ];

  async function handleProvision() {
    if (!token.trim()) {
      error = "API token is required";
      return;
    }
    error = null;
    // Will be wired to Tauri invoke() in a future phase
  }
</script>

<div class="provider-form">
  <h2>Provision Server</h2>

  <label>
    Provider
    <select bind:value={selectedProvider}>
      {#each providers as p}
        <option value={p.value}>{p.label}</option>
      {/each}
    </select>
  </label>

  <label>
    API Token
    <input type="password" bind:value={token} placeholder="Enter provider API token" />
  </label>

  <label>
    Region
    <input type="text" bind:value={region} placeholder="e.g. fra1" />
  </label>

  <label>
    Size
    <input type="text" bind:value={size} placeholder="e.g. s-1vcpu-1gb" />
  </label>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <button onclick={handleProvision}>Provision</button>
</div>

<style>
  .provider-form {
    padding: 1rem;
    max-width: 400px;
  }
  .provider-form h2 {
    margin-bottom: 0.75rem;
    font-size: 1.1rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin-bottom: 0.75rem;
    font-size: 0.9rem;
    color: var(--text-secondary);
  }
  input, select {
    padding: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 0.9rem;
  }
  .error {
    color: var(--danger);
    font-size: 0.85rem;
    margin-bottom: 0.5rem;
  }
  button {
    padding: 0.6rem 1.5rem;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: white;
    font-weight: 600;
  }
  button:hover {
    background: var(--accent-hover);
  }
</style>
