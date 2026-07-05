<script lang="ts">
  interface Server {
    id: string;
    provider: string;
    region: string;
    ip: string;
    status: "active" | "provisioning" | "error";
  }

  let servers = $state<Server[]>([]);

  async function destroyServer(id: string) {
    // Will be wired to Tauri invoke() in a future phase
    servers = servers.filter((s) => s.id !== id);
  }
</script>

<div class="server-list">
  <h2>Servers</h2>
  {#if servers.length === 0}
    <p class="empty">No servers provisioned</p>
  {:else}
    {#each servers as server (server.id)}
      <div class="server-item">
        <span class="provider-badge">{server.provider}</span>
        <span class="region">{server.region}</span>
        <span class="ip">{server.ip}</span>
        <span class="status {server.status}">{server.status}</span>
        <button class="destroy-btn" onclick={() => destroyServer(server.id)}>
          Destroy
        </button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .server-list {
    padding: 1rem;
  }
  .server-list h2 {
    margin-bottom: 0.75rem;
    font-size: 1.1rem;
  }
  .empty {
    color: var(--text-secondary);
    font-style: italic;
  }
  .server-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: var(--bg-secondary);
    border-radius: 6px;
    margin-bottom: 0.5rem;
  }
  .provider-badge {
    background: var(--accent);
    color: white;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-size: 0.8rem;
    font-weight: 600;
  }
  .region { color: var(--text-secondary); font-size: 0.9rem; }
  .ip { font-family: monospace; font-size: 0.9rem; }
  .status { font-size: 0.8rem; }
  .status.active { color: var(--success); }
  .status.error { color: var(--danger); }
  .destroy-btn {
    margin-left: auto;
    padding: 0.3rem 0.75rem;
    border: 1px solid var(--danger);
    border-radius: 4px;
    background: transparent;
    color: var(--danger);
    font-size: 0.8rem;
  }
  .destroy-btn:hover {
    background: var(--danger);
    color: white;
  }
</style>
