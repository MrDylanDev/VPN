<script lang="ts">
  let status = $state<"disconnected" | "connecting" | "connected">("disconnected");
  let handshakeTime = $state<string | null>(null);
  let bytesSent = $state<number>(0);
  let bytesReceived = $state<number>(0);
</script>

<div class="status-bar">
  <span class="indicator {status}"></span>
  <span class="status-text">{status}</span>
  {#if handshakeTime}
    <span class="meta">Handshake: {handshakeTime}</span>
  {/if}
  {#if status === "connected"}
    <span class="meta">↑ {bytesSent} B ↓ {bytesReceived} B</span>
  {/if}
</div>

<style>
  .status-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
  }
  .indicator {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .indicator.disconnected { background: var(--danger); }
  .indicator.connecting { background: var(--warning); }
  .indicator.connected { background: var(--success); }
  .status-text {
    font-weight: 600;
    text-transform: capitalize;
  }
  .meta {
    color: var(--text-secondary);
    font-size: 0.85em;
    margin-left: auto;
  }
</style>
