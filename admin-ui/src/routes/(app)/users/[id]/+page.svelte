<script lang="ts">
	import { enhance } from '$app/forms';

	let { data, form } = $props();
	let claimsText = $state(JSON.stringify(data.claims, null, 2));
	let showDeleteConfirm = $state(false);
</script>

<div class="max-w-3xl">
	<div class="flex items-center gap-3 mb-6">
		<a href="/users" class="text-gray-400 hover:text-gray-300">&larr;</a>
		<h2 class="text-xl font-semibold text-white">User Detail</h2>
		{#if data.user.status === 'Active'}
			<span class="px-2 py-0.5 rounded text-xs font-medium bg-green-500/10 text-green-400">Active</span>
		{:else if data.user.status === 'Suspended'}
			<span class="px-2 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400">Suspended</span>
		{:else}
			<span class="px-2 py-0.5 rounded text-xs font-medium bg-red-500/10 text-red-400">Deleted</span>
		{/if}
	</div>

	{#if form?.success}
		<div class="bg-green-900/30 border border-green-700 text-green-300 rounded-lg p-3 mb-4 text-sm">User updated.</div>
	{/if}

	<!-- Read-only info -->
	<div class="bg-gray-900 rounded-xl border border-gray-800 p-5 mb-4">
		<div class="grid grid-cols-2 gap-4 text-sm">
			<div>
				<span class="text-gray-500">ID</span>
				<p class="text-gray-300 font-mono text-xs mt-1">{data.user.id}</p>
			</div>
			<div>
				<span class="text-gray-500">External ID</span>
				<p class="text-gray-300 font-mono text-xs mt-1">{data.user.external_id}</p>
			</div>
			<div>
				<span class="text-gray-500">Provider</span>
				<p class="text-gray-300 mt-1">{data.user.provider}</p>
			</div>
			<div>
				<span class="text-gray-500">Created</span>
				<p class="text-gray-300 mt-1">{new Date(data.user.created_at).toLocaleString()}</p>
			</div>
		</div>
	</div>

	<!-- Editable fields -->
	<form method="POST" action="?/update" use:enhance class="bg-gray-900 rounded-xl border border-gray-800 p-5 mb-4">
		<h3 class="text-sm font-medium text-gray-300 mb-4">Edit User</h3>
		<div class="space-y-4">
			<label class="block">
				<span class="text-gray-400 text-sm">Email</span>
				<input name="email" value={data.user.email || ''} class="mt-1 w-full bg-gray-800 border border-gray-700 rounded-lg p-2.5 text-white text-sm focus:outline-none focus:border-blue-500" />
			</label>
			<label class="block">
				<span class="text-gray-400 text-sm">Display Name</span>
				<input name="display_name" value={data.user.display_name || ''} class="mt-1 w-full bg-gray-800 border border-gray-700 rounded-lg p-2.5 text-white text-sm focus:outline-none focus:border-blue-500" />
			</label>
			<label class="block">
				<span class="text-gray-400 text-sm">Status</span>
				<select name="status" class="mt-1 w-full bg-gray-800 border border-gray-700 rounded-lg p-2.5 text-white text-sm focus:outline-none focus:border-blue-500">
					<option value="Active" selected={data.user.status === 'Active'}>Active</option>
					<option value="Suspended" selected={data.user.status === 'Suspended'}>Suspended</option>
				</select>
			</label>
		</div>
		<button type="submit" class="mt-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium py-2 px-4 rounded-lg transition-colors">
			Save Changes
		</button>
	</form>

	<!-- Claims editor -->
	<form method="POST" action="?/updateClaims" use:enhance class="bg-gray-900 rounded-xl border border-gray-800 p-5 mb-4">
		<h3 class="text-sm font-medium text-gray-300 mb-4">Claims</h3>
		{#if form?.claimsError}
			<div class="bg-red-900/30 border border-red-700 text-red-300 rounded-lg p-3 mb-3 text-sm">{form.claimsError}</div>
		{/if}
		{#if form?.claimsSuccess}
			<div class="bg-green-900/30 border border-green-700 text-green-300 rounded-lg p-3 mb-3 text-sm">Claims updated.</div>
		{/if}
		<textarea
			name="claims"
			rows="8"
			bind:value={claimsText}
			class="w-full bg-gray-800 border border-gray-700 rounded-lg p-3 text-white text-sm font-mono focus:outline-none focus:border-blue-500"
		></textarea>
		<button type="submit" class="mt-3 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium py-2 px-4 rounded-lg transition-colors">
			Save Claims
		</button>
	</form>

	<!-- Danger zone -->
	<div class="bg-gray-900 rounded-xl border border-red-900/50 p-5">
		<h3 class="text-sm font-medium text-red-400 mb-3">Danger Zone</h3>
		{#if !showDeleteConfirm}
			<button onclick={() => showDeleteConfirm = true} class="bg-red-600/20 hover:bg-red-600/30 border border-red-600/50 text-red-400 text-sm font-medium py-2 px-4 rounded-lg transition-colors">
				Delete User
			</button>
		{:else}
			<p class="text-gray-400 text-sm mb-3">Are you sure? This will soft-delete the user and revoke all sessions.</p>
			<div class="flex gap-2">
				<form method="POST" action="?/delete" use:enhance>
					<button type="submit" class="bg-red-600 hover:bg-red-700 text-white text-sm font-medium py-2 px-4 rounded-lg transition-colors">
						Confirm Delete
					</button>
				</form>
				<button onclick={() => showDeleteConfirm = false} class="bg-gray-800 hover:bg-gray-700 text-gray-300 text-sm font-medium py-2 px-4 rounded-lg transition-colors">
					Cancel
				</button>
			</div>
		{/if}
	</div>
</div>
