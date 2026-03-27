<script lang="ts">
	import '../app.css';

	let { children } = $props();
</script>

<svelte:head>
	<title>oidc-exchange admin</title>
</svelte:head>

{@render children()}
