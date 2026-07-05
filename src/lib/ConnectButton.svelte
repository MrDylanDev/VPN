<script lang="ts">
  let isConnected = $state(false);
  let isLoading = $state(false);

  async function handleClick() {
    isLoading = true;
    try {
      // Will be wired to Tauri invoke() in a future phase
      await new Promise((resolve) => setTimeout(resolve, 1000));
      isConnected = !isConnected;
    } finally {
      isLoading = false;
    }
  }
</script>

<button
  class="connect-btn"
  class:connected={isConnected}
  class:loading={isLoading}
  onclick={handleClick}
  disabled={isLoading}
>
  {#if isLoading}
    <span class="spinner"></span>
  {/if}
  {isConnected ? "Disconnect" : "Connect"}
</button>

<style>
  .connect-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 2rem;
    border: none;
    border-radius: 8px;
    font-size: 1rem;
    font-weight: 600;
    background: var(--accent);
    color: white;
    transition: background 0.2s;
  }
  .connect-btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .connect-btn.connected {
    background: var(--danger);
  }
  .connect-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
