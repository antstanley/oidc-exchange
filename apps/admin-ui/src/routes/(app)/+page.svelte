<script lang="ts">
    let { data } = $props();

    const cards = $derived([
        { label: 'Total Users', value: data.stats.users.total, color: 'blue' },
        { label: 'Active', value: data.stats.users.active, color: 'green' },
        { label: 'Suspended', value: data.stats.users.suspended, color: 'yellow' },
        { label: 'Active Sessions', value: data.stats.sessions.active, color: 'purple' },
    ]);

    const colorClasses: Record<string, string> = {
        blue: 'bg-blue-500/10 border-blue-500/30 text-blue-400',
        green: 'bg-green-500/10 border-green-500/30 text-green-400',
        yellow: 'bg-yellow-500/10 border-yellow-500/30 text-yellow-400',
        purple: 'bg-purple-500/10 border-purple-500/30 text-purple-400',
    };
</script>

<div>
  <h2 class="text-xl font-semibold text-white mb-6">Dashboard</h2>

  <!-- Stat cards -->
  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-8">
    {#each cards as card}
      <div class="rounded-xl border p-5 {colorClasses[card.color]}">
        <p class="text-3xl font-bold">{card.value.toLocaleString()}</p>
        <p class="text-sm mt-1 opacity-70">{card.label}</p>
      </div>
    {/each}
  </div>

  <!-- Recent users -->
  <div class="bg-gray-900 rounded-xl border border-gray-800 overflow-hidden">
    <div class="px-5 py-4 border-b border-gray-800">
      <h3 class="text-sm font-medium text-gray-300">Recent Users</h3>
    </div>
    <table class="w-full">
      <thead>
        <tr class="border-b border-gray-800 text-left text-xs text-gray-500 uppercase">
          <th class="px-5 py-3">Email</th>
          <th class="px-5 py-3">Provider</th>
          <th class="px-5 py-3">Status</th>
          <th class="px-5 py-3">Created</th>
        </tr>
      </thead>
      <tbody>
        {#each data.recentUsers as user}
          <tr class="border-b border-gray-800/50 hover:bg-gray-800/30 transition-colors">
            <td class="px-5 py-3">
              <a href="/users/{user.id}" class="text-blue-400 hover:text-blue-300 text-sm">
                {user.email || user.external_id}
              </a>
            </td>
            <td class="px-5 py-3 text-sm text-gray-400">{user.provider}</td>
            <td class="px-5 py-3">
              {#if user.status === 'Active'}
                <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-green-500/10 text-green-400">Active</span>
              {:else if user.status === 'Suspended'}
                <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400">Suspended</span>
              {:else}
                <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-red-500/10 text-red-400">Deleted</span>
              {/if}
            </td>
            <td class="px-5 py-3 text-sm text-gray-500">{new Date(user.created_at).toLocaleDateString()}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
