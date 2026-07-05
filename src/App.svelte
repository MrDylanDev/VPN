<script lang="ts">
  import "./app.css";
  import StatusBar from "./lib/StatusBar.svelte";
  import ConnectButton from "./lib/ConnectButton.svelte";
  import ServerList from "./lib/ServerList.svelte";
  import ProviderForm from "./lib/ProviderForm.svelte";
  import ProgressIndicator from "./lib/ProgressIndicator.svelte";
</script>

<main>
  <StatusBar />
  <ConnectButton />
  <ServerList />
  <ProviderForm />
  <ProgressIndicator />
</main>
