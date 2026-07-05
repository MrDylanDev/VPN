<script lang="ts">
  interface Step {
    label: string;
    status: "pending" | "active" | "done" | "error";
  }

  let steps = $state<Step[]>([]);
  let visible = $derived(steps.length > 0);
</script>

{#if visible}
  <div class="progress-indicator">
    {#each steps as step}
      <div class="step {step.status}">
        <span class="step-icon">
          {#if step.status === "done"}✓
          {:else if step.status === "error"}✗
          {:else if step.status === "active"}◌
          {:else}○
          {/if}
        </span>
        <span class="step-label">{step.label}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .progress-indicator {
    padding: 1rem;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0;
    font-size: 0.9rem;
  }
  .step-icon {
    width: 20px;
    text-align: center;
    font-weight: bold;
  }
  .step.pending { color: var(--text-secondary); }
  .step.active { color: var(--accent); font-weight: 600; }
  .step.done { color: var(--success); }
  .step.error { color: var(--danger); }
</style>
