<script lang="ts">
	let { data } = $props();
</script>

<div>
	<h2 class="text-xl font-semibold text-white mb-6">Users</h2>

	<div class="bg-gray-900 rounded-xl border border-gray-800 overflow-hidden">
		<table class="w-full">
			<thead>
				<tr class="border-b border-gray-800 text-left text-xs text-gray-500 uppercase">
					<th class="px-5 py-3">ID</th>
					<th class="px-5 py-3">Email</th>
					<th class="px-5 py-3">Provider</th>
					<th class="px-5 py-3">Status</th>
					<th class="px-5 py-3">Created</th>
				</tr>
			</thead>
			<tbody>
				{#each data.users as user}
					<tr class="border-b border-gray-800/50 hover:bg-gray-800/30 transition-colors cursor-pointer">
						<td class="px-5 py-3 text-sm text-gray-400 font-mono">{user.id.slice(0, 16)}...</td>
						<td class="px-5 py-3">
							<a href="/users/{user.id}" class="text-blue-400 hover:text-blue-300 text-sm">
								{user.email || user.external_id}
							</a>
						</td>
						<td class="px-5 py-3 text-sm text-gray-400">{user.provider}</td>
						<td class="px-5 py-3">
							{#if user.status === 'Active'}
								<span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-green-500/10 text-green-400">Active</span>
							{:else if user.status === 'Suspended'}
								<span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400">Suspended</span>
							{:else}
								<span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-red-500/10 text-red-400">Deleted</span>
							{/if}
						</td>
						<td class="px-5 py-3 text-sm text-gray-500">{new Date(user.created_at).toLocaleDateString()}</td>
					</tr>
				{/each}
				{#if data.users.length === 0}
					<tr>
						<td colspan="5" class="px-5 py-8 text-center text-gray-500">No users found</td>
					</tr>
				{/if}
			</tbody>
		</table>
	</div>

	<!-- Pagination -->
	<div class="flex justify-between items-center mt-4">
		<a
			href="/users?page={data.page - 1}"
			class="px-4 py-2 rounded-lg text-sm {data.page <= 1 ? 'text-gray-600 pointer-events-none' : 'text-gray-300 hover:bg-gray-800'}"
		>
			Previous
		</a>
		<span class="text-gray-500 text-sm">Page {data.page}</span>
		<a
			href="/users?page={data.page + 1}"
			class="px-4 py-2 rounded-lg text-sm {data.users.length < data.limit ? 'text-gray-600 pointer-events-none' : 'text-gray-300 hover:bg-gray-800'}"
		>
			Next
		</a>
	</div>
</div>
